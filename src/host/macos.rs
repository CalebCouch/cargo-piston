use plist::{Dictionary, Value};
use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::host::cli::Error as HostError;
use crate::cli::Error as SharedError;
use crate::devices::IOSDevice;
use crate::apple::{NotificationSetup, PushBuild, PushPlatform, PushSigning, XcodeSigning};
use crate::manifest::Manifest;
use crate::project::Project;
use crate::Result;

pub use super::Cli;

pub mod apple;

pub mod cli;

pub struct Host;

impl Host {
    pub fn linux_lima() -> Result<PathBuf> {
        if Command::new("limactl")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        { return Ok(PathBuf::from("limactl")); }
        Project::run(Command::new("brew").args(["install", "lima"]))?;
        let prefix = Project::output(Command::new("brew").args(["--prefix", "lima"]))?;
        Ok(PathBuf::from(String::from_utf8_lossy(&prefix).trim()).join("bin/limactl"))
    }

    pub fn android_sdk() -> Result<PathBuf> {
        if let Some(path) = env::var_os("ANDROID_HOME").or_else(|| env::var_os("ANDROID_SDK_ROOT"))
        { return Ok(PathBuf::from(path)); }
        let root = PathBuf::from(
            env::var_os("HOME").ok_or_else(|| SharedError::MissingEnvironment { name: "HOME" })?,
        )
        .join("Library/Android/sdk");
        if root.join("cmdline-tools").is_dir() { return Ok(root); }
        Project::run(Command::new("brew").args(["install", "--cask", "android-commandlinetools"]))?;
        let prefix = Project::output(Command::new("brew").arg("--prefix"))?;
        Ok(PathBuf::from(String::from_utf8_lossy(&prefix).trim())
            .join("share/android-commandlinetools"))
    }

    pub fn android_java() -> Result<PathBuf> {
        if let Some(path) = env::var_os("JAVA_HOME") { return Ok(PathBuf::from(path)); }
        let bundled = PathBuf::from("/Applications/Android Studio.app/Contents/jbr/Contents/Home");
        if bundled.join("bin/java").is_file() { return Ok(bundled); }
        if let Ok(output) = Command::new("/usr/libexec/java_home").args(["-v", "17+"]).output()
            && output.status.success() {
            return Ok(PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()));
        }
        Project::run(Command::new("brew").args(["install", "openjdk@17"]))?;
        let prefix = Project::output(Command::new("brew").args(["--prefix", "openjdk@17"]))?;
        Ok(PathBuf::from(String::from_utf8_lossy(&prefix).trim())
            .join("libexec/openjdk.jdk/Contents/Home"))
    }

    pub fn check_build() -> Result<()> { Ok(()) }
    pub fn default_target() -> &'static str { "macos" }
    pub fn require_apple_tools(_target: &str) -> Result<()> { Ok(()) }

    pub fn find_files(name: &str) -> Result<Vec<PathBuf>> {
        let output = Project::output(Command::new("/usr/bin/mdfind").args(["-0", "-name", name]))?;
        Ok(output
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(OsStr::from_bytes(path)))
            .filter(|path| path.file_name() == Some(OsStr::new(name)))
            .collect())
    }

    pub fn ios_devices() -> Result<Vec<IOSDevice>> {
        let output = Command::new("xcrun")
            .args(["xctrace", "list", "devices"])
            .stdout(Stdio::piped())
            .output()
            .map_err(|source| SharedError::CommandSpawn { program: "xcrun".into(), source })?;
        let stdout = str::from_utf8(&output.stdout)
            .map_err(|e| SharedError::ParseUTF8(e.to_string()))?;
        let mut in_devices_section = false;
        let mut devices = Vec::new();
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }
            if trimmed == "== Devices ==" { in_devices_section = true; continue; }
            if trimmed == "== Simulators ==" || trimmed == "== Devices Offline ==" {
                in_devices_section = false;
                continue;
            }
            if !in_devices_section { continue; }
            if !trimmed.to_lowercase().contains("iphone") { continue; }
            if let (Some(last_open), Some(last_close)) = (trimmed.rfind('('), trimmed.rfind(')'))
                && last_open < last_close {
                let id = trimmed[last_open + 1..last_close].trim().to_string();
                let model = trimmed[..last_open].trim().to_string();
                if id.len() > 6 { devices.push(IOSDevice { model, id }); }
            }
        }
        Ok(devices)
    }

    pub fn run_local(release: bool, cwd: PathBuf, env_vars: HashMap<String, String>, developer: Option<&Path>) -> Result<()> {
        let cargo_path = env_vars.get("cargo_path").map(String::as_str).unwrap_or("cargo");
        let cargo_args = format!("run {}", if release { "--release" } else { "" });
        Command::new("bash")
            .arg("-c")
            .arg(format!("{cargo_path} {cargo_args}"))
            .current_dir(cwd)
            .env("DEVELOPER_DIR", developer.unwrap())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|source| SharedError::CommandSpawn { program: "bash".into(), source })?;
        Ok(())
    }

    pub fn android_keystore(release: bool) -> Result<String> {
        let output = Project::output(&mut Command::new("whoami"))?;
        let user = String::from_utf8_lossy(&output).trim().to_string();
        if user.is_empty() { return Err(HostError::HostUsernameMissing.into()); }

        Ok(if release {
            format!("/Users/{}/.android/release.keystore", user)
        } else {
            format!("/Users/{}/.android/debug.keystore", user)
        })
    }

    pub fn make_executable(path: &Path) -> Result<()> {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|source| SharedError::SetPermissions { path: path.into(), source }.into())
    }

    pub fn private_permissions(path: &Path, directory: bool) -> Result<()> {
        fs::set_permissions(path, fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }))
            .map_err(|source| SharedError::SetPermissions { path: path.into(), source }.into())
    }
}

pub struct LinuxToolchain {
    zigbuild_path: String,
    homebrew_path: String,
}
impl LinuxToolchain {
    pub fn new(env_vars: &HashMap<String, String>) -> Result<Self> {
        let zigbuild_path = env_vars.get("zigbuild_path").cloned().ok_or_else(|| {
            SharedError::MissingSetting { key: "zigbuild_path".into() }
        })?;
        let homebrew_path = env_vars.get("homebrew_path").cloned().ok_or_else(|| {
            SharedError::MissingSetting { key: "homebrew_path".into() }
        })?;
        Ok(Self { zigbuild_path, homebrew_path })
    }

    pub fn build(&self, target: &str, release: bool, cwd: &Path) -> Result<()> {
        let cargo_args =
            format!("build --target {} {}", target, if release { "--release" } else { "" });
        let current_path = env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", self.homebrew_path, current_path);
        Project::run(
            Command::new("bash")
                .arg("-c")
                .arg(format!("{} {}", self.zigbuild_path, cargo_args))
                .current_dir(cwd)
                .env("PATH", new_path)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit()),
        )
    }
}

struct MacDevelopment {
    project: Project,
    developer: PathBuf,
    bundle_id: String,
    notifications: Option<PushBuild>,
    app_args: Vec<String>,
}

impl MacDevelopment {
    fn new(mut project: Project, app_args: Vec<String>, developer: PathBuf) -> Result<Self> {
        let target = project.args.cargo.target.clone().unwrap_or(Project::host_target()?);
        if !matches!(target.as_str(), "aarch64-apple-darwin" | "x86_64-apple-darwin") {
            return Err(SharedError::UnsupportedTarget(target).into());
        }
        project.args.cargo.target = Some(target.clone());
        Project::rust_target(&target)?;
        let manifest = Manifest::new(project.package.manifest_path.as_std_path())?;
        let settings = manifest.settings(&["macos"])?;
        let bundle_id = project.args.bundle_id.clone()
            .or_else(|| settings.get("bundle_id").cloned())
            .or(manifest.setting(&["ios", "bundle_id"])?
                .as_ref().and_then(toml_edit::Item::as_str).map(str::to_owned))
            .unwrap_or_else(|| format!("dev.piston.{}", project.package.name.replace('_', "-")));
        if !bundle_id.contains('.')
            || bundle_id.split('.').any(str::is_empty)
            || !bundle_id.chars().all(|character| {
                character.is_ascii_alphanumeric() || ".-".contains(character)
            })
        { return Err(SharedError::InvalidBundleId { value: bundle_id }.into()); }
        Ok(Self { project, developer, bundle_id, notifications: None, app_args })
    }

    fn project(&self) -> &Project { &self.project }
    fn developer(&self) -> &Path { &self.developer }
    fn notifications(&mut self, push: PushBuild) { self.notifications = Some(push); }

    fn notification_setup(&self, team: &str) -> Result<NotificationSetup> {
        let manifest = Manifest::new(self.project.package.manifest_path.as_std_path())?;
        manifest.save(&["macos"], &[("team", team)])?;
        NotificationSetup::new(manifest.path(), PushSigning {
            platform: PushPlatform::Macos, team: team.into(),
            topic: self.bundle_id.clone(), sandbox: true,
        })
    }

    fn build_command(&self) -> Command {
        let mut command = self.project.build_command();
        command.env("DEVELOPER_DIR", &self.developer);
        if let Some(push) = &self.notifications { push.compile(&mut command); }
        command
    }

    fn bundle(&self, executable: &Path) -> Result<PathBuf> {
        let project = &self.project;
        let name = project.package.name.as_str();
        let bundle = project.work.join(format!("{name}.app"));
        if bundle.exists() {
            fs::remove_dir_all(&bundle)
                .map_err(|error| SharedError::WriteFile(error.to_string()))?;
        }
        let contents = bundle.join("Contents");
        let binary = contents.join("MacOS").join(name);
        fs::create_dir_all(binary.parent().unwrap())
            .map_err(|error| SharedError::WriteFile(error.to_string()))?;
        fs::copy(executable, &binary)
            .map_err(|error| SharedError::WriteFile(error.to_string()))?;
        let manifest = Manifest::new(project.package.manifest_path.as_std_path())?;
        let config = manifest.settings(&["macos"])?;
        let source = project.args.info_plist.clone()
            .or_else(|| config.get("info_plist").map(PathBuf::from));
        let source = source.map(|path| project.root().join(path));
        let mut info = if let Some(source) = &source {
            Value::from_file(source)?
        } else { Value::Dictionary(Dictionary::new()) };
        let dict = info.as_dictionary_mut().ok_or_else(|| {
            crate::apple::cli::Error::PlistNotDictionary { path: source.unwrap() }
        })?;
        for (key, value) in [
            ("CFBundleIdentifier", self.bundle_id.clone()),
            ("CFBundleExecutable", name.into()),
            ("CFBundleName", name.into()),
            ("CFBundleDisplayName", name.into()),
            ("CFBundlePackageType", "APPL".into()),
            ("CFBundleInfoDictionaryVersion", "6.0".into()),
            ("CFBundleVersion", format!(
                "{}.{}.{}", project.package.version.major,
                project.package.version.minor, project.package.version.patch,
            )),
            ("CFBundleShortVersionString", format!(
                "{}.{}.{}", project.package.version.major,
                project.package.version.minor, project.package.version.patch,
            )),
        ] { dict.insert(key.into(), Value::String(value)); }
        if !dict.contains_key("NSHighResolutionCapable") {
            dict.insert("NSHighResolutionCapable".into(), Value::Boolean(true));
        }
        if let Some(icon) = Project::find_icon(project.root()) {
            let resource = contents.join("Resources");
            let iconset = project.work.join("AppIcon.iconset");
            fs::create_dir_all(&resource)
                .and_then(|_| fs::create_dir_all(&iconset))
                .map_err(|error| SharedError::WriteFile(error.to_string()))?;
            let image = image::open(&icon)
                .map_err(|source| SharedError::OpenImage { path: icon, source })?;
            for size in [16, 32, 128, 256, 512] {
                for scale in [1, 2] {
                    let suffix = if scale == 2 { "@2x" } else { "" };
                    image.resize_exact(
                        size * scale, size * scale, image::imageops::FilterType::Lanczos3,
                    )
                        .save(iconset.join(format!("icon_{size}x{size}{suffix}.png")))
                        .map_err(SharedError::WriteImage)?;
                }
            }
            Project::output(
                Command::new("iconutil").args(["-c", "icns"]).arg(&iconset)
                    .arg("-o").arg(resource.join("AppIcon.icns")),
            )?;
            dict.insert("CFBundleIconFile".into(), Value::String("AppIcon.icns".into()));
        }
        if let Some(resources) = manifest.setting(&["macos", "resources"])?
            .as_ref().and_then(toml_edit::Item::as_array)
        {
            for resource in resources {
                let resource = resource.as_str().ok_or(cli::Error::MacResourcesInvalid)?;
                let path = project.root().join(resource);
                let name = path.file_name()
                    .ok_or_else(|| cli::Error::MacResourceInvalid { path: path.clone() })?;
                Project::output(
                    Command::new("ditto").arg(&path).arg(contents.join("Resources").join(name)),
                )?;
            }
        }
        info.to_file_xml(contents.join("Info.plist"))
            .map_err(|error| SharedError::WriteFile(error.to_string()))?;
        let entitlements = project.args.entitlements.clone()
            .or_else(|| config.get("entitlements").map(PathBuf::from))
            .map(|path| project.root().join(path));
        if let Some(push) = &self.notifications {
            let entitlements = push.signing.entitlements(
                entitlements.as_deref(), &project.work.join("notifications.entitlements"),
            )?;
            let signed = XcodeSigning::new(&push.signing.team, &self.developer).sign_macos(
                &bundle, &project.work, &entitlements, false, false,
            )?;
            push.signing.verify(&signed)?;
            Ok(signed)
        } else {
            let mut sign = Command::new("codesign");
            sign.args(["--force", "--sign", "-", "--timestamp=none"]);
            if let Some(entitlements) = entitlements {
                sign.arg("--entitlements").arg(entitlements);
            }
            Project::output(sign.arg(&bundle))?;
            Ok(bundle)
        }
    }

    fn launch(&self, bundle: &Path) -> Result<()> {
        self.project.launch(
            &bundle.join("Contents/MacOS").join(self.project.package.name.as_str()),
            &self.app_args,
        )
    }
}
