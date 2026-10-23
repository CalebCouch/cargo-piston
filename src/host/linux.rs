use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::project::Project;
use crate::devices::IOSDevice;
use crate::{PistonError, Result};

#[path = "linux/cli.rs"]
pub mod cli;

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
        Err(PistonError::Generic("Install Android SDK Command-line Tools and set ANDROID_HOME; Piston installs the remaining SDK components. https://developer.android.com/studio#command-tools".into()))
    }

    pub fn android_java() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os("JAVA_HOME") { return Ok(PathBuf::from(path)); }
        let output = crate::project::Project::output(
            Command::new("sh").args(["-c", "command -v java"]),
            "Finding Java",
        )?;
        let java = fs::canonicalize(String::from_utf8_lossy(&output).trim()).map_err(|error| {
            PistonError::Generic(format!("Install JDK 17 or newer and set JAVA_HOME: {error}"))
        })?;
        Ok(java.parent().unwrap().parent().unwrap().to_path_buf())
    }

    pub fn check_build() -> Result<()> {
        Err(PistonError::Generic(
            "Linux host support not yet implemented. Please try Cargo Piston on MacOS.".into(),
        ))
    }

    pub fn default_target() -> &'static str { "linux" }
    pub fn require_apple_tools(target: &str) -> Result<()> {
        Err(PistonError::UnsupportedOSError { os: "linux".into(), target: target.into() })
    }

    pub fn open_url(_url: &str) -> Result<()> { Ok(()) }
    pub fn open_accounts(_developer: &Path) -> Result<()> {
        Err(PistonError::Generic("Xcode account setup requires macOS.".into()))
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
            .map_err(|error| PistonError::BuildError(format!("Cargo Run failed: {error}")))?;
        Ok(())
    }

    pub fn android_keystore(release: bool) -> Result<String> {
        let user_output = Command::new("whoami")
            .output()
            .map_err(|e| PistonError::WhoAmIError(format!("Failed to run 'whoami': {}", e)))?;

        if !user_output.status.success() {
            return Err(PistonError::WhoAmIError(format!(
                "Failed to run 'whoami': {}",
                String::from_utf8_lossy(&user_output.stderr)
            )));
        }
        let user = String::from_utf8_lossy(&user_output.stdout).trim().to_string();
        if user.is_empty() {
            return Err(PistonError::WhoAmIError(format!(
                "Failed to obtain user id with whoami: {}",
                String::from_utf8_lossy(&user_output.stderr)
            )));
        }

        Ok(if release {
            format!("/Users/{}/.android/release.keystore", user)
        } else {
            format!("/Users/{}/.android/debug.keystore", user)
        })
    }

    pub fn make_executable(path: &Path) -> Result<()> {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|e| PistonError::Generic(format!("Failed to make binary executable: {}", e)))
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
        let builder = Command::new("bash")
            .arg("-c")
            .arg(&cargo_cmd)
            .current_dir(cwd)
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
        Ok(())
    }
}

pub struct Credentials;

impl Credentials {
    pub fn needs_setup() -> Result<bool> { Ok(false) }

    pub fn new() -> Result<Self> {
        Err(PistonError::Generic("Apple notification setup requires macOS Keychain.".into()))
    }

    pub fn get(&self, _account: &str) -> Option<&[u8]> { None }

    pub fn accounts(&self) -> Vec<String> { Vec::new() }

    pub fn save(&mut self, _account: &str, _secret: Vec<u8>) -> Result<()> {
        Err(PistonError::Generic("Apple notification setup requires macOS Keychain.".into()))
    }
}
