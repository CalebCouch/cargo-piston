use serde_json::json;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::Result;
use crate::cli::Error;
use crate::apple::{AppleSigningTarget, AscApiKey, XcodeSigning};
use super::application::IOSApplication;

pub mod cli;
pub use cli::Cli;

pub struct IOSStore<'a> {
    application: IOSApplication,
    key: &'a AscApiKey,
    key_file: tempfile::NamedTempFile,
    developer: PathBuf,
    archive: Option<IOSArchive>,
}

impl<'a> IOSStore<'a> {
    pub fn new(mut application: IOSApplication, key: &'a AscApiKey, developer: PathBuf, build_number: String) -> Result<Self> {
        application.store_release()?;
        application.remember_build(build_number)?;
        let key_file = key.temporary_key()?;
        Ok(Self { application, key, key_file, developer, archive: None })
    }

    pub fn archive(&mut self, signing: &XcodeSigning, bundle: &Path) -> Result<()> {
        self.archive = None;
        let developer = self.developer.clone();
        let name = bundle.file_stem().unwrap().to_str().unwrap();
        let work = self.application.work().join("store");
        let project = signing.project(
            bundle,
            &work,
            self.application.entitlements().as_deref(),
            "Release",
        )?;
        let path = work.join(format!("{name}.xcarchive"));
        if path.exists() {
            fs::remove_dir_all(&path)
                .map_err(|error| Error::WriteFile(error.to_string()))?;
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
        Self::run(command, &archive.work.join("archive.log"))?;
        if !archive.path.join("Info.plist").is_file()
            || !archive.path.join("Products/Applications").join(format!("{name}.app")).is_dir()
        {
            return Err(cli::Error::ArchiveMissing {
                path: archive.path.clone(), log: archive.work.join("archive.log"),
            }.into());
        }
        self.archive = Some(archive);
        Ok(())
    }

    pub fn export(&self) -> Result<PathBuf> {
        let archive = self.archive.as_ref().unwrap();
        let name = archive.path.file_stem().unwrap().to_str().unwrap();
        let output = self.export_archive(archive, "export")?;
        let ipa = output.join(format!("{name}.ipa"));
        if !ipa.is_file() {
            return Err(cli::Error::ExportMissing {
                path: ipa, log: archive.work.join("export.log"),
            }.into());
        }
        self.verify_ipa(&ipa)?;
        Ok(ipa)
    }

    pub fn upload(&self) -> Result<()> {
        let archive = self.archive.as_ref().unwrap();
        let name = archive.path.file_stem().unwrap().to_str().unwrap();
        self.verify_ipa(&archive.work.join("export").join(format!("{name}.ipa")))?;
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
    fn verify_ipa(&self, ipa: &Path) -> Result<()> {
        let output = tempfile::tempdir_in(self.application.work())?;
        let extracted = Command::new("ditto").args(["-x", "-k"]).arg(ipa).arg(output.path()).output()
            .map_err(|source| Error::CommandSpawn { program: "ditto".into(), source })?;
        IOSApplication::check(extracted, "ditto")?;
        let payload = output.path().join("Payload");
        let apps = fs::read_dir(&payload)
            .map_err(|source| Error::ReadDir { path: payload.clone(), source })?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|source| Error::MapDir { path: payload, source })?;
        let apps: Vec<_> = apps.into_iter()
            .filter(|path| path.extension().is_some_and(|value| value == "app"))
            .collect();
        let [app] = apps.as_slice() else {
            return Err(cli::Error::ExportAppCount { count: apps.len() }.into());
        };
        let archive = self.archive.as_ref().unwrap();
        XcodeSigning::new(&archive.team, &self.developer).verify_distribution(
            app, AppleSigningTarget::Ios, self.application.store_bundle_id()?,
        )?;
        self.application.verify_notifications(app)
    }

    fn command(&self, archive: &IOSArchive) -> Command {
        let mut command = IOSApplication::command(&archive.developer, "xcodebuild");
        command
            .arg("-allowProvisioningUpdates")
            .arg("-authenticationKeyPath")
            .arg(self.key_file.path())
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
            .map_err(|error| Error::WriteFile(error.to_string()))?;
        let output = Command::new("plutil")
            .args(["-convert", "xml1"])
            .arg(&options_path)
            .output()
            .map_err(|error| crate::apple::cli::Error::PlutilConvert(error.to_string()))?;
        IOSApplication::check(output, "plutil")?;
        let output = archive.work.join(destination);
        if output.exists() {
            fs::remove_dir_all(&output)
                .map_err(|error| Error::WriteFile(error.to_string()))?;
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
        Self::run(command, &archive.work.join(format!("{destination}.log")))?;
        Ok(output)
    }

    fn run(mut command: Command, log_path: &Path) -> Result<()> {
        let log = File::create(log_path)
            .map_err(|error| Error::WriteFile(error.to_string()))?;
        let stderr = log.try_clone()
            .map_err(|error| Error::WriteFile(error.to_string()))?;
        let status = command.stdout(Stdio::from(log)).stderr(Stdio::from(stderr)).status()
            .map_err(|source| Error::CommandLogged {
                log: log_path.to_path_buf(), diagnostics: String::new(),
                error: Box::new(Error::CommandSpawn { program: "xcodebuild".into(), source }),
            })?;
        if status.success() { return Ok(()); }
        let output = fs::read_to_string(log_path).unwrap_or_default();
        let lower = output.to_lowercase();
        let push = lower.contains("push notifications") || lower.contains("aps-environment");
        let error = cli::Error::DistributionFailed { log: log_path.to_path_buf(), output, status };
        Err(if push {
            Error::SetupRequired {
                error: error.into(),
                page: crate::cli::SetupPage::Url(
                    "https://developer.apple.com/account/resources/identifiers/list".into(),
                ),
            }.into()
        } else { error.into() })
    }
}
