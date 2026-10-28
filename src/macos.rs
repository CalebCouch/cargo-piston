use cargo_metadata::{Metadata, MetadataCommand};
use std::collections::HashMap;
use std::fs::{File, copy, create_dir_all, remove_file};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::Result;
use crate::cli::Error;
use crate::project::Project;
use crate::apple::{AppleSigning, AppleSigningTarget, AscApiKey};
use crate::apple::{PushBuild, XcodeSigning};
use crate::manifest::Manifest;

pub mod cli;
pub use cli::Cli;

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
    pub fn valid_setting(name: &str, value: &toml_edit::Item, root: &Path) -> bool {
        if name == "resources" {
            return value.as_array().is_some_and(|paths| paths.iter().all(|value| {
                value.as_str().is_some_and(|path| {
                    !path.is_empty() && root.join(path).exists()
                        && root.join(path).file_name().is_some()
                })
            }));
        }
        let Some(value) = value.as_str() else { return false; };
        match name {
            "team" => value.len() == 10 && value.bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()),
            "bundle_id" => value.contains('.') && !value.split('.').any(str::is_empty)
                && value.chars().all(|character| {
                    character.is_ascii_alphanumeric() || ".-".contains(character)
                }),
            "info_plist" | "entitlements" => !value.is_empty()
                && plist::Value::from_file(root.join(value))
                    .is_ok_and(|value| value.as_dictionary().is_some()),
            _ => unreachable!(),
        }
    }

    fn new(release: bool, external: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>, developer: PathBuf) -> Result<Self> {
        let cargo_path = env_vars.get("cargo_path").cloned().unwrap_or("cargo".to_string());
        let keystore_path = env_vars.get("keystore_path").cloned();
        let external_cert = env_vars.get("external_cert").cloned();
        let team_id = env_vars.get("team_id").cloned();
        let metadata: Metadata = MetadataCommand::new()
            .current_dir(&cwd)
            .exec()
            .map_err(|e| Error::CargoParse(e.to_string()))?;

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

    fn pre_build(&self) -> Result<()> {
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
        create_dir_all(path).map_err(|e| Error::CreateDirAll {
            path: self.output_path.clone(),
            source: e,
        })?;
        create_dir_all(macos_path).map_err(|e| Error::CreateDirAll {
            path: self.output_path.clone(),
            source: e,
        })?;
        let assets_src = Path::new(&self.assets);
        Project::sync_assets(assets_src, &assets_tgt)?;
        let icon_path: PathBuf = res_path.join("macos_icon.icns");
        let plist_path: PathBuf = contents_path.join("Info.plist");
        if plist_path.exists() {
            remove_file(&plist_path).map_err(|e| Error::RemoveFile {
                path: plist_path.to_path_buf(),
                source: e,
            })?;
        }
        let mut plist_file = File::create(&plist_path).map_err(|e| {
            Error::CreateFile { path: plist_path.to_path_buf(), source: e }
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
            capitalized, self.app_name, self.bundle_id, self.app_version, self.app_version,
        );
        plist_file
            .write_all(plist_content.as_bytes())
            .map_err(|e| Error::WriteFile(e.to_string()))?;
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
            .map_err(|e| Error::MacOSIcon {
                input_path: img_path.to_path_buf(),
                output_path: icon_path,
                source: e,
            })?;
        Ok(())
    }

    fn build(&self) -> Result<()> {
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
            .map_err(|e| Error::CommandSpawn { program: "bash".into(), source: e })?;
        if !builder.status.success() {
            return Err(Error::CommandFailed { program: "bash".into(), output: builder }.into());
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
                .map_err(|e| Error::CommandSpawn { program: "bash".into(), source: e })?;
            if !builder_second.status.success() {
                return Err(Error::CommandFailed {
                    program: "bash".into(), output: builder_second,
                }.into());
            }
        }

        Ok(())
    }

    fn notarize(&self) -> Result<()> {
        let working_path = &self.output_path;
        let parent = working_path.parent().unwrap();
        let zip_str = format!("{}.zip", self.app_name);
        let zip_path = parent.join(zip_str);

        let zip = Command::new("ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(working_path)
            .arg(&zip_path)
            .output()
            .map_err(|e| Error::CommandSpawn { program: "ditto".into(), source: e })?;

        if !zip.status.success() {
            return Err(Error::CommandFailed { program: "ditto".into(), output: zip }.into());
        }

        let submit = Command::new("xcrun")
            .env("DEVELOPER_DIR", &self.developer)
            .args(["notarytool", "submit"])
            .arg(&zip_path)
            .args(["--keychain-profile", "DeveloperID-Notary", "--wait"])
            .output()
            .map_err(|e| Error::CommandSpawn { program: "xcrun".into(), source: e })?;

        if !submit.status.success()
            || String::from_utf8_lossy(&submit.stdout).contains("status: Invalid")
        {
            return Err(Error::CommandFailed { program: "xcrun".into(), output: submit }.into());
        }

        let staple = Command::new("xcrun")
            .env("DEVELOPER_DIR", &self.developer)
            .args(["stapler", "staple"])
            .arg(working_path)
            .output()
            .map_err(|e| Error::CommandSpawn { program: "xcrun".into(), source: e })?;

        if !staple.status.success() {
            return Err(Error::CommandFailed { program: "xcrun".into(), output: staple }.into());
        }

        Ok(())
    }
}

impl MacOSBuilder {
    fn post_build(&self) -> Result<()> {
        let binary_path = self
            .cwd
            .join("target")
            .join(&self.target)
            .join(if self.release { "release" } else { "debug" })
            .join(&self.app_name);
        let binary_target_path =
            self.output_path.join("Contents").join("MacOS").join(&self.app_name);
        if !self.release {
            copy(&binary_path, &binary_target_path).map_err(|e| Error::CopyFile {
                input_path: binary_path.to_path_buf(),
                output_path: binary_target_path.to_path_buf(),
                source: e,
            })?;
        } else {
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
                .map_err(|e| Error::Lipo {
                    first_binary: binary_path.clone(),
                    second_binary: secondary_path.clone(),
                    source: e.to_string(),
                })?;
            if !lipo.status.success() {
                return Err(Error::Lipo {
                    first_binary: binary_path,
                    second_binary: secondary_path,
                    source: String::from_utf8_lossy(&lipo.stderr).to_string(),
                }.into());
            }
        }

        if let Some(notifications) = &self.notifications {
            let work = self.cwd.join("target/piston/macos").join(&self.app_name);
            create_dir_all(&work)
                .map_err(|error| Error::WriteFile(error.to_string()))?;
            let settings = Manifest::new(&self.manifest)?.settings(&["macos"])?;
            let source =
                settings.get("entitlements").map(|path| self.manifest.parent().unwrap().join(path));
            let entitlements = notifications
                .signing
                .entitlements(source.as_deref(), &work.join("notifications.entitlements"))?;
            if self.release && !self.external {
                let mut values = plist::Value::from_file(&entitlements)
                    .map_err(|error| crate::apple::cli::Error::Codesign(error.to_string()))?;
                let values_dict = values.as_dictionary_mut().unwrap();
                values_dict
                    .insert("com.apple.security.app-sandbox".into(), plist::Value::Boolean(true));
                values_dict.insert(
                    "com.apple.security.network.client".into(),
                    plist::Value::Boolean(true),
                );
                values
                    .to_file_xml(&entitlements)
                    .map_err(|error| crate::apple::cli::Error::Codesign(error.to_string()))?;
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
                .map_err(|error| Error::WriteFile(error.to_string()))?;
            let copy = Command::new("ditto")
                .arg(&signed)
                .arg(&self.output_path)
                .output()
                .map_err(|error| crate::apple::cli::Error::Codesign(error.to_string()))?;
            if !copy.status.success() {
                return Err(Error::CommandFailed { program: "ditto".into(), output: copy }.into());
            }
            if self.external { self.notarize()?; }
            return Ok(());
        }
        if self.keystore_path.is_none() || self.asc_api_key.is_none() { return Ok(()); }
        if self.external {
            if self.external_cert.is_none() {
                return Err(Error::MissingSetting { key: "external_cert".into() }.into());
            }
            AppleSigning::sign_app_bundle(
                &self.output_path,
                self.external_cert.as_ref().unwrap(),
                AppleSigningTarget::MacExternal,
            )?;
            self.notarize()?;
        } else {
            let asc = AppleSigning::new(
                self.asc_api_key.clone().unwrap(),
                self.keystore_path.clone().unwrap(),
                &self.manifest,
            )?;
            let security_cert = asc.create_or_find_security_cert(self.team_id.clone())?;
            let security_profile = format!("{} ({})", security_cert.1, security_cert.0);
            AppleSigning::sign_app_bundle(
                &self.output_path,
                &security_profile,
                AppleSigningTarget::MacAppStore,
            )?;
        }
        Ok(())
    }
}
