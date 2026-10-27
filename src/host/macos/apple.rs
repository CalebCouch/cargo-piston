use serde_json::Value;
use std::env;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use crate::host::Host;
use crate::project::Project;
use crate::{PistonError, Result};

pub mod cli;
pub use cli::Cli;

pub struct Xcode {
    system: [u32; 3],
    architecture: String,
    home: PathBuf,
}

impl Xcode {
    pub fn new() -> Result<Self> {
        let output = Project::output(Command::new("sw_vers").arg("-productVersion"))?;
        let value = String::from_utf8_lossy(&output);
        let system = Self::version(value.trim()).ok_or_else(|| {
            PistonError::XcodeSystemVersionInvalid { value: value.into_owned() }
        })?;
        let silicon =
            Command::new("sysctl").args(["-n", "hw.optional.arm64"]).output().is_ok_and(|output| {
                output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "1"
            });
        let architecture = if silicon {
            "arm64".to_string()
        } else {
            let output = Command::new("uname")
                .arg("-m")
                .output()
                .map_err(|source| PistonError::CommandSpawn { program: "uname".into(), source })?;
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };
        if !matches!(architecture.as_str(), "arm64" | "x86_64") {
            return Err(PistonError::XcodeArchitectureUnsupported { architecture });
        }
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or(PistonError::MissingEnvironment { name: "HOME" })?;
        Ok(Self { system, architecture, home })
    }

    fn selected(&self, minimum: u32, report: &mut dyn FnMut(PistonError)) -> Result<Option<PathBuf>> {
        let developer = if let Some(path) = env::var_os("DEVELOPER_DIR") {
            let (version, path) = self.inspect(&PathBuf::from(path))?;
            if version[0] < minimum {
                return Err(PistonError::XcodeSelectedTooOld { version, minimum });
            }
            path
        } else {
            return self.installed(minimum, report);
        };
        Ok(Some(developer))
    }

    fn verify_sdk(&self, developer: &Path, sdk: &str, minimum: u32) -> Result<()> {
        let output = Command::new("xcrun")
            .env("DEVELOPER_DIR", &developer)
            .args(["--sdk", sdk, "--show-sdk-version"])
            .output()
            .map_err(|source| PistonError::CommandSpawn { program: "xcrun".into(), source })?;
        let version = String::from_utf8_lossy(&output.stdout);
        if !output.status.success()
            || Self::version(version.trim()).is_none_or(|version| version[0] < minimum)
        {
            return Err(PistonError::XcodeSdkUnavailable {
                developer: developer.into(), sdk: sdk.into(), minimum, output,
            });
        }
        Ok(())
    }
}

struct Release {
    version: [u32; 3],
    name: String,
    build: String,
    minimum: [u32; 3],
    url: String,
    native: bool,
}

impl Xcode {
    fn version(value: &str) -> Option<[u32; 3]> {
        let mut result = [0; 3];
        for (index, part) in value.split('.').enumerate() {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) { return None; }
            *result.get_mut(index)? = part.parse().ok()?;
        }
        Some(result)
    }

    fn plist(path: &Path) -> Result<Value> {
        let output = Project::output(Command::new("plutil")
            .args(["-convert", "json", "-o", "-"]).arg(path))?;
        serde_json::from_slice(&output)
            .map_err(|error| PistonError::IntoJSONError(error.to_string()))
    }

    fn inspect(&self, path: &Path) -> Result<([u32; 3], PathBuf)> {
        let path = if path.join("Contents/Developer").is_dir() {
            path.join("Contents/Developer")
        } else {
            path.to_path_buf()
        };
        let developer = fs::canonicalize(&path)
            .map_err(|source| PistonError::ReadFile { path, source })?;
        if !developer.join("usr/bin/xcodebuild").is_file() {
            return Err(PistonError::XcodeIncomplete { developer });
        }
        let contents = developer.parent().unwrap();
        let info = Self::plist(&contents.join("Info.plist"))?;
        let version = info["CFBundleShortVersionString"].as_str().and_then(Self::version);
        let required = info["LSMinimumSystemVersionByArchitecture"][&self.architecture]
            .as_str()
            .or_else(|| info["LSMinimumSystemVersion"].as_str())
            .and_then(Self::version);
        if info["CFBundleIdentifier"] != "com.apple.dt.Xcode"
            || version.is_none()
            || required.is_none_or(|required| required > self.system)
        {
            return Err(PistonError::XcodeIncompatible { developer });
        }
        Ok((version.unwrap(), developer))
    }

    fn installed(&self, minimum: u32, report: &mut dyn FnMut(PistonError)) -> Result<Option<PathBuf>> {
        let mut paths = Vec::new();
        let selected = Command::new("xcode-select")
            .arg("-p")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()));
        if let Some(path) = &selected { paths.push(path.clone()); }
        for root in [PathBuf::from("/Applications"), self.home.join("Applications")] {
            match fs::read_dir(&root) {
                Ok(entries) => {
                    for entry in entries {
                        let entry = entry
                            .map_err(|source| PistonError::MapDirError { path: root.clone(), source })?;
                        let path = entry.path();
                        if path.join("Contents/Developer/usr/bin/xcodebuild").is_file() {
                            paths.push(path);
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(source) => report(PistonError::ReadDirError { path: root, source }),
            }
        }
        let spotlight = Command::new("mdfind")
            .args(["-0", "kMDItemCFBundleIdentifier == 'com.apple.dt.Xcode'"])
            .output();
        match spotlight {
            Ok(output) if output.status.success() => {
                paths.extend(
                    output
                        .stdout
                        .split(|byte| *byte == 0)
                        .filter(|path| !path.is_empty())
                        .map(|path| PathBuf::from(std::ffi::OsStr::from_bytes(path))),
                );
            }
            Ok(output) => report(PistonError::CommandFailed { program: "mdfind".into(), output }),
            Err(source) => report(PistonError::CommandSpawn { program: "mdfind".into(), source }),
        }
        let mut candidates = Vec::new();
        for path in paths {
            match self.inspect(&path) {
                Ok(candidate) if candidate.0[0] >= minimum => candidates.push(candidate),
                Ok(_) => {}
                Err(error) => report(PistonError::XcodeDiscovery { path, error: Box::new(error) }),
            }
        }
        candidates.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| {
                    (Some(&right.1) == selected.as_ref()).cmp(&(Some(&left.1) == selected.as_ref()))
                })
                .then_with(|| left.1.cmp(&right.1))
        });
        candidates.dedup();
        for (_, developer) in candidates {
            let output = Command::new(developer.join("usr/bin/xcodebuild"))
                .env("DEVELOPER_DIR", &developer)
                .arg("-version")
                .output();
            match output {
                Ok(output) if output.status.success() => return Ok(Some(developer)),
                Ok(output) => {
                    let error = String::from_utf8_lossy(&output.stderr);
                    if error.to_lowercase().contains("license") { return Ok(Some(developer)); }
                    report(PistonError::XcodeDiscovery {
                        path: developer.clone(),
                        error: Box::new(PistonError::CommandFailed {
                            program: developer.join("usr/bin/xcodebuild").into(), output,
                        }),
                    });
                }
                Err(error) => {
                    report(PistonError::XcodeDiscovery {
                        path: developer.clone(),
                        error: Box::new(PistonError::CommandSpawn {
                            program: developer.join("usr/bin/xcodebuild").into(), source: error,
                        }),
                    })
                }
            }
        }
        Ok(None)
    }

    fn latest(&self, sdk: &str, minimum: u32) -> Result<Release> {
        let releases: Vec<Value> = ureq::AgentBuilder::new().timeout(Duration::from_secs(15))
            .build().get("https://xcodereleases.com/data.json").call()
            .map_err(|source| PistonError::XcodeCatalogUnavailable { source: Box::new(source) })?
            .into_json().map_err(|error| PistonError::IntoJSONError(error.to_string()))?;
        let mut candidates = Vec::new();
        for release in releases {
            if release.pointer("/version/release/release").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let Some(name) = release.pointer("/version/number").and_then(Value::as_str) else {
                continue;
            };
            let Some(version) = Self::version(name) else { continue; };
            if version[0] < minimum { continue; }
            let Some(required) = release["requires"].as_str().and_then(Self::version) else {
                continue;
            };
            let sdk_name = if sdk == "iphoneos" { "iOS" } else { "macOS" };
            let provides_sdk = release["sdks"][sdk_name].as_array().is_some_and(|sdks| {
                sdks.iter().any(|sdk| {
                    sdk["number"]
                        .as_str()
                        .and_then(Self::version)
                        .is_some_and(|version| version[0] >= minimum)
                })
            });
            if !provides_sdk { continue; }
            let download = &release["links"]["download"];
            let architectures = download["architectures"].as_array();
            if architectures
                .is_some_and(|values| !values.iter().any(|value| value == &self.architecture))
            { continue; }
            let Some(url) = download["url"].as_str() else { continue; };
            if !url.starts_with("https://download.developer.apple.com/Developer_Tools/")
                || !url.ends_with(".xip")
            { continue; }
            let Some(build) = release.pointer("/version/build").and_then(Value::as_str) else {
                continue;
            };
            if build.is_empty() || !build.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
                continue;
            }
            candidates.push(Release {
                version,
                name: name.into(),
                build: build.into(),
                minimum: required,
                url: url.into(),
                native: architectures.is_some_and(|values| values.len() == 1),
            });
        }
        let minimum_os = candidates.iter().map(|release| release.minimum).min();
        candidates.retain(|release| release.minimum <= self.system);
        candidates.sort_by(|left, right| {
            right
                .version
                .cmp(&left.version)
                .then_with(|| right.native.cmp(&left.native))
                .then_with(|| right.build.cmp(&left.build))
        });
        if let Some(release) = candidates.into_iter().next() { return Ok(release); }
        if let Some(required) = minimum_os {
            return Err(PistonError::XcodeSystemTooOld { sdk: sdk.into(), minimum: required }
                .setup(crate::error::SetupPage::Url(
                    "x-apple.systempreferences:com.apple.Software-Update-Settings.extension".into(),
                )));
        }
        Err(PistonError::XcodeReleaseUnavailable {
            architecture: self.architecture.clone(), sdk: sdk.into(),
        })
    }

    fn archive(&self, release: &Release, report: &mut dyn FnMut(PistonError)) -> Result<Option<PathBuf>> {
        if let Some(path) = env::var_os("PISTON_XCODE_ARCHIVE") {
            let path = PathBuf::from(path);
            return fs::canonicalize(&path).map(Some)
                .map_err(|source| PistonError::ReadFile { path, source });
        }
        let filename = release.url.rsplit('/').next().unwrap();
        let mut files = vec![self.home.join("Downloads").join(filename)];
        match Host::find_files(filename) {
            Ok(found) => files.extend(found),
            Err(error) => report(error),
        }
        for path in files {
            if path.is_file()
                && !path.with_extension("xip.part").exists()
                && !path.with_extension("xip.crdownload").exists()
                && path.metadata().is_ok_and(|metadata| metadata.len() > 0)
            {
                return fs::canonicalize(&path)
                    .map(Some)
                    .map_err(|source| PistonError::ReadFile { path, source });
            }
        }
        Ok(None)
    }

    fn install(&self, release: &Release, archive: &Path) -> Result<PathBuf> {
        let applications = self.home.join("Applications");
        fs::create_dir_all(&applications)
            .map_err(|source| PistonError::CreateDirAllError { path: applications.clone(), source })?;
        let destination =
            applications.join(format!("Xcode-{}-{}.app", release.name, release.build));
        if destination.exists() {
            return Err(PistonError::XcodeDestinationExists { path: destination });
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let staging = applications.join(format!(".piston-xcode-{}-{stamp}", std::process::id()));
        fs::create_dir(&staging)
            .map_err(|source| PistonError::CreateDirAllError { path: staging.clone(), source })?;
        Self::run(
            Command::new("xip").arg("--expand").arg(&archive).current_dir(&staging),
            "extract",
            &staging,
        )?;
        let app = staging.join("Xcode.app");
        Self::run(Command::new("codesign").args(["--verify", "--deep", "--strict", "-R",
            "=identifier \"com.apple.dt.Xcode\" and (anchor apple or (anchor apple generic and \
             (certificate leaf[field.1.2.840.113635.100.6.1.9] or \
             certificate leaf[subject.OU] = \"59GAB85EFG\")))"
        ]).arg(&app), "verify", &staging)?;
        let (version, developer) = self.inspect(&app)?;
        let info = Self::plist(&developer.parent().unwrap().join("version.plist"))?;
        if version != release.version || info["ProductBuildVersion"] != release.build {
            return Err(PistonError::XcodeArchiveMismatch {
                version: release.name.clone(), build: release.build.clone(), staging,
            });
        }
        fs::rename(&app, &destination).map_err(|source| PistonError::XcodeInstallMove {
            destination: destination.clone(), staging, source,
        })?;
        Ok(destination.join("Contents/Developer"))
    }

    fn license_accepted(&self, developer: &Path) -> Result<bool> {
        let output = Command::new(developer.join("usr/bin/xcodebuild"))
            .args(["-license", "check"])
            .env("DEVELOPER_DIR", developer)
            .output()
            .map_err(|source| PistonError::CommandSpawn {
                program: developer.join("usr/bin/xcodebuild").into(), source,
            })?;
        Ok(output.status.success())
    }

    fn review_license(&self, developer: &Path) -> Result<()> {
        let status = Command::new("sudo")
            .arg("/usr/bin/env")
            .arg(format!("DEVELOPER_DIR={}", developer.display()))
            .arg(developer.join("usr/bin/xcodebuild"))
            .arg("-license")
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|source| PistonError::CommandSpawn { program: "sudo".into(), source })?;
        if !status.success() || !self.license_accepted(developer)? {
            return Err(PistonError::XcodeLicenseIncomplete);
        }
        Ok(())
    }

    fn components_ready(&self, developer: &Path) -> Result<bool> {
        let output = Command::new(developer.join("usr/bin/xcodebuild"))
            .arg("-checkFirstLaunchStatus")
            .env("DEVELOPER_DIR", developer)
            .output()
            .map_err(|source| PistonError::CommandSpawn {
                program: developer.join("usr/bin/xcodebuild").into(), source,
            })?;
        Ok(output.status.success())
    }

    fn install_components(&self, developer: &Path, elevated: bool) -> Result<()> {
        let logs = self.home.join("Library/Logs/Piston/Xcode");
        fs::create_dir_all(&logs)
            .map_err(|source| PistonError::CreateDirAllError { path: logs.clone(), source })?;
        let binary = developer.join("usr/bin/xcodebuild");
        if elevated {
            let status = Command::new("sudo")
                .arg("-v")
                .status()
                .map_err(|source| PistonError::CommandSpawn { program: "sudo".into(), source })?;
            if !status.success() {
                return Err(PistonError::XcodeAdministratorApprovalIncomplete);
            }
            Self::run(
                Command::new("sudo")
                    .args(["-n", "/usr/bin/env"])
                    .arg(format!("DEVELOPER_DIR={}", developer.display()))
                    .arg(binary)
                    .arg("-runFirstLaunch"),
                "components",
                &logs,
            )
        } else {
            Self::run(
                Command::new(binary).arg("-runFirstLaunch").env("DEVELOPER_DIR", developer),
                "components",
                &logs,
            )
        }
    }

    fn components_need_admin(&self) -> bool {
        let log = fs::read_to_string(self.home.join("Library/Logs/Piston/Xcode/components.log"))
            .unwrap_or_default()
            .to_lowercase();
        log.contains("root") || log.contains("permission") || log.contains("administrator")
    }

    fn run(command: &mut Command, stage: &str, directory: &Path) -> Result<()> {
        let log = directory.join(format!("{stage}.log"));
        let file = fs::File::create(&log)
            .map_err(|source| PistonError::CreateFileError { path: log.clone(), source })?;
        let errors = file.try_clone()
            .map_err(|source| PistonError::ReadFile { path: log.clone(), source })?;
        let program = command.get_program().to_owned();
        let status = command.stdout(Stdio::from(file)).stderr(Stdio::from(errors)).status()
            .map_err(|source| PistonError::CommandSpawn { program: program.clone(), source })?;
        if status.success() { return Ok(()); }
        Err(PistonError::CommandLogged {
            diagnostics: fs::read_to_string(&log).unwrap_or_default(), log,
            error: Box::new(PistonError::CommandStatus { program, status }),
        })
    }
}
