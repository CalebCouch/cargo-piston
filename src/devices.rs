use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use crate::error::PistonError;
use crate::host::Host;

pub mod cli;
pub use cli::Cli;

#[derive(Debug, Clone)]
pub struct IOSDevice {
    pub model: String,
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct AndroidDevice {
    pub id: String,
}

#[derive(Debug)]
pub struct Devices {
    pub android_available: bool,
    pub ios: Vec<IOSDevice>,
    pub android: Vec<AndroidDevice>,
}

impl Devices {
    pub fn discover(env_vars: HashMap<String, String>) -> Result<Self, PistonError> {
        let sdk_path = env_vars.get("sdk_path").map(String::as_str).unwrap_or_default();
        let adb_path = format!("{sdk_path}/platform-tools/adb");
        let android_available = Path::new(&adb_path).exists();
        let android =
            if android_available { Self::android_devices(&adb_path)? } else { Vec::new() };
        let ios = Host::ios_devices()?;
        let devices = Self { ios, android, android_available };
        Ok(devices)
    }

    fn android_devices(adb_path: &str) -> Result<Vec<AndroidDevice>, PistonError> {
        let output = Command::new(adb_path)
            .arg("devices")
            .output()
            .map_err(|e| PistonError::ADBDevicesError(e.to_string()))?;
        let stdout = str::from_utf8(&output.stdout)
            .map_err(|e| PistonError::ParseUTF8Error(e.to_string()))?;
        Ok(stdout
            .lines()
            .skip(1)
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let id = parts.next()?;
                if parts.next() == Some("device") && parts.next().is_none() {
                    Some(AndroidDevice { id: id.to_string() })
                } else {
                    None
                }
            })
            .collect())
    }
}
