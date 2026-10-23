use serde_json::json;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::apple::AscApiKey;
use crate::apple::XcodeSigning;
use crate::{PistonError, Result};
use super::application::IOSApplication;

pub mod cli;

pub struct IOSStore<'a> {
    application: IOSApplication,
    key: &'a AscApiKey,
    developer: PathBuf,
    archive: Option<IOSArchive>,
}

impl<'a> IOSStore<'a> {
    pub fn new(mut application: IOSApplication, key: &'a AscApiKey, developer: PathBuf, build_number: String) -> Result<Self> {
        application.store_release()?;
        application.remember_build(build_number)?;
        Ok(Self { application, key, developer, archive: None })
    }

    pub fn archive(&mut self, signing: &XcodeSigning, bundle: &Path) -> Result<()> {
        self.archive = None;
        let developer = self.developer.clone();
        let name = bundle.file_stem().unwrap().to_str().unwrap();
        let work = self.application.work().join("store");
        let project = signing.project(
            &bundle,
            &work,
            self.application.entitlements().as_deref(),
            "Release",
        )?;
        let path = work.join(format!("{name}.xcarchive"));
        if path.exists() {
            fs::remove_dir_all(&path)
                .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        }
        let archive = IOSArchive { path, work, developer, team: signing.team().into() };
        let mut command = self.command(&archive);
        command
            .arg("-project")
            .arg(&project)
            .args(["-scheme", "Piston", "-configuration", "Release"])
            .args(["-sdk", "iphoneos", "-destination", "generic/platform=iOS"])
            .arg("-archivePath")
            .arg(&archive.path)
            .arg("-derivedDataPath")
            .arg(archive.work.join("DerivedData"))
            .args(["clean", "archive"]);
        Self::run(command, &archive.work.join("archive.log"), "Archiving the App Store app")?;
        if !archive.path.join("Info.plist").is_file()
            || !archive.path.join("Products/Applications").join(format!("{name}.app")).is_dir()
        {
            return Err(PistonError::CodesignError(format!(
                "Xcode did not produce an application archive at {}. See {}.",
                archive.path.display(),
                archive.work.join("archive.log").display(),
            )));
        }
        self.archive = Some(archive);
        Ok(())
    }

    pub fn export(&self) -> Result<PathBuf> {
        let archive = self.archive.as_ref().ok_or_else(|| {
            PistonError::Generic("Create the App Store archive before exporting it.".into())
        })?;
        let name = archive.path.file_stem().unwrap().to_str().unwrap();
        let output = self.export_archive(archive, "export")?;
        let ipa = output.join(format!("{name}.ipa"));
        if !ipa.is_file() {
            return Err(PistonError::CodesignError(format!(
                "Xcode did not export {}. See {}.",
                ipa.display(),
                archive.work.join("export.log").display(),
            )));
        }
        self.application.verify_store_notifications(&ipa)?;
        Ok(ipa)
    }

    pub fn upload(&self) -> Result<()> {
        let archive = self.archive.as_ref().ok_or_else(|| {
            PistonError::Generic(
                "Build the App Store archive successfully before uploading it.".into(),
            )
        })?;
        self.export_archive(archive, "upload")?;
        Ok(())
    }
}

struct IOSArchive {
    path: PathBuf,
    work: PathBuf,
    developer: PathBuf,
    team: String,
}

impl IOSStore<'_> {
    fn command(&self, archive: &IOSArchive) -> Command {
        let mut command = IOSApplication::command(&archive.developer, "xcodebuild");
        command
            .arg("-allowProvisioningUpdates")
            .arg("-authenticationKeyPath")
            .arg(self.key.path())
            .args([
                "-authenticationKeyID",
                self.key.key_id(),
                "-authenticationKeyIssuerID",
                self.key.issuer_id(),
            ]);
        command
    }

    fn export_archive(&self, archive: &IOSArchive, destination: &str) -> Result<PathBuf> {
        let options = json!({
            "method": "app-store-connect", "destination": destination,
            "signingStyle": "automatic", "teamID": archive.team,
            "manageAppVersionAndBuildNumber": false, "uploadSymbols": true,
        });
        let options_path = archive.work.join(format!("{destination}-options.plist"));
        fs::write(&options_path, serde_json::to_vec_pretty(&options).unwrap())
            .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        let output = Command::new("plutil")
            .args(["-convert", "xml1"])
            .arg(&options_path)
            .output()
            .map_err(|error| PistonError::PlutilConvertError(error.to_string()))?;
        IOSApplication::check(output, "Preparing App Store export options")?;
        let output = archive.work.join(destination);
        if output.exists() {
            fs::remove_dir_all(&output)
                .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        }
        let mut command = self.command(archive);
        command
            .arg("-exportArchive")
            .arg("-archivePath")
            .arg(&archive.path)
            .arg("-exportPath")
            .arg(&output)
            .arg("-exportOptionsPlist")
            .arg(&options_path);
        Self::run(
            command,
            &archive.work.join(format!("{destination}.log")),
            &format!("App Store {destination}"),
        )?;
        Ok(output)
    }

    fn run(mut command: Command, log_path: &Path, operation: &str) -> Result<()> {
        let log = File::create(log_path)
            .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        let stderr =
            log.try_clone().map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        let status =
            command.stdout(Stdio::from(log)).stderr(Stdio::from(stderr)).status().map_err(
                |error| {
                    PistonError::CodesignError(format!(
                        "{operation} could not run: {error}. Log: {}",
                        log_path.display(),
                    ))
                },
            )?;
        if !status.success() {
            let log = fs::read_to_string(log_path).unwrap_or_default();
            let lower = log.to_lowercase();
            let guidance = if lower.contains("redundant binary upload")
                || lower.contains("previously used build number")
                || lower.contains("bundle version must be higher")
                || lower.contains("bundle version has already been used")
                || lower.contains("already uploaded a build with build number")
            {
                "Apple already has this build number. Rerun deployment to select a new build number \
                 automatically. If TestFlight resumes a pending upload you want to replace, use \
                 cargo piston deploy testflight --new-build."
            } else if lower.contains("agreement")
                && (lower.contains("accept")
                    || lower.contains("updated")
                    || lower.contains("required")
                    || lower.contains("expired"))
            {
                "Ask the team's Account Holder to accept the current agreements at \
                 https://developer.apple.com/account, then rerun deployment."
            } else if lower.contains("401")
                || lower.contains("unauthorized")
                || lower.contains("invalid api key")
                || lower.contains("invalid authentication")
                || lower.contains("unable to authenticate")
                || lower.contains("unable to load authentication key")
            {
                "Verify ASC_KEY_PATH, ASC_KEY_ID and ASC_ISSUER_ID against the active team API key \
                 at https://appstoreconnect.apple.com/access/integrations/api, then rerun deployment."
            } else if lower.contains("sdk version issue")
                || lower.contains("unsupported sdk")
                || lower.contains("unsupported toolchain")
                || lower.contains("must be built with")
                || lower.contains("sdk or later")
            {
                "Update the full Xcode app to the version Apple requires. Select that Xcode \
                 (including DEVELOPER_DIR if set), then rebuild and deploy."
            } else if lower.contains("team has no devices")
                || lower.contains("no devices from which to generate")
            {
                "Xcode needs an eligible development profile for the archive step. Connect and \
                 register a development iPhone with cargo piston dev once, then retry with the \
                 same team, or select a team with an existing eligible profile. App Store export \
                 uses distribution signing."
            } else if lower.contains("push notifications") || lower.contains("aps-environment") {
                "Enable Push Notifications for this bundle ID on the opened Apple Identifiers page. \
                 Use a team with Push Notifications access and an APNs key for the same team, topic, \
                 and production environment. Rerun to regenerate the signing profile."
            } else if lower.contains("no profiles for")
                || lower.contains("provisioning profile")
                || lower.contains("signing certificate")
                || lower.contains("no account for team")
                || lower.contains("could not find team")
                || lower.contains("not a member")
                || lower.contains("permission")
                || lower.contains("not authorized")
            {
                "Ensure --team identifies the paid Apple Developer Program team associated with \
                 the API key. Ask an Admin to verify the key's role and access to certificates, \
                 identifiers, profiles and cloud-managed distribution signing."
            } else {
                "Resolve the Xcode error below, then rerun deployment. The full log includes \
                 Apple's underlying diagnostic."
            };
            let details = log
                .lines()
                .filter(|line| {
                    let line = line.to_lowercase();
                    line.contains("error:")
                        || line.contains("error itms")
                        || line.contains("error domain=")
                })
                .take(6)
                .map(|line| line.chars().take(500).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            let error = PistonError::CodesignError(format!(
                "{operation} failed ({status}). {guidance}\n\n{details}\n\nFull Xcode log: {}",
                log_path.display(),
            ));
            return Err(
                if lower.contains("push notifications") || lower.contains("aps-environment") {
                    error.setup(crate::error::SetupPage::Url(
                        "https://developer.apple.com/account/resources/identifiers/list".into(),
                    ))
                } else {
                    error
                },
            );
        }
        Ok(())
    }
}
