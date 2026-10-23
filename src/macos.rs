use cargo_metadata::{Metadata, MetadataCommand};
use std::collections::HashMap;
use std::fs::{File, copy, create_dir_all, remove_file};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::project::Project;
use crate::apple::{AppleSigning, AppleSigningTarget, AscApiKey};
use crate::apple::{PushBuild, XcodeSigning};
use crate::manifest::Manifest;
use crate::PistonError;

mod development;
pub use development::MacDevelopment;
pub mod cli;

pub struct MacOSBuilder {
    developer: PathBuf,
    manifest: PathBuf,
    release: bool,
    external: bool,
    target: String,
    cwd: PathBuf,
    output_path: PathBuf,
    icon_path: String,
    assets: String,
    cargo_path: String,
    app_name: String,
    bundle_id: String,
    app_version: String,
    asc_api_key: Option<AscApiKey>,
    keystore_path: Option<String>,
    external_cert: Option<String>,
    team_id: Option<String>,
    notifications: Option<PushBuild>,
}

impl MacOSBuilder {
    fn new(
        release: bool, external: bool, target: String, cwd: PathBuf,
        env_vars: HashMap<String, String>, developer: PathBuf,
    ) -> Result<Self, PistonError> {
        let cargo_path = env_vars.get("cargo_path").cloned().unwrap_or("cargo".to_string());
        let keystore_path = env_vars.get("keystore_path").cloned();
        let external_cert = env_vars.get("external_cert").cloned();
        let team_id = env_vars.get("team_id").cloned();
        let metadata: Metadata = MetadataCommand::new()
            .current_dir(&cwd)
            .exec()
            .map_err(|e| PistonError::CargoParseError(e.to_string()))?;

        let icon_path = Project::get_icon_path(&metadata, &cwd);
        let manifest = metadata.root_package().unwrap().manifest_path.clone().into_std_path_buf();
        let assets = Project::get_assets_path(&metadata);
        let app_name = Project::get_app_name(&metadata)?;
        let version = &metadata.root_package().unwrap().version;
        let app_version = format!("{}.{}.{}", version.major, version.minor, version.patch);
        let package = metadata.root_package().unwrap();
        let bundle_id = package
            .metadata
            .pointer("/piston/macos/bundle_id")
            .or_else(|| package.metadata.pointer("/piston/ios/bundle_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| Project::get_bundle_id(&metadata, &app_name));
        let capitalized = Project::capitalize_first(&app_name);
        let profile = if external {
            "release-external"
        } else if release {
            "release"
        } else {
            "debug"
        };
        let output_path = cwd.join(format!("target/{}/macos/{}.app", profile, capitalized));

        let asc_api_key = AscApiKey::from_hm(&env_vars).ok();
        Ok(MacOSBuilder {
            developer,
            manifest,
            release,
            external,
            target,
            cwd,
            output_path,
            icon_path,
            assets,
            cargo_path,
            app_name,
            bundle_id,
            app_version,
            asc_api_key,
            keystore_path,
            external_cert,
            team_id,
            notifications: None,
        })
    }

    fn pre_build(&self) -> Result<(), PistonError> {
        //TODO allow user to specify a security cert for offline signing?
        let cwd = &self.cwd;
        let capitalized = Project::capitalize_first(&self.app_name);
        let true_bundle_path = self.output_path.strip_prefix(&self.cwd).unwrap();
        let contents_path: PathBuf = true_bundle_path.join("Contents");
        let res_path: PathBuf = contents_path.join("Resources");
        let assets_tgt = cwd.join(&res_path).join("assets");
        let macos_path = contents_path.join("MacOS");
        let path = res_path.as_path();
        Project::empty_directory(path, &["assets"])?;
        create_dir_all(path).map_err(|e| PistonError::CreateDirAllError {
            path: self.output_path.clone(),
            source: e,
        })?;
        create_dir_all(macos_path).map_err(|e| PistonError::CreateDirAllError {
            path: self.output_path.clone(),
            source: e,
        })?;
        let assets_src = Path::new(&self.assets);
        Project::sync_assets(assets_src, &assets_tgt)?;
        let icon_path: PathBuf = res_path.join("macos_icon.icns");
        let plist_path: PathBuf = contents_path.join("Info.plist");
        if plist_path.exists() {
            remove_file(&plist_path).map_err(|e| PistonError::RemoveFileError {
                path: plist_path.to_path_buf(),
                source: e,
            })?;
        }
        let mut plist_file = File::create(&plist_path).map_err(|e| {
            PistonError::CreateFileError { path: plist_path.to_path_buf(), source: e }
        })?;
        let plist_content = format!(
            r#"
            <?xml version="1.0" encoding="UTF-8"?>
            <!DOCTYPE plist PUBLIC "-//Apple Computer//DTD PLIST 1.0//EN"
            "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
            <plist version="1.0">
            <dict>
                <key>CFBundleName</key>
                <string>{}</string>
                <key>CFBundleExecutable</key>
                <string>{}</string>
                <key>CFBundleIdentifier</key>
                <string>{}</string>
                <key>CFBundleIconFile</key>
                <string>macos_icon</string>
                <key>CFBundleVersion</key>
                <string>{}</string>
                <key>CFBundleShortVersionString</key>
                <string>{}</string>
                <key>LSMinimumSystemVersion</key>
                <string>11.0</string>
                <key>CFBundlePackageType</key>
                <string>APPL</string>
            </dict>
            </plist>
            "#,
            &capitalized, &self.app_name, &self.bundle_id, &self.app_version, &self.app_version,
        );
        plist_file
            .write_all(plist_content.as_bytes())
            .map_err(|e| PistonError::WriteFileError(e.to_string()))?;
        let img_path = Path::new(&self.icon_path);
        Command::new("sips")
            .args([
                "-s",
                "format",
                "icns",
                &self.icon_path,
                "--out",
                &icon_path.display().to_string(),
            ])
            .output()
            .map_err(|e| PistonError::MacOSIconError {
                input_path: img_path.to_path_buf(),
                output_path: icon_path,
                source: e,
            })?;
        Ok(())
    }

    fn build(&self) -> Result<(), PistonError> {
        let cargo_args = format!(
            "build --target {} {}",
            self.target,
            if self.release { "--release" } else { "" }
        );
        let cargo_cmd = format!("{} {}", self.cargo_path, cargo_args);
        let mut command = Command::new("bash");
        command.env_remove("RAMP_NOTIFICATION_APPLICATION");
        if let Some(notifications) = &self.notifications { notifications.compile(&mut command); }
        let builder = command
            .env("DEVELOPER_DIR", &self.developer)
            .arg("-c")
            .arg(&cargo_cmd)
            .current_dir(&self.cwd)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| PistonError::BuildError(format!("Cargo build failed: {}", e)))?;
        if !builder.status.success() {
            return Err(PistonError::BuildError(format!(
                "Cargo build failed: {}",
                String::from_utf8_lossy(&builder.stderr)
            )));
        }
        if self.release {
            let secondary = if self.target.contains("aarch64") {
                "x86_64-apple-darwin"
            } else {
                "aarch64-apple-darwin"
            };
            let cargo_args_second = format!(
                "build --target {} {}",
                secondary,
                if self.release { "--release" } else { "" }
            );
            let cargo_cmd_second = format!("{} {}", self.cargo_path, cargo_args_second);
            let mut command = Command::new("bash");
            command.env_remove("RAMP_NOTIFICATION_APPLICATION");
            if let Some(notifications) = &self.notifications {
                notifications.compile(&mut command);
            }
            let builder_second = command
                .env("DEVELOPER_DIR", &self.developer)
                .arg("-c")
                .arg(&cargo_cmd_second)
                .current_dir(&self.cwd)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .output()
                .map_err(|e| {
                    PistonError::BuildError(format!("Second Cargo build failed: {}", e))
                })?;
            if !builder_second.status.success() {
                return Err(PistonError::BuildError(format!(
                    "Second Cargo build failed: {}",
                    String::from_utf8_lossy(&builder.stderr)
                )));
            }
        }

        Ok(())
    }

    fn notarize(&self) -> Result<(), PistonError> {
        let working_path = &self.output_path;
        let parent = working_path.parent().map(PathBuf::from).ok_or_else(|| {
            PistonError::Generic("Failed to determine zip path via parent method".to_string())
        })?;
        let zip_str = format!("{}.zip", &self.app_name);
        let zip_path = parent.join(zip_str);

        let zip = Command::new("ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(working_path)
            .arg(&zip_path)
            .output()
            .map_err(|e| PistonError::Generic(format!("Ditto failed to execute: {}", e)))?;

        if !zip.status.success() {
            return Err(PistonError::Generic(format!(
                "ditto failed: {}",
                String::from_utf8_lossy(&zip.stderr)
            )));
        }

        let submit = Command::new("xcrun")
            .env("DEVELOPER_DIR", &self.developer)
            .args(["notarytool", "submit"])
            .arg(&zip_path)
            .args(["--keychain-profile", "DeveloperID-Notary", "--wait"])
            .output()
            .map_err(|e| {
                PistonError::Generic(format!("notarytool submit failed to execute: {}", e))
            })?;

        if !submit.status.success()
            || String::from_utf8_lossy(&submit.stdout).contains("status: Invalid")
        {
            return Err(PistonError::Generic(format!(
                "notarytool submit failed: {} : {}",
                String::from_utf8_lossy(&submit.stderr).trim(),
                String::from_utf8_lossy(&submit.stdout).trim(),
            )));
        }

        let staple = Command::new("xcrun")
            .env("DEVELOPER_DIR", &self.developer)
            .args(["stapler", "staple"])
            .arg(working_path)
            .output()
            .map_err(|e| PistonError::Generic(format!("stapler failed to execute: {}", e)))?;

        if !staple.status.success() {
            return Err(PistonError::Generic(format!(
                "stapler failed: {}",
                String::from_utf8_lossy(&staple.stderr).trim()
            )));
        }

        Ok(())
    }
}

impl MacOSBuilder {
    fn post_build(&self) -> Result<Vec<String>, PistonError> {
        let mut messages = Vec::new();
        messages.push(format!("post build for macos"));
        let binary_path = self
            .cwd
            .join("target")
            .join(&self.target)
            .join(if self.release { "release" } else { "debug" })
            .join(&self.app_name);
        let binary_target_path =
            self.output_path.join("Contents").join("MacOS").join(&self.app_name);
        if !self.release {
            messages.push(format!("copying binary to app bundle"));
            copy(&binary_path, &binary_target_path).map_err(|e| PistonError::CopyFileError {
                input_path: binary_path.to_path_buf(),
                output_path: binary_target_path.to_path_buf(),
                source: e,
            })?;
            messages
                .push(format!("MacOS app bundle available at: {}", &binary_target_path.display()));
        } else {
            messages.push(format!("creating universal binary in the app bundle"));
            let secondary = if self.target.contains("aarch64") {
                "x86_64-apple-darwin"
            } else {
                "aarch64-apple-darwin"
            };
            let secondary_path =
                self.cwd.join("target").join(secondary).join("release").join(&self.app_name);
            let lipo = Command::new("xcrun")
                .env("DEVELOPER_DIR", &self.developer)
                .arg("lipo")
                .arg("-create")
                .arg(&binary_path)
                .arg(&secondary_path)
                .arg("-output")
                .arg(&binary_target_path)
                .output()
                .map_err(|e| PistonError::LipoError {
                    first_binary: binary_path.clone(),
                    second_binary: secondary_path.clone(),
                    source: e.to_string(),
                })?;
            if !lipo.status.success() {
                return Err(PistonError::LipoError {
                    first_binary: binary_path,
                    second_binary: secondary_path,
                    source: String::from_utf8_lossy(&lipo.stderr).to_string(),
                });
            }
            messages.push(format!(
                "Universal MacOS app bundle available at: {}",
                &binary_target_path.display()
            ));
        }

        if let Some(notifications) = &self.notifications {
            let work = self.cwd.join("target/piston/macos").join(&self.app_name);
            create_dir_all(&work)
                .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
            let settings = Manifest::new(&self.manifest)?.settings(&["macos"])?;
            let source =
                settings.get("entitlements").map(|path| self.manifest.parent().unwrap().join(path));
            let entitlements = notifications
                .signing
                .entitlements(source.as_deref(), &work.join("notifications.entitlements"))?;
            if self.release && !self.external {
                let mut values = plist::Value::from_file(&entitlements)
                    .map_err(|error| PistonError::CodesignError(error.to_string()))?;
                let values_dict = values.as_dictionary_mut().unwrap();
                values_dict
                    .insert("com.apple.security.app-sandbox".into(), plist::Value::Boolean(true));
                values_dict.insert(
                    "com.apple.security.network.client".into(),
                    plist::Value::Boolean(true),
                );
                values
                    .to_file_xml(&entitlements)
                    .map_err(|error| PistonError::CodesignError(error.to_string()))?;
            }
            let signing = XcodeSigning::new(&notifications.signing.team, &self.developer);
            let signed = signing.sign_macos(
                &self.output_path,
                &work,
                &entitlements,
                self.release,
                self.external,
            )?;
            notifications.signing.verify(&signed)?;
            std::fs::remove_dir_all(&self.output_path)
                .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
            let copy = Command::new("ditto")
                .arg(&signed)
                .arg(&self.output_path)
                .output()
                .map_err(|error| PistonError::CodesignError(error.to_string()))?;
            if !copy.status.success() {
                return Err(PistonError::CodesignError(
                    "Cannot copy the signed macOS app bundle.".into(),
                ));
            }
            if self.external { self.notarize()?; }
            messages.push(format!("Signed notification app: {}", self.output_path.display()));
            return Ok(messages);
        }
        if self.keystore_path.is_none() || self.asc_api_key.is_none() {
            messages.push(format!(
                "Either the Keystore path or ASC API key missing from .env, skipping automated signing"
            ));
        } else if self.external {
            if self.external_cert.is_none() {
                return Err(PistonError::Generic(
                    "external-release certificate is not properly configured, see documentation"
                        .to_string(),
                ));
            }
            AppleSigning::sign_app_bundle(
                &self.output_path,
                self.external_cert.as_ref().unwrap(),
                AppleSigningTarget::MacExternal,
            )?;
            self.notarize()?;
        } else {
            messages.push(format!("keystore path & ASC API key properly configured"));
            let asc = AppleSigning::new(
                self.asc_api_key.clone().unwrap(),
                self.keystore_path.clone().unwrap(),
                &self.manifest,
            )?;
            let security_cert = asc.create_or_find_security_cert(self.team_id.clone())?;
            let security_profile = format!("{} ({})", security_cert.1, security_cert.0);
            messages.push(format!("your security profile is: {:?}", security_profile));
            AppleSigning::sign_app_bundle(
                &self.output_path,
                &security_profile,
                AppleSigningTarget::MacAppStore,
            )?;
        }
        Ok(messages)
    }
}
