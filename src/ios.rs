use cargo_metadata::{Metadata, MetadataCommand};
use std::collections::HashMap;
use std::fs::{self, File, copy, create_dir_all, remove_file};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use crate::project::Project;
use crate::apple::{AppleSigning, AppleSigningTarget, AscApiKey};
use crate::devices::IOSDevice;
use crate::host::Host;
use crate::PistonError;
pub use crate::cli::ApplicationArgs;

mod application;
pub use application::IOSApplication;
pub mod cli;
pub use cli::Cli;
pub mod deployment;
pub use deployment::DeploymentArgs;
mod development;
mod simulator;
pub mod distribution;
pub use distribution::IOSStore;

pub struct IOSBuilder {
    developer: PathBuf,
    manifest: PathBuf,
    release: bool,
    target: String,
    cwd: PathBuf,
    output_path: PathBuf,
    ipa_path: Option<PathBuf>,
    icon_path: String,
    cargo_path: String,
    app_name: String,
    app_version: String,
    bundle_id: String,
    min_os_version: f32,
    asc_api_key: Option<AscApiKey>,
    device_target: Option<IOSDevice>,
    idp_path: Option<String>,
    keystore_path: Option<String>,
    team_id: Option<String>,
}

impl IOSBuilder {
    fn new(
        release: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>,
        device_target: Option<IOSDevice>, developer: PathBuf,
    ) -> Result<Self, PistonError> {
        let cargo_path = env_vars.get("cargo_path").cloned().unwrap_or("cargo".to_string());
        let idp_path = env_vars.get("idp_path").cloned();
        let keystore_path = env_vars.get("keystore_path").cloned();
        let team_id = env_vars.get("team_id").cloned();
        let metadata: Metadata = MetadataCommand::new()
            .current_dir(&cwd)
            .exec()
            .map_err(|e| PistonError::CargoParseError(e.to_string()))?;

        let icon_path = Project::get_icon_path(&metadata, &cwd);
        let manifest = metadata.root_package().unwrap().manifest_path.clone().into_std_path_buf();
        let app_name = Project::get_app_name(&metadata)?;
        let capitalized = Project::capitalize_first(&app_name);
        let profile = if release { "release" } else { "debug" };
        let output_path = cwd.join(format!("target/{profile}/ios/{capitalized}.app"));
        let version = &metadata.root_package().unwrap().version;
        let app_version = format!("{}.{}.{}", version.major, version.minor, version.patch);
        let bundle_id = Project::get_bundle_id(&metadata, &app_name);
        let min_os_version = Project::get_min_os(&metadata);

        let asc_api_key = AscApiKey::from_hm(&env_vars).ok();
        Ok(Self {
            developer,
            manifest,
            release,
            target,
            cwd,
            output_path,
            ipa_path: None,
            icon_path,
            cargo_path,
            app_name,
            app_version,
            bundle_id,
            min_os_version,
            asc_api_key,
            device_target,
            idp_path,
            keystore_path,
            team_id,
        })
    }

    fn pre_build(&self) -> Result<(), PistonError> {
        let capitalized = Project::capitalize_first(&self.app_name);
        let bundle_path = &self.output_path;

        if bundle_path.as_path().exists() { let _ = fs::remove_dir_all(bundle_path); }

        create_dir_all(bundle_path).map_err(|e| PistonError::CreateDirAllError {
            path: bundle_path.to_path_buf(),
            source: e,
        })?;

        let parent = bundle_path.parent();
        let assets_path = parent.unwrap().join("Assets.xcassets");
        let appicon_path = assets_path.join("AppIcon.appiconset");

        if appicon_path.exists() { let _ = fs::remove_dir_all(&appicon_path); }

        create_dir_all(&appicon_path).map_err(|e| PistonError::CreateDirAllError {
            path: self.output_path.clone(),
            source: e,
        })?;

        let icon_path120: PathBuf = appicon_path.join("ios_icon120.png");
        Project::resize_png(&self.icon_path, &icon_path120.display().to_string(), 120, 120)?;
        let icon_path180: PathBuf = appicon_path.join("ios_icon180.png");
        Project::resize_png(&self.icon_path, &icon_path180.display().to_string(), 180, 180)?;

        let contents_path: PathBuf = appicon_path.join("Contents.json");
        if contents_path.exists() {
            remove_file(&contents_path).map_err(|e| PistonError::RemoveFileError {
                path: contents_path.clone(),
                source: e,
            })?;
        }
        let mut contents_file = File::create(&contents_path)
            .map_err(|e| PistonError::CreateFileError { path: contents_path.clone(), source: e })?;

        let json_contents = r#"{
    "images": [
        { "size": "120x120", "idiom": "iphone", "filename": "ios_icon120.png", "scale": "1x" },
        { "size": "180x180", "idiom": "iphone", "filename": "ios_icon180.png", "scale": "1x" }
    ],
    "info": { "version": 1, "author": "xcode" }
}"#;

        contents_file
            .write_all(json_contents.trim().as_bytes())
            .map_err(|e| PistonError::WriteFileError(e.to_string()))?;

        let assets_output = bundle_path.join("Assets.car");
        let status = Command::new("xcrun")
            .env("DEVELOPER_DIR", &self.developer)
            .args([
                "actool",
                "--output-format",
                "human-readable-text",
                "--platform",
                "iphoneos",
                "--minimum-deployment-target",
                "15.0",
                "--app-icon",
                "AppIcon",
                "--compile",
                &assets_path.display().to_string(),
                &assets_output.display().to_string(),
            ])
            .output()
            .map_err(|e| PistonError::CommandSpawn { program: "xcrun".into(), source: e })?;

        if !status.status.success() {
            return Err(PistonError::CommandFailed { program: "xcrun".into(), output: status });
        }

        let plist_path: PathBuf = bundle_path.join("Info.plist");
        if plist_path.exists() {
            remove_file(&plist_path).map_err(|e| PistonError::RemoveFileError {
                path: plist_path.clone(),
                source: e,
            })?;
        }
        let mut plist_file = File::create(&plist_path)
            .map_err(|e| PistonError::CreateFileError { path: plist_path.clone(), source: e })?;

        let plist_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>{}</string>
    <key>CFBundleIdentifier</key>
    <string>{}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleDisplayName</key>
    <string>{}</string>
    <key>CFBundleExecutable</key>
    <string>{}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>{}</string>
    <key>CFBundleVersion</key>
    <string>{}</string>
    <key>LSRequiresIPhoneOS</key>
    <true/>
    <key>MinimumOSVersion</key>
    <string>{}</string>
    <key>UIDeviceFamily</key>
    <array>
        <integer>1</integer>
        <integer>2</integer>
    </array>
    <key>CFBundleSupportedPlatforms</key>
    <array>
        <string>iPhoneOS</string>
    </array>
    <key>DTPlatformName</key>
    <string>iphoneos</string>
    <key>DTPlatformVersion</key>
    <string>18.5</string>
    <key>DTSDKName</key>
    <string>iphoneos18.5</string>
    <key>DTCompiler</key>
    <string>com.apple.compilers.llvm.clang.1_0</string>
    <key>DTXcode</key>
    <string>1620</string>
    <key>CFBundleIconName</key>
    <string>AppIcon</string>
    <key>CFBundleIcons</key>
    <dict>
        <key>CFBundlePrimaryIcon</key>
        <dict>
            <key>CFBundleIconName</key>
            <string>AppIcon</string>
        </dict>
    </dict>
</dict>
</plist>
"#,
            capitalized,
            self.bundle_id,
            capitalized,
            capitalized,
            self.app_version,
            self.app_version,
            self.min_os_version
        );

        plist_file
            .write_all(plist_content.trim().as_bytes())
            .map_err(|e| PistonError::WriteFileError(e.to_string()))?;

        let output = Command::new("plutil")
            .args(["-convert", "binary1", &plist_path.display().to_string()])
            .output()
            .map_err(|e| PistonError::PlutilConvertError(e.to_string()))?;
        if !output.status.success() {
            return Err(PistonError::PlutilConvertError(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }
        Ok(())
    }

    fn build(&self) -> Result<(), PistonError> {
        let cargo_args = format!(
            "build --target {} {}",
            self.target,
            if self.release { "--release" } else { "" }
        );
        let cargo_cmd = format!("{} {}", self.cargo_path, cargo_args);
        let builder = Command::new("bash")
            .env("DEVELOPER_DIR", &self.developer)
            .arg("-c")
            .arg(&cargo_cmd)
            .current_dir(&self.cwd)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| PistonError::CommandSpawn { program: "bash".into(), source: e })?;

        if !builder.status.success() {
            return Err(PistonError::CommandFailed { program: "bash".into(), output: builder });
        }

        Ok(())
    }
}

impl IOSBuilder {
    fn post_build(&mut self) -> Result<(), PistonError> {
        let provision_cache = self.cwd.join("target").join("ios-cache").join("profiles");
        if !provision_cache.exists() {
            create_dir_all(&provision_cache).map_err(|e| PistonError::CreateDirAllError {
                path: provision_cache.to_path_buf(),
                source: e,
            })?;
        }
        let binary_path = self
            .cwd
            .join("target")
            .join(&self.target)
            .join(if self.release { "release" } else { "debug" })
            .join(&self.app_name);
        let capitalized = Project::capitalize_first(&self.app_name);
        let bundle_path = self.output_path.join(&capitalized);
        copy(&binary_path, &bundle_path).map_err(|e| PistonError::CopyFileError {
            input_path: binary_path.clone(),
            output_path: bundle_path.clone(),
            source: e,
        })?;
        let exe_path = self.output_path.join(&self.app_name);
        Host::make_executable(&exe_path)?;
        let output = Command::new("xattr")
            .args(["-cr", &bundle_path.display().to_string()])
            .output()
            .map_err(|e| PistonError::Generic(e.to_string()))?;
        if !output.status.success() {
            return Err(PistonError::Generic(String::from_utf8_lossy(&output.stderr).to_string()));
        }

        if self.keystore_path.is_some() && self.asc_api_key.is_some() {
            let asc_client = AppleSigning::new(
                self.asc_api_key.clone().unwrap(),
                self.keystore_path.clone().unwrap(),
                &self.manifest,
            )?;
            let security_cert = asc_client.create_or_find_security_cert(self.team_id.clone())?;
            let security_profile = format!("{} ({})", security_cert.1, security_cert.0);
            let output_path = &self.output_path;
            if let Some(device) = &self.device_target {
                let target_id = &device.id;
                let idp_path = self.idp_path.as_ref().unwrap();
                let provisioned = AppleSigning::is_device_provisioned(
                    output_path,
                    target_id,
                    idp_path,
                    &provision_cache,
                )?;
                if !provisioned {
                    asc_client.provision_ios_device(
                        target_id,
                        &self.bundle_id,
                        &self.app_name,
                        &security_profile,
                        output_path,
                        idp_path,
                        &provision_cache,
                    )?;
                }
            }
            AppleSigning::sign_app_bundle(output_path, &security_profile, AppleSigningTarget::Ios)?;
            let parent = output_path.parent().unwrap();
            let ipa_path = parent.join(format!("{}.ipa", &capitalized));
            self.ipa_path = Some(ipa_path.clone());
            if ipa_path.as_path().exists() {
                remove_file(&ipa_path).map_err(|e| PistonError::RemoveFileError {
                    path: ipa_path.to_path_buf(),
                    source: e,
                })?;
            }
            let payload_path = parent.join("Payload");
            if payload_path.exists() { let _ = fs::remove_dir_all(&payload_path); }
            create_dir_all(&payload_path).map_err(|e| PistonError::CreateDirAllError {
                path: payload_path.to_path_buf(),
                source: e,
            })?;
            let dest = payload_path.join(format!("{}.app", &capitalized));
            let status =
                Command::new("cp").arg("-r").arg(output_path).arg(&dest).output().map_err(|e| {
                    PistonError::CopyFileError {
                        input_path: output_path.to_path_buf(),
                        output_path: dest.clone(),
                        source: e,
                    }
                })?;

            if !status.status.success() {
                return Err(PistonError::CommandFailed { program: "cp".into(), output: status });
            }

            let status = Command::new("zip")
                .arg("-r")
                .arg(&ipa_path)
                .arg("Payload")
                .current_dir(parent)
                .output()
                .map_err(|e| PistonError::CommandSpawn { program: "zip".into(), source: e })?;

            if !status.status.success() {
                return Err(PistonError::CommandFailed { program: "zip".into(), output: status });
            }
            let _ = std::fs::remove_dir_all(payload_path);

        }
        Ok(())
    }
}

pub struct IOSRunner;

impl IOSRunner {
    //TODO this is currently broken
    fn deploy_usb(device_id: &str, output_path: &str, bundle_id: &str, developer: &std::path::Path) -> Result<(), PistonError> {
        let _ = Command::new("xcrun")
            .env("DEVELOPER_DIR", &developer)
            .args([
                "devicectl",
                "device",
                "uninstall",
                "app",
                "--device",
                device_id,
                "--bundle-id",
                bundle_id,
            ])
            .output();
        let output = Command::new("xcrun")
            .env("DEVELOPER_DIR", &developer)
            .args(["devicectl", "device", "install", "app", "--device", device_id, output_path])
            .output()
            .map_err(|e| PistonError::XcrunInstallError(e.to_string()))?;
        if !output.status.success() {
            return Err(PistonError::XcrunInstallError(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        let output = Command::new("xcrun")
            .env("DEVELOPER_DIR", &developer)
            .args(["devicectl", "device", "process", "launch", "--device", device_id, bundle_id])
            .output()
            .map_err(|e| PistonError::XcrunLaunchError(e.to_string()))?;
        if !output.status.success() {
            return Err(PistonError::XcrunLaunchError(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        Ok(())
    }
}
