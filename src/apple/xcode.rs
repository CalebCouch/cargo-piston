use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::PistonError;

pub mod cli;

pub struct XcodeSigning {
    team: String,
    developer: PathBuf,
}

impl XcodeSigning {
    pub fn new(team: &str, developer: &Path) -> Self {
        Self { team: team.trim().into(), developer: developer.to_path_buf() }
    }
    pub fn teams() -> Result<Vec<(String, String)>, PistonError> {
        let output = Command::new("defaults")
            .args(["read", "com.apple.dt.Xcode", "IDEProvisioningTeamByIdentifier"])
            .output()
            .map_err(|e| PistonError::CodesignError(format!("Cannot read Xcode teams: {e}")))?;
        if !output.status.success() {
            let reason = String::from_utf8_lossy(&output.stderr);
            if reason.to_lowercase().contains("does not exist") { return Ok(Vec::new()); }
            return Err(PistonError::CodesignError(format!(
                "Could not read Xcode's saved signing teams.\n{}",
                reason.trim(),
            )));
        }

        let accounts = Self::plist_json(&output.stdout)?;
        let mut teams: Vec<(String, String)> = accounts
            .as_object()
            .into_iter()
            .flat_map(|accounts| accounts.values())
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(|team| {
                let id = team.get("teamID")?.as_str()?;
                let name = team.get("teamName").and_then(Value::as_str).unwrap_or("Unnamed team");
                Some((id.to_string(), name.to_string()))
            })
            .collect();
        teams.sort_by(|a, b| a.0.cmp(&b.0));
        teams.dedup_by(|a, b| a.0 == b.0);
        Ok(teams)
    }

    pub fn team(&self) -> &str { &self.team }

    pub fn sign_macos(&self, bundle: &Path, work: &Path, entitlements: &Path, release: bool, external: bool) -> Result<PathBuf, PistonError> {
        let configuration = if release { "Release" } else { "Debug" };
        let project = self.project(bundle, work, Some(entitlements), configuration)?;
        let name = bundle.file_stem().unwrap().to_str().unwrap();
        let archive = work.join(format!("{name}.xcarchive"));
        if release && archive.exists() {
            fs::remove_dir_all(&archive)
                .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        }
        let mut command = Command::new("xcrun");
        command
            .env("DEVELOPER_DIR", &self.developer)
            .arg("xcodebuild")
            .arg("-project")
            .arg(&project)
            .args(["-scheme", "Piston", "-configuration", configuration])
            .args([
                "-sdk",
                "macosx",
                "-destination",
                if release { "generic/platform=macOS" } else { "platform=macOS" },
            ])
            .arg("-derivedDataPath")
            .arg(work.join("DerivedData"))
            .args(["-allowProvisioningUpdates", "-allowProvisioningDeviceRegistration"]);
        if release {
            command.arg("-archivePath").arg(&archive).args(["clean", "archive"]);
        } else {
            command.args(["clean", "build"]);
        }
        self.run(command, &work.join("xcodebuild.log"), "Signing the macOS notification app")?;
        if !release {
            return Ok(work.join("DerivedData/Build/Products/Debug").join(format!("{name}.app")));
        }

        let options = work.join("ExportOptions.plist");
        let settings = json!({
            "method": if external { "developer-id" } else { "app-store-connect" },
            "destination": "export", "teamID": self.team, "signingStyle": "automatic",
            "manageAppVersionAndBuildNumber": false,
        });
        fs::write(&options, serde_json::to_vec(&settings).unwrap())
            .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        let converted = Command::new("plutil")
            .args(["-convert", "xml1"])
            .arg(&options)
            .output()
            .map_err(|error| PistonError::CodesignError(error.to_string()))?;
        if !converted.status.success() {
            return Err(PistonError::CodesignError("Cannot create macOS export options.".into()));
        }
        let exported = work.join("export");
        if exported.exists() {
            fs::remove_dir_all(&exported)
                .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        }
        let mut command = Command::new("xcrun");
        command
            .env("DEVELOPER_DIR", &self.developer)
            .args(["xcodebuild", "-exportArchive"])
            .arg("-archivePath")
            .arg(&archive)
            .arg("-exportPath")
            .arg(&exported)
            .arg("-exportOptionsPlist")
            .arg(&options)
            .arg("-allowProvisioningUpdates");
        self.run(command, &work.join("export.log"), "Exporting the macOS notification app")?;
        let app = exported.join(format!("{name}.app"));
        if app.is_dir() { return Ok(app); }

        let packages = fs::read_dir(&exported)
            .map_err(|error| PistonError::CodesignError(error.to_string()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|error| PistonError::CodesignError(error.to_string()))?;
        let packages: Vec<_> = packages
            .into_iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "pkg"))
            .collect();
        let [package] = packages.as_slice() else {
            return Err(PistonError::CodesignError(format!(
                "Xcode produced no app or installer in {}.",
                exported.display()
            )));
        };
        let expanded = work.join("expanded-package");
        if expanded.exists() {
            fs::remove_dir_all(&expanded)
                .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        }
        let mut command = Command::new("pkgutil");
        command.arg("--expand-full").arg(package).arg(&expanded);
        self.run(command, &work.join("package.log"), "Checking the macOS installer")?;
        let mut directories = vec![expanded];
        let mut apps = Vec::new();
        while let Some(directory) = directories.pop() {
            for entry in fs::read_dir(directory)
                .map_err(|error| PistonError::CodesignError(error.to_string()))?
            {
                let entry = entry.map_err(|error| PistonError::CodesignError(error.to_string()))?;
                if !entry
                    .file_type()
                    .map_err(|error| PistonError::CodesignError(error.to_string()))?
                    .is_dir()
                { continue; }
                if entry.file_name() == std::ffi::OsStr::new(&format!("{name}.app")) {
                    apps.push(entry.path());
                } else {
                    directories.push(entry.path());
                }
            }
        }
        if apps.len() != 1 {
            return Err(PistonError::CodesignError(
                "The exported macOS installer must contain one matching app.".into(),
            ));
        }
        Ok(apps.remove(0))
    }

    pub fn project(&self, bundle: &Path, work: &Path, entitlements: Option<&Path>, configuration: &str) -> Result<PathBuf, PistonError> {
        let bundle = fs::canonicalize(bundle).map_err(|e| {
            PistonError::CodesignError(format!("Cannot open app bundle {}: {e}", bundle.display()))
        })?;
        let macos = bundle.join("Contents/Info.plist").is_file();
        let plist_path = bundle.join(if macos { "Contents/Info.plist" } else { "Info.plist" });
        let plist = Self::plist_json(&fs::read(&plist_path).map_err(|e| {
            PistonError::CodesignError(format!("Cannot read {}: {e}", plist_path.display()))
        })?)?;
        let bundle_id = Self::plist_string(&plist, "CFBundleIdentifier")?;
        let executable = Self::plist_string(&plist, "CFBundleExecutable")?;
        let minimum_os = if macos {
            plist.get("LSMinimumSystemVersion").and_then(Value::as_str).unwrap_or("11.0")
        } else {
            Self::plist_string(&plist, "MinimumOSVersion")?
        };
        let name = bundle.file_stem().and_then(|name| name.to_str()).ok_or_else(|| {
            PistonError::CodesignError("The app bundle needs a UTF-8 filename.".into())
        })?;
        let executable_path = if macos {
            bundle.join("Contents/MacOS").join(executable)
        } else {
            bundle.join(executable)
        };
        if !executable_path.is_file() {
            return Err(PistonError::CodesignError(format!(
                "The app executable is missing: {}",
                executable_path.display(),
            )));
        }

        fs::create_dir_all(&work)
            .map_err(|e| PistonError::CreateDirAllError { path: work.to_path_buf(), source: e })?;
        let work = fs::canonicalize(&work).map_err(|e| {
            PistonError::CodesignError(format!("Cannot resolve {}: {e}", work.display()))
        })?;
        let project = work.join("Piston.xcodeproj");
        let schemes = project.join("xcshareddata/xcschemes");
        fs::create_dir_all(&schemes)
            .map_err(|e| PistonError::CreateDirAllError { path: schemes.clone(), source: e })?;

        let mut settings = json!({
            "ALWAYS_SEARCH_USER_PATHS": "NO",
            "ARCHS": "arm64",
            "CODE_SIGN_IDENTITY": "Apple Development",
            "CODE_SIGN_STYLE": "Automatic",
            "COPY_PHASE_STRIP": "NO",
            "DEVELOPMENT_TEAM": self.team,
            "ENABLE_DEBUG_DYLIB": "NO",
            "ENABLE_USER_SCRIPT_SANDBOXING": "NO",
            "EXECUTABLE_NAME": executable,
            "GENERATE_INFOPLIST_FILE": "NO",
            "INFOPLIST_FILE": plist_path,
            "IPHONEOS_DEPLOYMENT_TARGET": minimum_os,
            "PISTON_INPUT_BUNDLE": bundle,
            "PRODUCT_BUNDLE_IDENTIFIER": bundle_id,
            "PRODUCT_NAME": name,
            "PROVISIONING_PROFILE_SPECIFIER": "",
            "SDKROOT": "iphoneos", "SKIP_INSTALL": "NO", "INSTALL_PATH": "$(LOCAL_APPS_DIR)",
            "STRIP_INSTALLED_PRODUCT": "NO",
            "SUPPORTED_PLATFORMS": "iphoneos",
            "TARGETED_DEVICE_FAMILY": plist.get("UIDeviceFamily").and_then(Value::as_array)
                .map(|families| families.iter().map(Value::to_string).collect::<Vec<_>>().join(","))
                .unwrap_or_else(|| "1,2".into())
        });
        if macos {
            let architectures = Command::new("/usr/bin/lipo")
                .arg("-archs")
                .arg(&executable_path)
                .output()
                .map_err(|error| PistonError::CodesignError(error.to_string()))?;
            if !architectures.status.success() {
                return Err(PistonError::CodesignError(
                    "Cannot read the macOS executable's architectures.".into(),
                ));
            }
            settings["ARCHS"] = json!(String::from_utf8_lossy(&architectures.stdout).trim());
            settings["SDKROOT"] = json!("macosx");
            settings["SUPPORTED_PLATFORMS"] = json!("macosx");
            settings["MACOSX_DEPLOYMENT_TARGET"] = json!(minimum_os);
            settings["ENABLE_HARDENED_RUNTIME"] = json!("YES");
            settings.as_object_mut().unwrap().remove("IPHONEOS_DEPLOYMENT_TARGET");
            settings.as_object_mut().unwrap().remove("TARGETED_DEVICE_FAMILY");
        }
        if let Some(entitlements) = entitlements {
            settings["CODE_SIGN_ENTITLEMENTS"] =
                json!(fs::canonicalize(entitlements).map_err(|e| {
                    PistonError::CodesignError(format!(
                        "Cannot read entitlements {}: {e}",
                        entitlements.display(),
                    ))
                })?);
        }

        let push_enabled = entitlements
            .map(plist::Value::from_file)
            .transpose()
            .map_err(|error| PistonError::CodesignError(format!("Invalid entitlements: {error}")))?
            .and_then(|value| {
                value
                    .as_dictionary()
                    .and_then(|value| {
                        value.get(if macos {
                            "com.apple.developer.aps-environment"
                        } else {
                            "aps-environment"
                        })
                    })
                    .cloned()
            })
            .is_some();
        let prefix = if macos { "/Contents" } else { "" };
        let profile = if macos { "embedded.provisionprofile" } else { "embedded.mobileprovision" };
        let copy_script = format!(
            "set -eu\n/usr/bin/rsync -a --delete --exclude {prefix}/Info.plist \
             --exclude {prefix}/PkgInfo --exclude {prefix}/_CodeSignature --exclude {prefix}/{profile} \
             \"${{PISTON_INPUT_BUNDLE}}/\" \"${{TARGET_BUILD_DIR}}/${{FULL_PRODUCT_NAME}}/\"\n\
             /bin/chmod +x \"${{TARGET_BUILD_DIR}}/${{EXECUTABLE_PATH}}\"\n"
        );
        let project_data = json!({
            "archiveVersion": "1", "objectVersion": "56", "classes": {},
            "rootObject": "A00000000000000000000001",
            "objects": {
                "A00000000000000000000001": {
                    "isa": "PBXProject", "compatibilityVersion": "Xcode 14.0",
                    "buildConfigurationList": "A00000000000000000000010",
                    "developmentRegion": "en", "knownRegions": ["en", "Base"],
                    "mainGroup": "A00000000000000000000002",
                    "productRefGroup": "A00000000000000000000002",
                    "projectDirPath": "", "projectRoot": "",
                    "targets": ["A00000000000000000000003"],
                    "attributes": { "TargetAttributes": { "A00000000000000000000003": {
                        "DevelopmentTeam": self.team, "ProvisioningStyle": "Automatic",
                        "SystemCapabilities": if push_enabled {
                            json!({"com.apple.Push": {"enabled": 1}})
                        } else { json!({}) }
                    } } }
                },
                "A00000000000000000000002": {
                    "isa": "PBXGroup", "sourceTree": "<group>",
                    "children": ["A00000000000000000000004"]
                },
                "A00000000000000000000003": {
                    "isa": "PBXNativeTarget", "name": "Piston", "productName": name,
                    "productType": "com.apple.product-type.application",
                    "productReference": "A00000000000000000000004",
                    "buildConfigurationList": "A00000000000000000000011",
                    "buildPhases": ["A00000000000000000000005"],
                    "buildRules": [], "dependencies": []
                },
                "A00000000000000000000004": {
                    "isa": "PBXFileReference", "explicitFileType": "wrapper.application",
                    "path": format!("{name}.app"), "sourceTree": "BUILT_PRODUCTS_DIR"
                },
                "A00000000000000000000005": {
                    "isa": "PBXShellScriptBuildPhase", "name": "Copy Rust application",
                    "buildActionMask": "2147483647", "alwaysOutOfDate": "1",
                    "files": [], "runOnlyForDeploymentPostprocessing": "0",
                    "inputPaths": [executable_path],
                    "outputPaths": ["$(TARGET_BUILD_DIR)/$(EXECUTABLE_PATH)"],
                    "shellPath": "/bin/sh",
                    "shellScript": copy_script
                },
                "A00000000000000000000010": {
                    "isa": "XCConfigurationList", "defaultConfigurationName": configuration,
                    "defaultConfigurationIsVisible": "0",
                    "buildConfigurations": ["A00000000000000000000012"]
                },
                "A00000000000000000000011": {
                    "isa": "XCConfigurationList", "defaultConfigurationName": configuration,
                    "defaultConfigurationIsVisible": "0",
                    "buildConfigurations": ["A00000000000000000000013"]
                },
                "A00000000000000000000012": {
                    "isa": "XCBuildConfiguration", "name": configuration, "buildSettings": {}
                },
                "A00000000000000000000013": {
                    "isa": "XCBuildConfiguration", "name": configuration, "buildSettings": settings
                }
            }
        });
        let project_path = project.join("project.pbxproj");
        fs::write(&project_path, serde_json::to_vec_pretty(&project_data).unwrap())
            .map_err(|e| PistonError::WriteFileError(format!("{}: {e}", project_path.display())))?;
        let output =
            Command::new("plutil").args(["-convert", "xml1"]).arg(&project_path).output().map_err(
                |e| PistonError::CodesignError(format!("Cannot generate Xcode project: {e}")),
            )?;
        if !output.status.success() {
            return Err(PistonError::CodesignError(format!(
                "Cannot generate Xcode project: {}",
                String::from_utf8_lossy(&output.stderr),
            )));
        }

        let scheme_name = name
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;");
        fs::write(schemes.join("Piston.xcscheme"), format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="1630" version="1.3">
  <BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES">
    <BuildActionEntries>
      <BuildActionEntry buildForRunning="YES" buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES" buildForTesting="NO">
        <BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="A00000000000000000000003" BuildableName="{scheme_name}.app" BlueprintName="Piston" ReferencedContainer="container:Piston.xcodeproj"/>
      </BuildActionEntry>
    </BuildActionEntries>
  </BuildAction>
  <LaunchAction buildConfiguration="{configuration}"/>
  <ArchiveAction buildConfiguration="{configuration}" revealArchiveInOrganizer="NO"/>
</Scheme>
"#)).map_err(|e| PistonError::WriteFileError(format!("Cannot write Xcode scheme: {e}")))?;

        Ok(project)
    }

    pub fn sign(&self, bundle: &Path, work: &Path, udid: &str, entitlements: Option<&Path>) -> Result<PathBuf, PistonError> {
        let project = self.project(bundle, &work.join("signing"), entitlements, "Debug")?;
        let work = project.parent().unwrap();
        let name = bundle.file_stem().unwrap().to_str().unwrap();
        let log_path = work.join("xcodebuild.log");
        let log = File::create(&log_path)
            .map_err(|e| PistonError::WriteFileError(format!("{}: {e}", log_path.display())))?;
        let stderr = log.try_clone().map_err(|e| {
            PistonError::WriteFileError(format!("Cannot capture Xcode output: {e}"))
        })?;
        let derived_data = work.join("DerivedData");
        let status = Command::new("xcrun")
            .env("DEVELOPER_DIR", &self.developer)
            .arg("xcodebuild")
            .arg("-project")
            .arg(&project)
            .args(["-scheme", "Piston", "-configuration", "Debug"])
            .args(["-sdk", "iphoneos", "-destination", &format!("platform=iOS,id={udid}")])
            .arg("-derivedDataPath")
            .arg(&derived_data)
            // The copy phase replaces signed code that Xcode otherwise considers up to date.
            .args([
                "-allowProvisioningUpdates",
                "-allowProvisioningDeviceRegistration",
                "clean",
                "build",
            ])
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr))
            .status()
            .map_err(|e| PistonError::CodesignError(format!("Cannot run xcodebuild: {e}")))?;
        if !status.success() {
            let log = fs::read_to_string(&log_path).unwrap_or_default();
            let (error, needs_account) = self.build_error(&log, &log_path);
            if needs_account {
                return Err(
                    error.setup(crate::error::SetupPage::XcodeAccounts(self.developer.clone()))
                );
            }
            return Err(error);
        }

        let signed_bundle =
            derived_data.join("Build/Products/Debug-iphoneos").join(format!("{name}.app"));
        if !signed_bundle.join("embedded.mobileprovision").is_file() {
            return Err(PistonError::CodesignError(format!(
                "Xcode did not produce a provisioned app at {}. See {}.",
                signed_bundle.display(),
                log_path.display(),
            )));
        }
        let verification = Command::new("codesign")
            .args(["--verify", "--strict"])
            .arg(&signed_bundle)
            .output()
            .map_err(|error| PistonError::CodesignError(error.to_string()))?;
        if !verification.status.success() {
            return Err(PistonError::CodesignError(format!(
                "Invalid development signature: {}\nXcode log: {}",
                String::from_utf8_lossy(&verification.stderr).trim(),
                log_path.display(),
            )));
        }
        Ok(signed_bundle)
    }
}

impl XcodeSigning {
    fn run(&self, mut command: Command, log_path: &Path, operation: &str) -> Result<(), PistonError> {
        let log = File::create(log_path)
            .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        let stderr =
            log.try_clone().map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        let status = command
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr))
            .status()
            .map_err(|error| PistonError::CodesignError(format!("{operation}: {error}")))?;
        if !status.success() {
            let log = fs::read_to_string(log_path).unwrap_or_default();
            let (error, needs_account) = self.build_error(&log, log_path);
            if needs_account {
                return Err(
                    error.setup(crate::error::SetupPage::XcodeAccounts(self.developer.clone()))
                );
            }
            return Err(error);
        }
        Ok(())
    }

    fn build_error(&self, log: &str, log_path: &Path) -> (PistonError, bool) {
        let lower = log.to_lowercase();
        let errors =
            lower.lines().filter(|line| line.contains("error:")).collect::<Vec<_>>().join("\n");
        let guidance = if errors.contains("unable to log in with account")
            || (errors.contains("the login details for account")
                && errors.contains("were rejected"))
            || errors.contains("your session has expired. please log in")
            || errors.contains("authentication credentials are missing or invalid")
        {
            Some((
                true,
                "Xcode could not sign in to your Apple Account.",
                "Open Xcode > Settings > Accounts, select the Apple Account for the team below, \
                 and sign in again. Complete any password or verification prompts.",
            ))
        } else if errors.contains("there are no accounts registered with xcode")
            || errors.contains("no accounts: add a new account in accounts settings")
        {
            Some((
                true,
                "Xcode has no Apple Account available for signing.",
                "Open Xcode > Settings > Accounts, add your Apple Account, and complete sign-in. \
                 Confirm that the team below is listed for that account.",
            ))
        } else if lower.contains("program license agreement")
            && (lower.contains("must accept")
                || lower.contains("must be accepted")
                || lower.contains("agreement has been updated")
                || lower.contains("agreement needs to be updated")
                || lower.contains("accept the latest program license agreement"))
        {
            Some((
                false,
                "Apple requires an updated developer agreement.",
                "Sign in at https://developer.apple.com/account and review the agreement shown \
                 there. If the team belongs to an organization, ask its Account Holder to accept it.",
            ))
        } else if lower.contains("no account for team")
            || lower.contains("could not find team")
            || lower.contains("not a member of the specified team")
            || lower.contains("not a member of team")
            || (lower.contains("does not have permission to create")
                && lower.contains("provisioning profile"))
        {
            Some((
                false,
                "Xcode cannot use the selected development team.",
                "Open Xcode > Settings > Accounts and confirm that your account belongs to the \
                 team below and has permission to sign apps. Select an available team with \
                 --team <TEAM_ID>, or ask that team's administrator to grant the required access.",
            ))
        } else if lower
            .contains("cannot be registered to your development team because it is not available")
            || lower.contains("bundle identifier is already in use")
            || (lower.contains("an app id with identifier") && lower.contains("is not available"))
        {
            Some((
                false,
                "Apple cannot register this bundle identifier for the selected team.",
                "Choose a unique development identifier with --bundle-id <unique.identifier>, \
                 or select the team that owns the existing identifier with --team <TEAM_ID>.",
            ))
        } else if lower.contains("maximum app id limit has been reached") {
            Some((
                false,
                "The Apple team has reached its App ID registration limit.",
                "Reuse an app identifier already registered to this team, wait until Apple's \
                 registration limit resets, or select another eligible team with --team <TEAM_ID>.",
            ))
        } else if lower
            .contains("maximum number of apps for free development profiles has been reached")
        {
            Some((
                false,
                "The iPhone has reached its limit for apps signed with free development profiles.",
                "Choose an unused development app to remove from the iPhone, or use a team whose \
                 membership permits this installation. Keep the current bundle identifier when \
                 updating an app already installed.",
            ))
        } else if lower.contains("device limit has been reached")
            || (lower.contains("reached the maximum number of") && lower.contains("devices"))
            || (lower.contains("maximum number of registered devices") && lower.contains("reached"))
        {
            Some((
                false,
                "The Apple team has reached its device registration limit.",
                "Ask the team's Account Holder to review its device registration limit at \
                 https://developer.apple.com/account, or select another eligible team with \
                 --team <TEAM_ID>. Re-pairing the phone does not reset this limit.",
            ))
        } else if lower.contains("keychain is locked")
            || lower.contains("user interaction is not allowed")
            || lower.contains("errsecinteractionnotallowed")
        {
            Some((
                false,
                "Code signing could not access the signing key in Keychain.",
                "Open Keychain Access and unlock the keychain containing your development \
                 certificate. When macOS asks whether codesign may use its private key, allow it.",
            ))
        } else if lower.contains("no signing certificate")
            || lower.lines().any(|line| {
                line.contains("no ")
                    && line.contains("signing certificate matching team id")
                    && line.contains("with a private key was found")
            })
        {
            Some((
                false,
                "Xcode could not find a usable development signing certificate and private key.",
                "Open Xcode > Settings > Accounts, select your account and team, and open Manage \
                 Certificates. Create an Apple Development certificate if needed, or import your \
                 existing certificate together with its private key.",
            ))
        } else if lower.contains("push notifications") || lower.contains("aps-environment") {
            Some((
                false,
                "Apple could not provision Push Notifications for this app.",
                "Use an Apple Developer Program team with Push Notifications access. On the opened \
                 Identifiers page, select this app's bundle ID and enable Push Notifications. \
                 Confirm that the APNs key belongs to the same team and permits this topic and \
                 environment, then rerun so Xcode can regenerate the provisioning profile.",
            ))
        } else if (lower.contains("no profiles for ") && lower.contains("were found"))
            || lower
                .contains("couldn't find any ios app development provisioning profiles matching")
            || lower.contains("no provisioning profiles found")
        {
            Some((
                false,
                "Xcode could not obtain a development provisioning profile for this app.",
                "Open Xcode > Settings > Accounts and confirm the team below is available. \
                 Verify that the bundle identifier belongs to that team and that it can register \
                 this device and the app's requested capabilities. Piston already requests \
                 automatic provisioning; no distribution profile is needed.",
            ))
        } else {
            None
        };
        if let Some((needs_account, cause, action)) = guidance {
            let error = PistonError::CodesignError(format!(
                "{cause}\n\n1. {action}\n\
                 2. Rerun your original Piston command with any corrected options. \
                 Cargo will reuse its existing build cache.\n\n\
                 Selected team: {}\nXcode log: {}",
                self.team,
                log_path.display(),
            ));
            let error = if lower.contains("push notifications") || lower.contains("aps-environment")
            {
                error.setup(crate::error::SetupPage::Url(
                    "https://developer.apple.com/account/resources/identifiers/list".into(),
                ))
            } else {
                error
            };
            return (error, needs_account);
        }

        let mut details = Vec::new();
        for line in log.lines().rev().map(str::trim).filter(|line| !line.is_empty()) {
            let lower = line.to_lowercase();
            if lower.contains("error:")
                || lower.contains("error domain=")
                || lower.contains("failed with")
                || lower.contains("errsec")
            {
                let mut line: String = line.chars().take(500).collect();
                if lower.chars().count() > 500 { line.push('…'); }
                if !details.contains(&line) { details.push(line); }
                if details.len() == 5 { break; }
            }
        }
        details.reverse();
        let details = if details.is_empty() {
            "Xcode did not report a recognizable cause in its output.".into()
        } else {
            details.join("\n")
        };
        (
            PistonError::CodesignError(format!(
                "Xcode failed while preparing the development app.\n\n{details}\n\n\
             1. Open the Xcode log below and address the reported Xcode error.\n\
             2. Rerun your dev command. Cargo will reuse its existing build cache.\n\n\
             Xcode log: {}",
                log_path.display(),
            )),
            false,
        )
    }

    fn plist_json(data: &[u8]) -> Result<Value, PistonError> {
        let mut command = Command::new("plutil")
            .args(["-convert", "json", "-o", "-", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PistonError::CodesignError(format!("Cannot read property list: {e}")))?;
        command
            .stdin
            .take()
            .unwrap()
            .write_all(data)
            .map_err(|e| PistonError::CodesignError(format!("Cannot read property list: {e}")))?;
        let output = command
            .wait_with_output()
            .map_err(|e| PistonError::CodesignError(format!("Cannot read property list: {e}")))?;
        if !output.status.success() {
            return Err(PistonError::CodesignError(format!(
                "Invalid property list: {}",
                String::from_utf8_lossy(&output.stderr),
            )));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|e| PistonError::CodesignError(format!("Invalid property list: {e}")))
    }

    fn plist_string<'a>(plist: &'a Value, key: &str) -> Result<&'a str, PistonError> {
        plist.get(key).and_then(Value::as_str).filter(|value| !value.is_empty()).ok_or_else(|| {
            PistonError::CodesignError(format!("The app Info.plist needs a nonempty {key} string."))
        })
    }
}
