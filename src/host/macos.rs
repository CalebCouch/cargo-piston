use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::devices::IOSDevice;
use crate::project::Project;
use crate::{PistonError, Result};

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
            env::var_os("HOME").ok_or_else(|| PistonError::MissingEnvironment { name: "HOME" })?,
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
        if let Ok(output) = Command::new("/usr/libexec/java_home").args(["-v", "17+"]).output() {
            if output.status.success() {
                return Ok(PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()));
            }
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
            .map_err(|source| PistonError::CommandSpawn { program: "xcrun".into(), source })?;
        let stdout = str::from_utf8(&output.stdout)
            .map_err(|e| PistonError::ParseUTF8Error(e.to_string()))?;
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
            if let (Some(last_open), Some(last_close)) = (trimmed.rfind('('), trimmed.rfind(')')) {
                if last_open < last_close {
                    let id = trimmed[last_open + 1..last_close].trim().to_string();
                    let model = trimmed[..last_open].trim().to_string();
                    if id.len() > 6 { devices.push(IOSDevice { model, id }); }
                }
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
            .map_err(|source| PistonError::CommandSpawn { program: "bash".into(), source })?;
        Ok(())
    }

    pub fn android_keystore(release: bool) -> Result<String> {
        let output = Project::output(&mut Command::new("whoami"))?;
        let user = String::from_utf8_lossy(&output).trim().to_string();
        if user.is_empty() { return Err(PistonError::HostUsernameMissing); }

        Ok(if release {
            format!("/Users/{}/.android/release.keystore", user)
        } else {
            format!("/Users/{}/.android/debug.keystore", user)
        })
    }

    pub fn make_executable(path: &Path) -> Result<()> {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|source| PistonError::SetPermissions { path: path.into(), source })
    }

    pub fn private_permissions(path: &Path, directory: bool) -> Result<()> {
        fs::set_permissions(path, fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }))
            .map_err(|source| PistonError::SetPermissions { path: path.into(), source })
    }
}

pub struct LinuxToolchain {
    zigbuild_path: String,
    homebrew_path: String,
}
impl LinuxToolchain {
    pub fn new(env_vars: &HashMap<String, String>) -> Result<Self> {
        let zigbuild_path = env_vars.get("zigbuild_path").cloned().ok_or_else(|| {
            PistonError::MissingSetting { key: "zigbuild_path".into() }
        })?;
        let homebrew_path = env_vars.get("homebrew_path").cloned().ok_or_else(|| {
            PistonError::MissingSetting { key: "homebrew_path".into() }
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
