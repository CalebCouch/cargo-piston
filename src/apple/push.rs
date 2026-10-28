use plist::{Dictionary, Value};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use crate::Result;
use super::cli;

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
                .map_err(|source| crate::cli::Error::RemoveSubdir { path: output.clone(), source })?;
        }
        let extracted = Command::new("ditto")
            .args(["-x", "-k"])
            .arg(ipa)
            .arg(&output)
            .output()
            .map_err(|source| crate::cli::Error::CommandSpawn { program: "ditto".into(), source })?;
        if !extracted.status.success() {
            return Err(crate::cli::Error::CommandFailed { program: "ditto".into(), output: extracted }.into());
        }
        let apps = std::fs::read_dir(output.join("Payload"))
            .map_err(|source| crate::cli::Error::ReadDir { path: output.join("Payload"), source })?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|source| crate::cli::Error::MapDir { path: output.join("Payload"), source })?;
        let apps: Vec<_> = apps
            .into_iter()
            .filter(|path| path.extension().is_some_and(|value| value == "app"))
            .collect();
        let result = if let [app] = apps.as_slice() {
            self.verify(app)
        } else {
            Err(cli::Error::PushIpaApplicationCount { count: apps.len() }.into())
        };
        std::fs::remove_dir_all(&output)
            .map_err(|source| crate::cli::Error::RemoveSubdir { path: output.clone(), source })?;
        result
    }

    pub fn entitlements(&self, source: Option<&Path>, output: &Path) -> Result<PathBuf> {
        let mut entitlements = match source {
            Some(path) => Value::from_file(path).map_err(|error| {
                cli::Error::ApplePlistRead { path: path.to_path_buf(), source: error }
            })?,
            None => Value::Dictionary(Dictionary::new()),
        };
        let dictionary = entitlements.as_dictionary_mut().ok_or_else(|| {
            cli::Error::PlistNotDictionary { path: source.unwrap().to_path_buf() }
        })?;
        // Xcode's archive uses development signing; export selects the distribution environment.
        dictionary.insert(self.platform.entitlement().into(), Value::String("development".into()));
        entitlements.to_file_xml(output).map_err(|error| {
            cli::Error::ApplePlistWrite { path: output.to_path_buf(), source: error }
        })?;
        Ok(output.to_path_buf())
    }

    pub fn verify(&self, bundle: &Path) -> Result<()> {
        let verification = Command::new("codesign")
            .args(["--verify", "--strict", "--all-architectures"])
            .arg(bundle)
            .output()
            .map_err(|source| crate::cli::Error::CommandSpawn { program: "codesign".into(), source })?;
        if !verification.status.success() {
            return Err(crate::cli::Error::CommandFailed {
                program: "codesign".into(), output: verification,
            }.into());
        }
        let profile = Command::new("security")
            .args(["cms", "-D", "-i"])
            .arg(bundle.join(self.platform.profile()))
            .output()
            .map_err(|source| crate::cli::Error::CommandSpawn { program: "security".into(), source })?;
        if !profile.status.success() {
            return Err(cli::Error::PushProfileMissing { topic: self.topic.clone(), output: profile }.into());
        }
        let profile = Value::from_reader(Cursor::new(profile.stdout)).map_err(|error| {
            cli::Error::ApplePlistRead { path: bundle.join(self.platform.profile()), source: error }
        })?;
        let info_path = match self.platform {
            PushPlatform::Ios => "Info.plist",
            PushPlatform::Macos => "Contents/Info.plist",
        };
        let info = Value::from_file(bundle.join(info_path)).map_err(|error| {
            cli::Error::ApplePlistRead { path: bundle.join(info_path), source: error }
        })?;
        if info
            .as_dictionary()
            .and_then(|value| value.get("CFBundleIdentifier"))
            .and_then(Value::as_string)
            != Some(&self.topic)
        {
            return Err(cli::Error::PushTopicMismatch { topic: self.topic.clone() }.into());
        }
        let executable = info
            .as_dictionary()
            .and_then(|value| value.get("CFBundleExecutable"))
            .and_then(Value::as_string)
            .ok_or_else(|| {
                cli::Error::ApplePlistField { key: "CFBundleExecutable".into() }
            })?;
        let executable = match self.platform {
            PushPlatform::Ios => bundle.join(executable),
            PushPlatform::Macos => bundle.join("Contents/MacOS").join(executable),
        };
        let architectures = Command::new("/usr/bin/lipo")
            .arg("-archs")
            .arg(executable)
            .output()
            .map_err(|source| crate::cli::Error::CommandSpawn { program: "/usr/bin/lipo".into(), source })?;
        if !architectures.status.success() || architectures.stdout.is_empty() {
            return Err(cli::Error::PushArchitectures { output: architectures }.into());
        }
        for architecture in String::from_utf8_lossy(&architectures.stdout).split_whitespace() {
            let signed = Command::new("codesign")
                .args(["--display", "--entitlements", "-", "--xml", "--arch", architecture])
                .arg(bundle)
                .output()
                .map_err(|source| crate::cli::Error::CommandSpawn { program: "codesign".into(), source })?;
            if !signed.status.success() {
                return Err(cli::Error::PushEntitlementsRead {
                    architecture: architecture.into(), output: signed,
                }.into());
            }
            let entitlements = Value::from_reader(Cursor::new(signed.stdout))
                .map_err(cli::Error::PushEntitlementsParse)?;
            self.validate(&profile, &entitlements)?;
        }
        Ok(())
    }

    fn validate(&self, profile: &Value, signed: &Value) -> Result<()> {
        let profile = profile.as_dictionary().ok_or(cli::Error::PushProfileNotDictionary)?;
        let allowed = profile.get("Entitlements").and_then(Value::as_dictionary)
            .ok_or(cli::Error::PushProfileEntitlementsMissing)?;
        let signed = signed.as_dictionary().ok_or(cli::Error::PushEntitlementsNotDictionary)?;
        let environment = if self.sandbox { "development" } else { "production" };
        let key = self.platform.entitlement();
        if signed.get(key).and_then(Value::as_string) != Some(environment)
            || allowed.get(key).and_then(Value::as_string) != Some(environment)
        {
            return Err(cli::Error::PushEnvironmentMismatch {
                key, environment, topic: self.topic.clone(),
            }.into());
        }
        let teams = profile.get("TeamIdentifier").and_then(Value::as_array);
        if !teams.is_some_and(|teams| teams.iter().any(|team| team.as_string() == Some(&self.team)))
            || signed.get("com.apple.developer.team-identifier").and_then(Value::as_string)
                != Some(&self.team)
        {
            return Err(cli::Error::PushTeamMismatch { team: self.team.clone() }.into());
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
            return Err(cli::Error::PushProfileTopicMismatch { topic: self.topic.clone() }.into());
        }
        let expires = profile.get("ExpirationDate").and_then(Value::as_date).map(SystemTime::from);
        if expires.is_none_or(|expires| expires <= SystemTime::now()) {
            return Err(cli::Error::PushProfileExpired.into());
        }
        Ok(())
    }
}
