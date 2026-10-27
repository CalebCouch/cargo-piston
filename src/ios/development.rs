use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use crate::{cli, Result};
use super::cli::Error;
use super::application::IOSApplication;

pub struct IOSDevice {
    pub id: String,
    pub udid: String,
    pub name: String,
    connected: bool,
    available: bool,
    wired: bool,
    pub paired: bool,
    developer_mode: Option<String>,
    os_version: Option<String>,
}

impl IOSDevice {
    fn read(value: &Value) -> Option<Self> {
        if value.pointer("/hardwareProperties/platform")?.as_str()? != "iOS" { return None; }
        let id = value.get("identifier")?.as_str()?.to_owned();
        let udid = value.pointer("/hardwareProperties/udid")?.as_str()?.to_owned();
        let name = value.pointer("/deviceProperties/name")?.as_str()?.to_owned();
        let wired = value.pointer("/connectionProperties/transportType").and_then(Value::as_str)
            == Some("wired");
        let tunnel = value.pointer("/connectionProperties/tunnelState").and_then(Value::as_str);
        let connected = tunnel == Some("connected");
        let available = wired || matches!(tunnel, Some("connected" | "disconnected"));
        let paired = match value.pointer("/connectionProperties/pairingState")?.as_str()? {
            "paired" => true,
            "unpaired" => false,
            _ => return None,
        };
        let developer_mode = value
            .pointer("/deviceProperties/developerModeStatus")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let os_version = value
            .pointer("/deviceProperties/osVersionNumber")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Some(Self { id, udid, name, connected, available, wired, paired, developer_mode, os_version })
    }
}

pub struct IOSDevelopment {
    pub application: IOSApplication,
    pub developer: PathBuf,
}

impl IOSDevelopment {
    pub fn new(application: IOSApplication, developer: PathBuf) -> Self {
        Self { application, developer }
    }

    pub fn install(&self, bundle: &std::path::Path, device: &IOSDevice) -> Result<()> {
        let output = self
            .command("devicectl")
            .args(["device", "install", "app", "--device", &device.id])
            .arg(bundle)
            .arg("--quiet")
            .output()
            .map_err(|error| Error::XcrunInstallError(error.to_string()))?;
        IOSApplication::check(output, "devicectl")?;
        Ok(())
    }

    pub fn launch(&self, identifier: &str, device: &IOSDevice, arguments: &[String]) -> Result<()> {
        let diagnostics = tempfile::tempdir()
            .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
        let lock_state = diagnostics.path().join("lock-state.json");
        let output = self.command("devicectl")
            .args([
                "device", "info", "lockState", "--device", &device.id,
                "--quiet", "--timeout", "10", "--json-output",
            ])
            .arg(&lock_state).output()
            .map_err(|error| Error::XcrunDevicectlError(error.to_string()))?;
        IOSApplication::check(output, "devicectl")?;
        let contents = fs::read(&lock_state)
            .map_err(|source| cli::Error::ReadFile { path: lock_state, source })?;
        let document: Value = serde_json::from_slice(&contents)
            .map_err(|error| cli::Error::IntoJSONError(error.to_string()))?;
        let locked = document.pointer("/result/passcodeRequired").and_then(Value::as_bool)
            .ok_or(Error::IosLockStateMissing)?;
        if locked { return Err(Error::IosDeviceLocked { name: device.name.clone() }.into()); }

        let launch_result = diagnostics.path().join("launch.json");
        let status = self
            .command("devicectl")
            .args([
                "device", "process", "launch", "--device", &device.id,
                "--terminate-existing", "--console", "--json-output",
            ])
            .arg(&launch_result).arg(identifier).arg("--").args(arguments).status()
            .map_err(|error| Error::XcrunLaunchError(error.to_string()))?;
        if status.success() { return Ok(()); }
        if status.code() == Some(1) {
            let document = fs::read(&launch_result).ok()
                .and_then(|contents| serde_json::from_slice::<Value>(&contents).ok());
            let mut error = document.as_ref().and_then(|document| document.get("error"));
            while let Some(cause) = error {
                if cause.get("domain").and_then(Value::as_str)
                    == Some("FBSOpenApplicationErrorDomain")
                    && cause.get("code").and_then(Value::as_u64) == Some(7)
                {
                    return Err(Error::IosDeviceLocked { name: device.name.clone() }.into());
                }
                error = cause.pointer("/userInfo/NSUnderlyingError/error");
            }
        }
        Err(cli::Error::CommandStatus { program: "devicectl".into(), status }.into())
    }

    pub fn select_device(&self, selector: Option<&str>) -> Result<Option<IOSDevice>> {
        let mut devices = self.devices()?;
        if let Some(selector) = selector {
            devices.retain(|device| {
                device.id == selector || device.udid == selector || device.name == selector
            });
        } else {
            devices.retain(|device| device.available);
            if devices.iter().any(|device| device.wired) { devices.retain(|device| device.wired); }
        }
        if devices.is_empty() { return Ok(None); }
        if devices.len() > 1 {
            let devices = devices.into_iter().map(|device| (device.name, device.udid)).collect();
            return Err(Error::IosDevicesAmbiguous { devices }.into());
        }
        let device = devices.remove(0);
        if !device.available {
            return Err(Error::IosDeviceUnavailable { name: device.name }.into());
        }
        Ok(Some(device))
    }

    pub fn pair(&self, device: &IOSDevice) -> Result<IOSDevice> {
        let output = self
            .command("devicectl")
            .args(["manage", "pair", "--device", &device.id, "--timeout", "60", "--json-output"])
            .arg(self.application.work().join("pairing.json"))
            .output()
            .map_err(|error| Error::XcrunDevicectlError(error.to_string()))?;
        IOSApplication::check(output, "devicectl")?;
        let device = self.devices()?.into_iter().find(|current| current.udid == device.udid)
            .ok_or_else(|| Error::IosDeviceDisappeared { name: device.name.clone() })?;
        Ok(device)
    }

    pub fn connect(&self, device: &IOSDevice) -> Result<IOSDevice> {
        let output = self
            .command("devicectl")
            .args([
                "device",
                "info",
                "details",
                "--device",
                &device.id,
                "--timeout",
                "20",
                "--json-output",
            ])
            .arg(self.application.work().join("selected-device.json"))
            .output()
            .map_err(|error| Error::XcrunDevicectlError(error.to_string()))?;
        IOSApplication::check(output, "devicectl")?;
        for attempt in 0..5 {
            let current = self
                .devices()?
                .into_iter()
                .find(|current| current.udid == device.udid)
                .ok_or_else(|| Error::IosDeviceDisappeared { name: device.name.clone() })?;
            if current.developer_mode.as_deref() == Some("disabled") {
                return Err(Error::IosDeveloperModeDisabled {
                    name: current.name, version: current.os_version,
                }.into());
            }
            if current.paired && current.connected { return Ok(current); }
            if attempt < 4 {
                std::thread::sleep(std::time::Duration::from_secs(1));
            } else {
                return Err(Error::IosConnectionNotReady { name: current.name }.into());
            }
        }
        unreachable!()
    }

}

impl IOSDevelopment {
    fn command(&self, tool: &str) -> Command { IOSApplication::command(&self.developer, tool) }

    fn devices(&self) -> Result<Vec<IOSDevice>> {
        let path = self.application.work().join("devices.json");
        let output = self
            .command("devicectl")
            .args(["list", "devices", "--timeout", "20", "--json-output"])
            .arg(&path)
            .output()
            .map_err(|error| Error::XcrunDevicectlError(error.to_string()))?;
        IOSApplication::check(output, "devicectl")?;
        let contents = fs::read(&path)?;
        let document: Value = serde_json::from_slice(&contents)
            .map_err(|error| cli::Error::IntoJSONError(error.to_string()))?;
        let values = document.pointer("/result/devices").and_then(Value::as_array)
            .ok_or(Error::IosDeviceListMissing)?;
        Ok(values.iter().filter_map(IOSDevice::read).collect())
    }
}
