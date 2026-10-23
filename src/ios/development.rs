use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use crate::{PistonError, Result};
use super::application::IOSApplication;

pub struct IOSDevice {
    pub id: String,
    pub udid: String,
    pub name: String,
    connected: bool,
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
        let connected = value.pointer("/connectionProperties/tunnelState").and_then(Value::as_str)
            == Some("connected");
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
        Some(Self { id, udid, name, connected, wired, paired, developer_mode, os_version })
    }
}

pub struct IOSDevelopment {
    pub application: IOSApplication,
    device: Option<String>,
    pub developer: PathBuf,
}

impl IOSDevelopment {
    pub fn new(application: IOSApplication, device: Option<String>, developer: PathBuf) -> Self {
        Self { application, device, developer }
    }

    pub fn install(&self, bundle: &std::path::Path, device: &IOSDevice) -> Result<()> {
        let output = self
            .command("devicectl")
            .args(["device", "install", "app", "--device", &device.id])
            .arg(bundle)
            .arg("--quiet")
            .output()
            .map_err(|error| PistonError::XcrunInstallError(error.to_string()))?;
        IOSApplication::check(output, "Installing the app")?;
        Ok(())
    }

    pub fn launch(&self, identifier: &str, device: &IOSDevice, arguments: &[String]) -> Result<()> {
        let status = self
            .command("devicectl")
            .args([
                "device",
                "process",
                "launch",
                "--device",
                &device.id,
                "--terminate-existing",
                "--console",
                identifier,
            ])
            .arg("--")
            .args(arguments)
            .status()
            .map_err(|error| PistonError::XcrunLaunchError(error.to_string()))?;
        if !status.success() {
            return Err(PistonError::XcrunLaunchError(format!(
                "Device console exited with {status}"
            )));
        }
        Ok(())
    }
}

impl IOSDevelopment {
    fn command(&self, tool: &str) -> Command { IOSApplication::command(&self.developer, tool) }

    pub fn select_device(&self) -> Result<Option<IOSDevice>> {
        let mut devices = self.devices()?;
        if let Some(selector) = &self.device {
            devices.retain(|device| {
                &device.id == selector || &device.udid == selector || &device.name == selector
            });
        } else {
            devices.retain(|device| device.connected || device.wired);
            if devices.iter().any(|device| device.wired) { devices.retain(|device| device.wired); }
        }
        if devices.is_empty() { return Ok(None); }
        if devices.len() > 1 {
            let choices = devices
                .iter()
                .map(|device| format!("{} ({})", device.name, device.udid))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(PistonError::XcrunDevicectlError(format!(
                "Multiple devices match: {choices}. Select one with --device <name-or-UDID>."
            )));
        }
        let device = devices.remove(0);
        if !device.connected && !device.wired {
            return Err(PistonError::XcrunDevicectlError(format!(
                "{} is not connected.",
                device.name
            )));
        }
        Ok(Some(device))
    }

    pub fn pair(&self, device: &IOSDevice) -> Result<IOSDevice> {
        let output = self
            .command("devicectl")
            .args(["manage", "pair", "--device", &device.id, "--timeout", "60", "--json-output"])
            .arg(self.application.work().join("pairing.json"))
            .output()
            .map_err(|error| PistonError::XcrunDevicectlError(error.to_string()))?;
        IOSApplication::check(
            output,
            "Pairing with the phone; unlock it and accept Trust This Computer",
        )?;
        let device = self.devices()?.into_iter().find(|current| current.udid == device.udid)
            .ok_or_else(|| PistonError::XcrunDevicectlError(
                "The selected phone disappeared after pairing. Reconnect and unlock it, then rerun.".into()
            ))?;
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
            .map_err(|error| PistonError::XcrunDevicectlError(error.to_string()))?;
        IOSApplication::check(
            output,
            "Connecting to the selected device; unlock it and accept any Trust prompt",
        )?;
        for attempt in 0..5 {
            let current = self
                .devices()?
                .into_iter()
                .find(|current| current.udid == device.udid)
                .ok_or_else(|| {
                PistonError::XcrunDevicectlError(format!(
                    "{} disconnected while connecting. Reconnect and unlock it, then rerun.",
                    device.name
                ))
            })?;
            if current.developer_mode.as_deref() == Some("disabled") {
                let version = current
                    .os_version
                    .as_ref()
                    .map(|version| format!(" (iOS {version})"))
                    .unwrap_or_default();
                return Err(PistonError::XcrunDevicectlError(format!(
                    "Developer Mode is disabled on {}{version}. Pairing succeeded.\n\n\
                    1. Reopen Settings on the phone and scroll to the bottom of Privacy & Security.\n\
                    2. Turn on Developer Mode and accept the restart.\n\
                    3. Unlock the phone after restart and confirm Turn On.\n\
                    4. Rerun the same cargo piston dev command.\n\n\
                    If the setting is missing, reopen Settings after pairing. If it is still missing, \
                    select the phone in Xcode's device manager and let Xcode prepare it.\n\
                    Apple requires this confirmation on the phone.",
                    current.name
                )));
            }
            if current.paired && current.connected { return Ok(current); }
            if attempt < 4 {
                std::thread::sleep(std::time::Duration::from_secs(1));
            } else {
                return Err(PistonError::XcrunDevicectlError(format!(
                    "{} is attached, but Apple's development connection is not ready. \
                    Keep it unlocked, accept Trust This Computer, and enable \
                    Settings > Privacy & Security > Developer Mode, then rerun.",
                    current.name
                )));
            }
        }
        unreachable!()
    }

    fn devices(&self) -> Result<Vec<IOSDevice>> {
        let path = self.application.work().join("devices.json");
        let output = self
            .command("devicectl")
            .args(["list", "devices", "--timeout", "20", "--json-output"])
            .arg(&path)
            .output()
            .map_err(|error| PistonError::XcrunDevicectlError(error.to_string()))?;
        IOSApplication::check(output, "Discovering iPhones with Apple's CoreDevice service")?;
        let contents = fs::read(&path).map_err(|error| PistonError::Generic(error.to_string()))?;
        let document: Value = serde_json::from_slice(&contents)
            .map_err(|error| PistonError::IntoJSONError(error.to_string()))?;
        let values =
            document.pointer("/result/devices").and_then(Value::as_array).ok_or_else(|| {
                PistonError::IntoJSONError("devicectl returned no device list.".into())
            })?;
        Ok(values.iter().filter_map(IOSDevice::read).collect())
    }
}
