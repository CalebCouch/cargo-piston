use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::host::cli::Error as HostError;
use crate::cli::Error as SharedError;
use crate::project::Project;
use crate::devices::IOSDevice;
use crate::Result;

pub use super::Cli;

pub mod cli;
use cli::Error;

pub struct Host;

impl Host {
    pub fn android_sdk() -> Result<PathBuf> {
        let root = std::env::var_os("ANDROID_HOME")
            .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Android/Sdk")
            });
        if root.join("cmdline-tools").is_dir() { return Ok(root); }
        Err(Error::HostAndroidSdkMissing { path: root }.into())
    }

    pub fn android_java() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os("JAVA_HOME") { return Ok(PathBuf::from(path)); }
        let output = Project::output(Command::new("sh").args(["-c", "command -v java"]))?;
        let path = PathBuf::from(String::from_utf8_lossy(&output).trim());
        let java = fs::canonicalize(&path)
            .map_err(|source| Error::HostJavaMissing { path, source })?;
        Ok(java.parent().unwrap().parent().unwrap().to_path_buf())
    }

    pub fn check_build() -> Result<()> { Err(Error::HostBuildUnsupported.into()) }

    pub fn default_target() -> &'static str { "linux" }
    pub fn require_apple_tools(target: &str) -> Result<()> {
        Err(Error::UnsupportedOSError { os: "linux".into(), target: target.into() }.into())
    }

    pub fn find_files(_name: &str) -> Result<Vec<PathBuf>> { Ok(Vec::new()) }

    pub fn ios_devices() -> Result<Vec<IOSDevice>> { Ok(Vec::new()) }

    pub fn run_local(release: bool, cwd: PathBuf, env_vars: HashMap<String, String>, _developer: Option<&Path>) -> Result<()> {
        let cargo_path = env_vars.get("cargo_path").map(String::as_str).unwrap_or("cargo");
        let cargo_args = format!("run {}", if release { "--release" } else { "" });
        Command::new("bash")
            .arg("-c")
            .arg(format!("{cargo_path} {cargo_args}"))
            .current_dir(cwd)
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
    cargo_path: String,
}
impl LinuxToolchain {
    pub fn new(env_vars: &HashMap<String, String>) -> Result<Self> {
        let cargo_path = env_vars.get("cargo_path").cloned().unwrap_or("cargo".to_string());
        Ok(Self { cargo_path })
    }

    pub fn build(&self, target: &str, release: bool, cwd: &Path) -> Result<()> {
        let cargo_args =
            format!("build --target {} {}", target, if release { "--release" } else { "" });
        let cargo_cmd = format!("{} {}", self.cargo_path, cargo_args);
        Project::run(
            Command::new("bash")
                .arg("-c")
                .arg(&cargo_cmd)
                .current_dir(cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit()),
        )
    }
}
