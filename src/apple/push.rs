use plist::{Dictionary, Value};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use crate::{PistonError, Result};

#[derive(Clone, Copy)]
pub enum PushPlatform { Ios, Macos, }

impl PushPlatform {
    pub fn name(self) -> &'static str { match self { Self::Ios => "ios", Self::Macos => "macos", } }

    pub fn entitlement(self) -> &'static str {
        match self {
            Self::Ios => "aps-environment",
            Self::Macos => "com.apple.developer.aps-environment",
        }
    }

    pub fn profile(self) -> &'static str {
        match self {
            Self::Ios => "embedded.mobileprovision",
            Self::Macos => "Contents/embedded.provisionprofile",
        }
    }
}

pub struct PushSigning {
    pub platform: PushPlatform,
    pub team: String,
    pub topic: String,
    pub sandbox: bool,
}

pub struct PushBuild {
    pub application: String,
    pub signing: PushSigning,
}

impl PushBuild {
    pub fn compile(&self, command: &mut Command) {
        command.env("RAMP_NOTIFICATION_APPLICATION", &self.application);
    }
}

impl PushSigning {
    pub fn verify_ipa(&self, ipa: &Path, work: &Path) -> Result<()> {
        let output = work.join("notification-verification");
        if output.exists() {
            std::fs::remove_dir_all(&output)
                .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        }
        let extracted = Command::new("ditto")
            .args(["-x", "-k"])
            .arg(ipa)
            .arg(&output)
            .output()
            .map_err(|error| PistonError::CodesignError(error.to_string()))?;
        if !extracted.status.success() {
            return Err(PistonError::CodesignError(
                "Cannot inspect the exported IPA's push signature.".into(),
            ));
        }
        let apps = std::fs::read_dir(output.join("Payload"))
            .map_err(|error| PistonError::CodesignError(error.to_string()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|error| PistonError::CodesignError(error.to_string()))?;
        let apps: Vec<_> = apps
            .into_iter()
            .filter(|path| path.extension().is_some_and(|value| value == "app"))
            .collect();
        let result = if let [app] = apps.as_slice() {
            self.verify(app)
        } else {
            Err(PistonError::CodesignError("The exported IPA must contain one app.".into()))
        };
        std::fs::remove_dir_all(output)
            .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        result
    }

    pub fn entitlements(&self, source: Option<&Path>, output: &Path) -> Result<PathBuf> {
        let mut entitlements = match source {
            Some(path) => Value::from_file(path).map_err(|error| {
                PistonError::CodesignError(format!("Cannot read {}: {error}", path.display()))
            })?,
            None => Value::Dictionary(Dictionary::new()),
        };
        let dictionary = entitlements.as_dictionary_mut().ok_or_else(|| {
            PistonError::CodesignError(
                "App entitlements must be a property-list dictionary.".into(),
            )
        })?;
        // Xcode's archive uses development signing; export selects the distribution environment.
        dictionary.insert(self.platform.entitlement().into(), Value::String("development".into()));
        entitlements.to_file_xml(output).map_err(|error| {
            PistonError::CodesignError(format!("Cannot write {}: {error}", output.display()))
        })?;
        Ok(output.to_path_buf())
    }

    pub fn verify(&self, bundle: &Path) -> Result<()> {
        let verification = Command::new("codesign")
            .args(["--verify", "--strict", "--all-architectures"])
            .arg(bundle)
            .output()
            .map_err(|error| PistonError::CodesignError(error.to_string()))?;
        if !verification.status.success() {
            return Err(PistonError::CodesignError(format!(
                "The notification app has an invalid signature: {}",
                String::from_utf8_lossy(&verification.stderr).trim(),
            )));
        }
        let profile = Command::new("security")
            .args(["cms", "-D", "-i"])
            .arg(bundle.join(self.platform.profile()))
            .output()
            .map_err(|error| PistonError::CodesignError(error.to_string()))?;
        if !profile.status.success() {
            return Err(PistonError::CodesignError(format!(
                "Push notifications require an embedded provisioning profile for {}. \
                 Enable Push Notifications for this App ID and rebuild with automatic signing.",
                self.topic,
            )));
        }
        let profile = Value::from_reader(Cursor::new(profile.stdout)).map_err(|error| {
            PistonError::CodesignError(format!("Invalid provisioning profile: {error}"))
        })?;
        let info_path = match self.platform {
            PushPlatform::Ios => "Info.plist",
            PushPlatform::Macos => "Contents/Info.plist",
        };
        let info = Value::from_file(bundle.join(info_path)).map_err(|error| {
            PistonError::CodesignError(format!("Invalid app Info.plist: {error}"))
        })?;
        if info
            .as_dictionary()
            .and_then(|value| value.get("CFBundleIdentifier"))
            .and_then(Value::as_string)
            != Some(&self.topic)
        {
            return Err(PistonError::CodesignError(
                "The app bundle ID does not match its APNs topic.".into(),
            ));
        }
        let executable = info
            .as_dictionary()
            .and_then(|value| value.get("CFBundleExecutable"))
            .and_then(Value::as_string)
            .ok_or_else(|| {
                PistonError::CodesignError("App Info.plist has no executable.".into())
            })?;
        let executable = match self.platform {
            PushPlatform::Ios => bundle.join(executable),
            PushPlatform::Macos => bundle.join("Contents/MacOS").join(executable),
        };
        let architectures = Command::new("/usr/bin/lipo")
            .arg("-archs")
            .arg(executable)
            .output()
            .map_err(|error| PistonError::CodesignError(error.to_string()))?;
        if !architectures.status.success() || architectures.stdout.is_empty() {
            return Err(PistonError::CodesignError(
                "Cannot inspect the app's signed architectures.".into(),
            ));
        }
        for architecture in String::from_utf8_lossy(&architectures.stdout).split_whitespace() {
            let signed = Command::new("codesign")
                .args(["--display", "--entitlements", "-", "--xml", "--arch", architecture])
                .arg(bundle)
                .output()
                .map_err(|error| PistonError::CodesignError(error.to_string()))?;
            if !signed.status.success() {
                return Err(PistonError::CodesignError(format!(
                    "Cannot inspect notification entitlements for {architecture}.",
                )));
            }
            let entitlements = Value::from_reader(Cursor::new(signed.stdout)).map_err(|error| {
                PistonError::CodesignError(format!("Invalid signed entitlements: {error}"))
            })?;
            self.validate(&profile, &entitlements)?;
        }
        Ok(())
    }

    fn validate(&self, profile: &Value, signed: &Value) -> Result<()> {
        let profile = profile.as_dictionary().ok_or_else(|| {
            PistonError::CodesignError("Invalid provisioning profile dictionary.".into())
        })?;
        let allowed =
            profile.get("Entitlements").and_then(Value::as_dictionary).ok_or_else(|| {
                PistonError::CodesignError(
                    "The provisioning profile contains no entitlements.".into(),
                )
            })?;
        let signed = signed.as_dictionary().ok_or_else(|| {
            PistonError::CodesignError("Invalid signed entitlements dictionary.".into())
        })?;
        let environment = if self.sandbox { "development" } else { "production" };
        let key = self.platform.entitlement();
        if signed.get(key).and_then(Value::as_string) != Some(environment)
            || allowed.get(key).and_then(Value::as_string) != Some(environment)
        {
            return Err(PistonError::CodesignError(format!(
                "The signed app and its provisioning profile must both grant {key}={environment}. \
                 The notification server is configured for that environment. Enable Push Notifications \
                 for {} and rebuild using the matching signing profile.",
                self.topic,
            )));
        }
        let teams = profile.get("TeamIdentifier").and_then(Value::as_array);
        if !teams.is_some_and(|teams| teams.iter().any(|team| team.as_string() == Some(&self.team)))
            || signed.get("com.apple.developer.team-identifier").and_then(Value::as_string)
                != Some(&self.team)
        {
            return Err(PistonError::CodesignError(
                "The signed app and APNs credentials must belong to the same Apple team.".into(),
            ));
        }
        let identifier = match self.platform {
            PushPlatform::Ios => "application-identifier",
            PushPlatform::Macos => "com.apple.application-identifier",
        };
        let prefixes = profile.get("ApplicationIdentifierPrefix").and_then(Value::as_array);
        let application = signed.get(identifier).and_then(Value::as_string);
        if !prefixes.is_some_and(|prefixes| {
            prefixes
                .iter()
                .filter_map(Value::as_string)
                .any(|prefix| application == Some(format!("{prefix}.{}", self.topic).as_str()))
        }) || allowed.get(identifier).and_then(Value::as_string) != application
        {
            return Err(PistonError::CodesignError(
                "Push signing requires an explicit provisioning profile matching the app's APNs topic.".into(),
            ));
        }
        let expires = profile.get("ExpirationDate").and_then(Value::as_date).map(SystemTime::from);
        if expires.is_none_or(|expires| expires <= SystemTime::now()) {
            return Err(PistonError::CodesignError(
                "The push provisioning profile has expired.".into(),
            ));
        }
        Ok(())
    }
}
