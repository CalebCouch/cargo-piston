use serde_json::Value;
use std::path::Path;
use std::process::Command;
use crate::project::Project;
use crate::{PistonError, Result};
use super::application::IOSApplication;

pub struct Simulator {
    pub id: String,
    pub name: String,
    booted: bool,
}

impl Simulator {
    pub fn select(developer: &Path, selector: Option<&str>) -> Result<Self> {
        let mut listing = Self::list(developer)?;
        let available = |listing: &Value| {
            listing.get("runtimes").and_then(Value::as_array).is_some_and(|runtimes| {
                runtimes.iter().any(|runtime| {
                    runtime["isAvailable"] == true
                        && runtime["identifier"].as_str().is_some_and(|id| id.contains(".iOS-"))
                })
            })
        };
        if !available(&listing) {
            Project::run(
                Command::new("xcodebuild")
                    .env("DEVELOPER_DIR", developer)
                    .args(["-downloadPlatform", "iOS"]),
                "Installing the iOS Simulator runtime",
            )?;
            listing = Self::list(developer)?;
        }
        let mut devices = Vec::new();
        if let Some(runtimes) = listing["devices"].as_object() {
            for (runtime, values) in runtimes {
                if !runtime.contains(".iOS-") { continue; }
                for value in values.as_array().into_iter().flatten() {
                    if value["isAvailable"] != true { continue; }
                    let (Some(id), Some(name)) = (value["udid"].as_str(), value["name"].as_str())
                    else { continue; };
                    if selector.is_some_and(|selector| selector != id && selector != name) {
                        continue;
                    }
                    devices.push((
                        runtime.clone(),
                        Self {
                            id: id.into(),
                            name: name.into(),
                            booted: value["state"] == "Booted",
                        },
                    ));
                }
            }
        }
        devices.sort_by_key(|(runtime, device)| {
            (
                device.booted,
                runtime.split('-').filter_map(|part| part.parse::<u32>().ok()).collect::<Vec<_>>(),
                device.name.starts_with("iPhone"),
            )
        });
        if let Some((_, device)) = devices.pop() { return Ok(device); }
        if let Some(selector) = selector {
            return Err(PistonError::Generic(format!(
                "No connected iPhone or available Simulator matches {selector}. Use --simulator without --device to create one."
            )));
        }
        let runtimes = listing["runtimes"]
            .as_array()
            .ok_or_else(|| PistonError::Generic("Simulator returned no runtimes.".into()))?;
        let runtime = runtimes
            .iter()
            .filter(|runtime| {
                runtime["isAvailable"] == true
                    && runtime["identifier"].as_str().is_some_and(|id| id.contains(".iOS-"))
            })
            .max_by_key(|runtime| {
                runtime["version"]
                    .as_str()
                    .unwrap_or("")
                    .split('.')
                    .filter_map(|part| part.parse::<u32>().ok())
                    .collect::<Vec<_>>()
            })
            .and_then(|runtime| runtime["identifier"].as_str())
            .ok_or_else(|| {
                PistonError::Generic(
                    "Xcode installed no usable iOS runtime. Open Xcode Settings > Components."
                        .into(),
                )
            })?;
        let device_type = listing["devicetypes"]
            .as_array()
            .into_iter()
            .flatten()
            .rev()
            .find(|value| value["productFamily"] == "iPhone")
            .and_then(|value| value["identifier"].as_str())
            .ok_or_else(|| PistonError::Generic("Xcode returned no iPhone device types.".into()))?;
        let output = Project::output(
            IOSApplication::command(developer, "simctl").args([
                "create",
                "Piston iPhone",
                device_type,
                runtime,
            ]),
            "Creating iOS Simulator",
        )?;
        Ok(Self {
            id: String::from_utf8_lossy(&output).trim().into(),
            name: "Piston iPhone".into(),
            booted: false,
        })
    }

    pub fn boot(&self, developer: &Path) -> Result<()> {
        if !self.booted {
            Project::output(
                IOSApplication::command(developer, "simctl").args(["boot", &self.id]),
                "Booting iOS Simulator",
            )?;
        }
        let app = developer.join("Applications/Simulator.app");
        Project::run(
            Command::new("open").arg("-a").arg(app).args([
                "--args",
                "-CurrentDeviceUDID",
                &self.id,
            ]),
            "Opening Simulator",
        )?;
        Project::run(
            IOSApplication::command(developer, "simctl").args(["bootstatus", &self.id, "-b"]),
            "Waiting for Simulator",
        )
    }

    pub fn install(&self, developer: &Path, bundle: &Path) -> Result<()> {
        Project::output(
            Command::new("codesign")
                .args(["--force", "--sign", "-", "--timestamp=none"])
                .arg(bundle),
            "Signing simulator app",
        )?;
        Project::output(
            IOSApplication::command(developer, "simctl").args(["install", &self.id]).arg(bundle),
            "Installing simulator app",
        )?;
        Ok(())
    }

    pub fn launch(&self, developer: &Path, identifier: &str, arguments: &[String]) -> Result<()> {
        Project::run(
            IOSApplication::command(developer, "simctl")
                .args([
                    "launch",
                    "--console-pty",
                    "--terminate-running-process",
                    &self.id,
                    identifier,
                ])
                .args(arguments),
            "Running simulator app",
        )
    }

    fn list(developer: &Path) -> Result<Value> {
        let output = Project::output(
            IOSApplication::command(developer, "simctl").args(["list", "--json"]),
            "Listing iOS Simulators",
        )?;
        serde_json::from_slice(&output)
            .map_err(|error| PistonError::IntoJSONError(error.to_string()))
    }
}
