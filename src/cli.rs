use cargo_subcommand::Subcommand;
use clap::Parser;
use std::env;
use std::fmt;
use std::path::PathBuf;
use crate::{android, dev, developer, devices, host, ios, linux, macos, windows};
use crate::host::Host;
use crate::project::Project;
use crate::{PistonError, Result};

#[derive(Clone, Debug, Parser)]
pub struct ApplicationArgs {
    #[command(flatten)]
    pub cargo: cargo_subcommand::Args,
    /// Apple signing team. Saved in the selected platform's Cargo metadata.
    #[arg(long)]
    pub team: Option<String>,
    /// App identifier. Development generates a default; store deployment requires one.
    #[arg(long)]
    pub bundle_id: Option<String>,
    /// Additional app settings, including privacy descriptions, from an Info.plist.
    #[arg(long)]
    pub info_plist: Option<PathBuf>,
    /// Entitlements to use when signing the development app.
    #[arg(long)]
    pub entitlements: Option<PathBuf>,
    /// Require the existing Cargo.lock.
    #[arg(long)]
    pub locked: bool,
    /// Build Cargo dependencies without accessing the network.
    #[arg(long)]
    pub offline: bool,
}

#[derive(Parser)]
#[command(name = "piston")]
pub struct Cli {
    #[clap(subcommand)]
    piston: PistonCmd,
}

impl Cli {
    pub fn run() {
        if let Err(error) = Self::start() {
            eprintln!("\nerror: {error}");
            if let PistonError::SetupRequired { page, .. } = &error {
                match page {
                    crate::error::SetupPage::Url(url) => {
                        eprintln!("\n{}", host::Cli::link("Continue setup", url));
                    }
                    crate::error::SetupPage::XcodeAccounts(developer) => {
                        eprintln!("\n{}", host::Cli::link(
                            "Xcode Settings > Accounts", "xcpref://Xcode.PreferencePane.Accounts",
                        ));
                        eprintln!("Selected Xcode: {}", developer.display());
                    }
                }
            }
            std::process::exit(1);
        }
    }

    pub fn start() -> Result<()> {
        let cwd = env::current_dir()
            .map_err(|_| PistonError::Generic("Error getting working directory".into()))?;
        let Self { piston: PistonCmd::Piston { cmd } } = Self::parse();
        let Some(cmd) = cmd else { return developer::Cli::start(false); };
        match &cmd {
            PistonSubCmd::Setup => return developer::Cli::start(true),
            PistonSubCmd::Backup => return developer::Cli::backup(),
            PistonSubCmd::Restore { secret } => return developer::Cli::restore(secret),
            _ => (),
        }

        if let PistonSubCmd::Dev(args) = cmd { return dev::Cli::start(args); }
        if let PistonSubCmd::Deploy(args) = cmd { return ios::deployment::Cli::start(args); }
        if let PistonSubCmd::Version = cmd {
            println!("{}, {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        let env_vars = match Project::load_env_file() {
            Ok(values) => values,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(error) => return Err(PistonError::Generic(format!("Error loading .env: {error}"))),
        };

        match cmd {
            PistonSubCmd::Dev(_) | PistonSubCmd::Deploy(_) | PistonSubCmd::Version
            | PistonSubCmd::Setup | PistonSubCmd::Backup | PistonSubCmd::Restore { .. } => {
                unreachable!()
            }
            PistonSubCmd::Build(args) => {
                if args.dry_run {
                    println!("Dry run: would build with {:?}", args.common);
                    return Ok(());
                }
                let manifest = Project::manifest(&args.common.subcommand_args)?;
                developer::Cli::credentials(&manifest, None, false)?;
                Host::check_build()?;
                let cmd = Subcommand::new(args.common.subcommand_args).map_err(|error| {
                    PistonError::Generic(format!("Error parsing subcommand: {error}"))
                })?;
                let target = cmd.target();
                let platform = Platform::from_target(target.unwrap_or(Host::default_target()));
                let release = cmd.args().release;
                let external = args.common.external;
                let appimage = args.common.appimage;
                let release_override = release || external || appimage;
                let target_string = Project::build_target(target)?;
                let profile = if release_override { "release" } else { "debug" };
                println!("Building for {target_string} ({profile})");
                match platform {
                    Platform::Android => {
                        android::Cli::build(release_override, target_string, cwd, env_vars)?;
                    }
                    Platform::Ios => {
                        ios::Cli::build(release_override, target_string, cwd, env_vars, None)?;
                    }
                    Platform::Linux => {
                        linux::Cli::build(release_override, appimage, target_string, cwd, env_vars)?;
                    }
                    Platform::Windows => {
                        windows::Cli::build(release_override, target_string, cwd, env_vars)?;
                    }
                    Platform::Macos => {
                        macos::Cli::build(release_override, external, target_string, cwd, env_vars)?;
                    }
                    Platform::Unknown => {
                        return Err(PistonError::Generic(format!(
                            "Unknown or unsupported target: {target:?}"
                        )));
                    }
                }
            }
            PistonSubCmd::Run(args) => {
                let manifest = Project::manifest(&args.common.subcommand_args)?;
                developer::Cli::credentials(&manifest, None, false)?;
                let cmd = Subcommand::new(args.common.subcommand_args).map_err(|error| {
                    PistonError::Generic(format!("Error parsing subcommand :{error}"))
                })?;
                let release = cmd.args().release;
                if let Some(device) = args.device {
                    let target = device.trim();
                    println!("Selecting device: {device}");
                    let devices = devices::Cli::list(env_vars.clone(), true)?;
                    let android = devices.android.iter().find(|device| device.id == target);
                    let ios = devices.ios.iter().find(|device| device.id == target);
                    if target == "ios" && !devices.ios.is_empty() {
                        let device = &devices.ios[0];
                        println!("Using {} ({})", device.model, device.id);
                        ios::Cli::run(release, cwd, env_vars, device)?;
                    } else if target == "android" && !devices.android.is_empty() {
                        let device = &devices.android[0];
                        println!("Using Android device {}", device.id);
                        android::Cli::run(release, cwd, env_vars, device)?;
                    } else if let Some(device) = android {
                        println!("Using Android device {}", device.id);
                        android::Cli::run(release, cwd, env_vars, device)?;
                    } else if let Some(device) = ios {
                        println!("Using {} ({})", device.model, device.id);
                        ios::Cli::run(release, cwd, env_vars, device)?;
                    } else {
                        return Err(PistonError::Generic("Device not found".into()));
                    }
                } else {
                    println!("Running on this computer");
                    host::Cli::run_local(release, cwd, env_vars)?;
                }
            }
            PistonSubCmd::ListDevices => {
                println!("list all available connected devices and relevant information");
                devices::Cli::list(env_vars, false)?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for PistonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LinuxVmHomeAccess { path, source } => write!(f,
                "Cannot use Linux VM storage directory {}: {source}", path.display()),
            Self::LinuxVmHomeInvalid { path } => write!(f,
                "Saved Linux VM storage directory {} is unavailable. Mount its drive or choose an existing directory.",
                path.display()),
            Self::DeveloperStorageLock { path, source } => write!(f,
                "Cannot lock developer identities at {}: {source}. Finish the other Piston \
                 identity or deployment operation and retry.", path.display()),
            Self::DeveloperStoragePath { path } => write!(f,
                "Developer identity storage must use ordinary private files and directories: {}",
                path.display()),
            Self::DeveloperConnection(source) => write!(f, "Cannot connect to AIR: {source}"),
            Self::DeveloperContract(source) => write!(f, "Cannot open the developer contract: {source}"),
            Self::DeveloperTimeout { team } => write!(f,
                "AIR did not confirm developer credentials for team {team} within 90 seconds. \
                 Check your connection and retry; Piston will read the existing contract first."),
            Self::DeveloperDisconnected { team } => {
                write!(f, "AIR disconnected while synchronizing developer credentials for team {team}.")
            }
            Self::DeveloperRejected { team } => write!(f,
                "AIR rejected a developer contract record for team {team}. Verify the restored \
                 team identity before retrying."),
            Self::DeveloperOwnerMismatch { team } => write!(f,
                "The developer contract for team {team} does not match the local identity. \
                 Restore the team's saved cargo piston backup."),
            Self::DeveloperUnconfirmed { team, account } => write!(f,
                "AIR did not retain the requested {account} credentials for team {team}. \
                 Another session may have changed them; reload the contract and retry."),
            Self::IosArchiveMissing { path, log } => write!(f,
                "Xcode did not produce an application archive at {}. See {}.",
                path.display(), log.display()),
            Self::IosExportMissing { path, log } => write!(f,
                "Xcode did not export {}. See {}.", path.display(), log.display()),
            Self::IosDistributionFailed { log, output, status } =>
                ios::distribution::Cli::failure(f, log, output, status),
            Self::SigningExportMissing { path } => {
                write!(f, "Xcode produced no app or installer in {}.", path.display())
            }
            Self::SigningExportAppCount { count } => write!(
                f, "The exported macOS installer must contain one matching app; found {count}.",
            ),
            Self::SigningBundleName { path } => {
                write!(f, "The app bundle needs a UTF-8 filename: {}", path.display())
            }
            Self::SigningExecutableMissing { path } => {
                write!(f, "The app executable is missing: {}", path.display())
            }
            Self::SigningProvisionedAppMissing { bundle, log } => write!(
                f, "Xcode did not produce a provisioned app at {}. See {}.",
                bundle.display(), log.display(),
            ),
            Self::SigningSignatureInvalid { output, log } => write!(
                f, "Invalid development signature: {}\nXcode log: {}",
                String::from_utf8_lossy(&output.stderr).trim(), log.display(),
            ),
            Self::SigningAccountRejected { team, log }
            | Self::SigningAccountMissing { team, log }
            | Self::SigningAgreementRequired { team, log }
            | Self::SigningTeamUnavailable { team, log }
            | Self::SigningBundleUnavailable { team, log }
            | Self::SigningAppIdLimit { team, log }
            | Self::SigningFreeAppLimit { team, log }
            | Self::SigningDeviceLimit { team, log }
            | Self::SigningKeychainLocked { team, log }
            | Self::SigningCertificateUnavailable { team, log }
            | Self::SigningPushUnavailable { team, log }
            | Self::SigningProfileUnavailable { team, log } => {
                let (cause, action) = match self {
                    Self::SigningAccountRejected { .. } => (
                        "Xcode could not sign in to your Apple Account.",
                        "Open Xcode > Settings > Accounts, select the Apple Account for the team \
                         below, and sign in again. Complete password or verification prompts.",
                    ),
                    Self::SigningAccountMissing { .. } => (
                        "Xcode has no Apple Account available for signing.",
                        "Open Xcode > Settings > Accounts, add your Apple Account, and complete \
                         sign-in. Confirm that the team below is listed for that account.",
                    ),
                    Self::SigningAgreementRequired { .. } => (
                        "Apple requires an updated developer agreement.",
                        "Open your Apple Developer account and review the agreement shown there.\n   \
                         For an organization, ask its Account Holder to accept it.",
                    ),
                    Self::SigningTeamUnavailable { .. } => (
                        "Xcode cannot use the selected development team.",
                        "Open Xcode > Settings > Accounts and confirm your account belongs to the \
                         team below and can sign apps. Select an available team with --team \
                         <TEAM_ID>, or ask that team's administrator to grant access.",
                    ),
                    Self::SigningBundleUnavailable { .. } => (
                        "Apple cannot register this bundle identifier for the selected team.",
                        "Choose a unique development identifier with --bundle-id \
                         <unique.identifier>, or select the team that owns the existing \
                         identifier with --team <TEAM_ID>.",
                    ),
                    Self::SigningAppIdLimit { .. } => (
                        "The Apple team has reached its App ID registration limit.",
                        "Reuse an app identifier registered to this team, wait until Apple's \
                         registration limit resets, or select another eligible team with \
                         --team <TEAM_ID>.",
                    ),
                    Self::SigningFreeAppLimit { .. } => (
                        "The iPhone has reached its limit for apps signed with free development \
                         profiles.",
                        "Choose an unused development app to remove from the iPhone, or use a \
                         team whose membership permits this installation. Keep the current \
                         bundle identifier when updating an app already installed.",
                    ),
                    Self::SigningDeviceLimit { .. } => (
                        "The Apple team has reached its device registration limit.",
                        "Ask the Account Holder to review the device limit in Apple Developer.\n   \
                         Or select another eligible team with --team <TEAM_ID>.\n   \
                         Re-pairing the phone does not reset the limit.",
                    ),
                    Self::SigningKeychainLocked { .. } => (
                        "Code signing could not access the signing key in Keychain.",
                        "Open Keychain Access and unlock the keychain containing your \
                         development certificate. When macOS asks whether codesign may use \
                         its private key, allow it.",
                    ),
                    Self::SigningCertificateUnavailable { .. } => (
                        "Xcode could not find a usable development signing certificate and \
                         private key.",
                        "Open Xcode > Settings > Accounts, select your account and team, and \
                         open Manage Certificates. Create an Apple Development certificate if \
                         needed, or import your existing certificate with its private key.",
                    ),
                    Self::SigningPushUnavailable { .. } => (
                        "Apple could not provision Push Notifications for this app.",
                        "Use an Apple Developer Program team with Push Notifications access. \
                         Open the Identifiers link below, select this app's bundle ID and enable \
                         Push Notifications. Confirm the APNs key belongs to the same team \
                         and permits this topic and environment, then rerun so Xcode can \
                         regenerate the provisioning profile.",
                    ),
                    Self::SigningProfileUnavailable { .. } => (
                        "Xcode could not obtain a development provisioning profile for this app.",
                        "Open Xcode > Settings > Accounts and confirm the team below is \
                         available. Verify that the bundle identifier belongs to that team \
                         and that it can register this device and the app's capabilities. \
                         Piston requests automatic provisioning; no distribution profile is needed.",
                    ),
                    _ => unreachable!(),
                };
                write!(f, "{cause}\n\nApple team: {team}\n\n\
                    1. {action}\n\n2. Rerun your Piston command with any corrected options.\n\n\
                    Xcode log: {}", log.display())?;
                if matches!(self, Self::SigningAgreementRequired { .. } | Self::SigningDeviceLimit { .. }) {
                    write!(f, "\n\n{}", host::Cli::link(
                        "Apple Developer account", "https://developer.apple.com/account",
                    ))?;
                }
                Ok(())
            }
            Self::SigningBuildFailed { log, details } => {
                writeln!(f, "Xcode failed while preparing the development app.\n")?;
                if details.is_empty() {
                    writeln!(f, "Xcode did not report a recognizable cause in its output.")?;
                } else {
                    writeln!(f, "{}", details.join("\n"))?;
                }
                write!(
                    f, "\n1. Open the Xcode log below and address the reported Xcode error.\n\
                     2. Rerun your dev command. Cargo will reuse its existing build cache.\n\n\
                     Xcode log: {}", log.display(),
                )
            }
            Self::IosDevicesAmbiguous { devices } => {
                let devices = devices.iter().map(|(name, id)| format!("{name} ({id})"))
                    .collect::<Vec<_>>().join(", ");
                write!(f, "Multiple devices match: {devices}. Select one with --device <name-or-UDID>.")
            }
            Self::IosDeviceUnavailable { name } => write!(f, "{name} is not connected."),
            Self::IosDeviceDisappeared { name } => write!(f,
                "{name} disconnected during setup. Reconnect and unlock it, then rerun."),
            Self::IosDeviceLocked { name } => write!(f,
                "{name} is locked. Unlock it and keep the screen awake, then rerun \
                cargo piston dev ios in an interactive terminal to retry launch when needed."),
            Self::IosLockStateMissing => write!(f,
                "Xcode did not report whether the device is locked. Unlock it and retry."),
            Self::IosDeveloperModeDisabled { name, version } => {
                write!(f, "Developer Mode is disabled on {name}")?;
                if let Some(version) = version { write!(f, " (iOS {version})")?; }
                write!(f, ". Pairing succeeded.\n\n\
                    1. Reopen Settings on the phone and scroll to the bottom of Privacy & Security.\n\
                    2. Turn on Developer Mode and accept the restart.\n\
                    3. Unlock the phone after restart and confirm Turn On.\n\
                    4. Rerun the same cargo piston dev command.\n\n\
                    If the setting is missing, reopen Settings after pairing. If it is still missing, \
                    select the phone in Xcode's device manager and let Xcode prepare it.\n\
                    Apple requires this confirmation on the phone.")
            }
            Self::IosConnectionNotReady { name } => write!(f,
                "{name} is attached, but Apple's development connection is not ready. \
                Keep it unlocked, accept Trust This Computer, and enable \
                Settings > Privacy & Security > Developer Mode, then rerun."),
            Self::IosDeviceListMissing => write!(f, "devicectl returned no device list."),
            Self::IosSimulatorNotFound { selector } => write!(f,
                "No connected iPhone or available Simulator matches {selector}. \
                Use --simulator without --device to create one."),
            Self::IosSimulatorRuntimesMissing => write!(f, "Simulator returned no runtimes."),
            Self::IosSimulatorRuntimeUnavailable => write!(f,
                "Xcode installed no usable iOS runtime. Open Xcode Settings > Components."),
            Self::IosSimulatorDeviceTypeMissing =>
                write!(f, "Xcode returned no iPhone device types."),
            Self::IosConfigInvalid(source) =>
                write!(f, "Invalid package.metadata.piston.ios: {source}"),
            Self::IosMinimumVersion { value } => write!(f,
                "minimum_os {value:?} must be an iOS version of 17.0 or newer for CoreDevice development."),
            Self::IosStoreIdentifierMissing => write!(f,
                "App Store deployment requires --bundle-id or package.metadata.piston.ios.bundle_id."),
            Self::IosStoreTargetInvalid => write!(f,
                "App Store builds require --target aarch64-apple-ios and the release profile."),
            Self::IosSimulatorTarget { target } =>
                write!(f, "{target} cannot run in the iOS Simulator."),
            Self::IosTargetUnsupported { target } =>
                write!(f, "Unsupported iOS device/simulator target: {target}"),
            Self::IosStoreIconMissing => write!(f,
                "Add icon.png, icon.jpg or icon.jpeg beside the selected package Cargo.toml \
                for App Store deployment."),
            Self::IosStoreIconInvalid { path } => write!(f,
                "App Store icon {} must be square, at least 1024 by 1024 pixels, and fully opaque.",
                path.display()),
            Self::IosResourceInvalid { path } =>
                write!(f, "Invalid resource path: {}", path.display()),
            Self::IosResourceConflict { path } =>
                write!(f, "Resource replaces an app file: {}", path.display()),
            Self::IosImageAssetCreate { path, source } => write!(f,
                "Cannot create image asset {}; resource image names must be unique: {source}",
                path.display()),
            Self::AppSelectionInvalid => write!(f,
                "Select one app package; Android uses its cdylib, desktop and iOS use its binary."),
            Self::AppPackageMissing => write!(f,
                "Select an application package with --package when running from a workspace."),
            Self::AppLibraryMissing => write!(f,
                "Android needs an app library with crate-type = [\"cdylib\"] and an android_main \
                entry point in Maverick."),
            Self::AppBinaryMissing => write!(f,
                "Choose an application executable with --bin (or set package.default-run)."),
            Self::RustHostMissing => write!(f, "rustc did not report its host target."),
            Self::HostArchitectureUnsupported { target } => write!(f,
                "Cannot select a target for host {target}. Specify an explicit --target."),
            Self::FileMetadata { path, source } =>
                write!(f, "Cannot read metadata for {}: {source}", path.display()),
            Self::SaveImage { path, source } =>
                write!(f, "Cannot save image {}: {source}", path.display()),
            Self::ManifestTableRequired { path, section } => write!(f,
                "Expected a table for package.metadata.piston.{section} in {}.", path.display()),
            Self::ManifestStringRequired { path, key } => write!(f,
                "package.metadata.piston.{key} in {} must be a string.", path.display()),
            Self::ManifestReadOnly { path } => write!(f, "{} is read-only.", path.display()),
            Self::ManifestChanged { path } => write!(f,
                "{} changed during setup; rerun to save settings.", path.display()),
            Self::ManifestPackageMissing { path } => write!(f,
                "{} has no application package. Select a package with --package.", path.display()),
            Self::ManifestParse { path, source } =>
                write!(f, "Invalid {}: {source}", path.display()),
            Self::ManifestWrite { path, source } =>
                write!(f, "Cannot save Piston settings in {}: {source}", path.display()),
            Self::StoreTemporaryKey(source) => write!(f,
                "Cannot prepare the temporary App Store Connect key for Xcode: {source}"),
            Self::StoreCredentialsInvalid(source) => write!(f,
                "Invalid App Store Connect credentials in the developer contract: {source}. \
                 Rerun deployment with --setup to replace them."),
            Self::StoreKeyInvalid { key_id, source } => write!(f,
                "Invalid App Store Connect private key {key_id}: {source}\n\n\
                 Rerun deployment with --setup and import the matching Team API key.\n\n{}",
                host::Cli::link("Team API keys", "https://appstoreconnect.apple.com/access/integrations/api")),
            Self::StoreKeyIdInvalid => write!(f,
                "Provide a 10-character Team API Key ID or an AuthKey_<KEY_ID>.p8 file path."),
            Self::StoreIssuerInvalid => write!(f,
                "Enter the Issuer ID shown above Team Keys (ASC_ISSUER_ID), in UUID format."),
            Self::StoreKeyMissing { key_id } => write!(f,
                "Cannot find AuthKey_{key_id}.p8 using system search or the standard key folders. \
                 Set ASC_KEY_PATH or use the interactive setup."),
            Self::StoreKeyAmbiguous { key_id, paths } => {
                writeln!(f, "Multiple files match Key ID {key_id}. Set ASC_KEY_PATH to choose one:")?;
                for path in paths { writeln!(f, "{}", path.display())?; }
                Ok(())
            }
            Self::StoreKeyUnreadable { path, source } => write!(f,
                "Cannot read the API key at {}: {source}. Choose the downloaded .p8 file.",
                path.display()),
            Self::StoreKeySearchFallback(source) => write!(f,
                "{source} Checking standard key folders directly."),
            Self::StoreKeySearchDenied { path } => write!(f,
                "Cannot search {}: permission denied. Supply the key file path if stored there.",
                path.display()),
            Self::StoreResponseField { field } => write!(f, "Apple's response is missing {field}."),
            Self::StoreReleaseExists { requested, released } => write!(f,
                "Cannot upload iOS version {requested}: App Store Connect already released {released}. \
                 Choose a higher package.version for a new public release. An unreleased version can \
                 be reused with a new build number; a released version cannot."),
            Self::StoreBuildNumberOverflow => write!(f,
                "The iOS build number is too large to increment."),
            Self::StoreResponseCollection { path } => write!(f,
                "Apple returned an invalid collection for {path}."),
            Self::StorePaginationUrl { url } => write!(f,
                "Apple returned an unexpected pagination URL: {url}"),
            Self::StorePaginationLimit => write!(f,
                "Apple's response exceeded 100 pages; narrow the query."),
            Self::StoreResponseJson { method, path, source } => write!(f,
                "Apple returned invalid JSON for {method} {path}: {source}"),
            Self::StoreRequestFailed { method, path, status, response } => {
                write!(f, "App Store Connect {method} {path} failed (HTTP {status}).")?;
                for error in response["errors"].as_array().into_iter().flatten() {
                    write!(f, " {}: {}", error["code"].as_str().unwrap_or("API_ERROR"),
                        error["detail"].as_str().or(error["title"].as_str())
                            .unwrap_or("No detail provided"))?;
                }
                let help = match status {
                    401 => "Rerun deployment with --setup to replace the API credentials. \
                        Also check your Mac's clock.",
                    403 => "Use an API key with Admin or App Manager access to this app \
                        and confirm the team's agreements are current.",
                    409 => "Check the reported resource and its current state \
                        in App Store Connect, then rerun.",
                    429 => "Apple is rate limiting requests. Wait and rerun the command.",
                    _ => "Rerun after resolving this App Store Connect error.",
                };
                write!(f, "\n{help}")
            }
            Self::StoreConnectionFailure { method, path, source } => write!(f,
                "Could not reach App Store Connect for {method} {path}: {source}. Check your network \
                 and rerun. Apple may have received the request; \
                 check its status before retrying an upload."),
            Self::StoreHttpError { method, path, source } => {
                write!(f, "App Store Connect {method} {path} failed: {source}")?;
                if path == "/v1/bundleIds" && matches!(source, ureq::Error::Status(409, _)) {
                    write!(f, ". Choose a new, unique bundle ID in Cargo.toml.")?;
                }
                Ok(())
            }
            Self::StoreHttpStatus { method, path, status, response } => write!(f,
                "App Store Connect {method} {path} failed (HTTP {status}): {response}"),
            Self::StoreCertificateMissing { name } => write!(f,
                "Could not find a Distribution certificate matching '{name}'."),
            Self::StoreVersionInvalid { version } => write!(f,
                "Cannot compare App Store version {version:?}; \
                 expected one to three numeric components."),
            Self::StoreAppMissing { bundle_id, testflight } => {
                let command = if *testflight {
                    "cargo piston deploy testflight"
                } else {
                    "cargo piston deploy"
                };
                write!(f, "No App Store Connect app is accessible for {bundle_id}.\n\n\
                    1. Open Apps and choose + > New App:\n   {}\n\n\
                    2. Select this bundle ID and enter the name, primary language, and SKU.\n\
                    3. Rerun {command}.\n\n\
                    Already created the app? Check the API key's team and access to it.",
                    host::Cli::link("App Store Connect apps", "https://appstoreconnect.apple.com/apps"))
            }
            Self::StoreScreenshotFolder { path, source } => write!(f,
                "Cannot open screenshot folder {}: {source}\n\
                 Add 1–10 PNG or JPEG screenshots in this folder.",
                path.display()),
            Self::StoreScreenshotNotFile { path, directory } => write!(f,
                "Screenshot {} is not a regular file. Put PNG or JPEG files directly in {} \
                 without subfolders or symbolic links.", path.display(), directory.display()),
            Self::StoreScreenshotFormat { path } => write!(f,
                "Unsupported screenshot {}. Export it as PNG or JPEG (.png, .jpg, or .jpeg); \
                 changing the filename extension is not enough.", path.display()),
            Self::StoreScreenshotCount { path, count } => write!(f,
                "Screenshot folder {} contains {count} images. Provide 1–10 screenshots.",
                path.display()),
            Self::StoreScreenshotSizesMissing { family } => write!(f,
                "Apple's screenshot catalog contains no accepted dimensions for {family}. \
                 Refresh the catalog before preparing screenshots."),
            Self::StoreScreenshotOutputOverlap { path } => write!(f,
                "Screenshot output {} overlaps the original screenshot folders. \
                 Choose an output folder outside store/apple/iphone and store/apple/ipad.",
                path.display()),
            Self::StoreScreenshotDecode { path, source } => write!(f,
                "Cannot decode screenshot {}: {source}\nExport a new PNG or JPEG copy.", path.display()),
            Self::StoreScreenshotOrientation { path, source } => write!(f,
                "Cannot read screenshot orientation for {}: {source}", path.display()),
            Self::StoreScreenshotColorProfile { path, source } => write!(f,
                "Cannot read or preserve screenshot color profile for {}: {source}", path.display()),
            Self::StoreScreenshotTransparent { path } => write!(f,
                "Screenshot {} contains transparent pixels. Apple requires opaque screenshots. \
                 Flatten it against your intended background and export a new PNG or JPEG copy.",
                path.display()),
            Self::StoreScreenshotDimensions { path, original, accepted } => {
                let choices = accepted.iter().map(|(width, height)| format!("{width}×{height}"))
                    .collect::<Vec<_>>().join(", ");
                write!(f, "Screenshot {} is {}×{} pixels and cannot be prepared without upscaling \
                    or changing its aspect ratio. Capture or export a screenshot in one of Apple's \
                    accepted sizes: {choices}. Piston does not crop or stretch screenshots.",
                    path.display(), original.0, original.1)
            }
            Self::StoreScreenshotWrite { path, source } => write!(f,
                "Cannot save prepared screenshot {}: {source}", path.display()),
            Self::StoreScreenshotFlush { path, source } => write!(f,
                "Cannot finish prepared screenshot {}: {source}", path.display()),
            Self::StoreCatalogEmpty => write!(f, "Apple returned an empty asset specification catalog."),
            Self::StoreScreenshotsMissing => write!(f,
                "Add at least one iPhone screenshot before deploying."),
            Self::StoreScreenshotGroupCount { group, count, maximum } => write!(f,
                "{group} has {count} screenshots; Apple allows at most {maximum}. \
                 Remove screenshots and rerun."),
            Self::StoreScreenshotFileSize { path, size, maximum } => write!(f,
                "{} is {size} bytes; Apple requires a nonempty image of at most {maximum} bytes.",
                path.display()),
            Self::StoreScreenshotSync { source, recovery } => {
                writeln!(f, "Screenshot synchronization failed: {source}")?;
                if recovery.is_empty() {
                    writeln!(f, "Previous screenshot placements and order were restored.")?;
                } else {
                    writeln!(f, "Could not fully restore previous placements:")?;
                    for (group, error) in recovery { writeln!(f, "{group}: {error}")?; }
                    writeln!(f, "Open the version's Media Manager in App Store Connect \
                        and check the order before retrying.")?;
                }
                write!(f, "Original image assets remain in Asset Library.")
            }
            Self::StoreScreenshotGroupMismatch { images } => {
                write!(f, "These screenshots do not fit one Apple device display group:")?;
                for (path, width, height) in images {
                    write!(f, " {} ({width}x{height})", path.display())?;
                }
                write!(f, ". Capture this device family's screenshots \
                    at the same supported resolution, then rerun.")
            }
            Self::StoreVersionNotEditable { version, state } => write!(f,
                "App Store version {version} is {state}, so Apple does not allow screenshot changes. \
                 Resolve its review state in App Store Connect, or increase package.version \
                 in Cargo.toml for a new release."),
            Self::StoreUploadUrlsRejected => write!(f,
                "Apple rejected freshly issued screenshot upload URLs. Check your network or proxy \
                 and retry later. Existing placements are unchanged."),
            Self::StoreUploadOperationsMissing { image_id } => write!(f,
                "Apple supplied no upload operations for image {image_id}."),
            Self::StoreUploadRangeInvalid => write!(f, "Apple returned an invalid upload byte range."),
            Self::StoreUploadRangeTooLarge => write!(f,
                "Apple's upload byte range exceeds the screenshot size."),
            Self::StoreUploadRangeDiscontinuous => write!(f,
                "Apple's upload ranges overlap or leave a gap."),
            Self::StoreUploadRangeIncomplete => write!(f,
                "Apple's upload ranges do not cover the screenshot."),
            Self::StoreUploadInsecureUrl => write!(f, "Apple returned an insecure asset upload URL."),
            Self::StoreUploadAuthorization => write!(f,
                "Unexpected authorization header in asset upload instructions."),
            Self::StoreScreenshotUpload { image_id, status } => {
                write!(f, "Screenshot {image_id} upload failed (")?;
                match status {
                    Some(status) => write!(f, "HTTP {status}")?,
                    None => write!(f, "network connection or timeout")?,
                }
                write!(f, "). Check your network and rerun cargo piston deploy to resume. \
                    Existing screenshot placements are unchanged.")
            }
            Self::StoreImageProcessingTimeout { image_id } => write!(f,
                "Apple is still processing screenshot {image_id}. Rerun cargo piston deploy later; \
                 it will reuse the uploaded image. Existing screenshot placements are unchanged."),
            Self::StoreImageSpecificationMismatch { image_id, group, spec } => write!(f,
                "Apple processed image {image_id} as specification {spec}, incompatible with {group}."),
            Self::StoreImageRejected { image_id, state, details } => write!(f,
                "Apple image {image_id} is {state}: {details}. \
                 Check this image in App Store Connect's Asset Library."),
            Self::StoreScreenshotOrder { group } => write!(f,
                "Apple returned a different screenshot order for {group}."),
            Self::StoreCatalogFeatureMissing => write!(f,
                "Apple's asset catalog has no App Store version feature."),
            Self::StoreCatalogPolicyMissing => write!(f,
                "Apple's asset catalog has no screenshot policy."),
            Self::StoreCatalogSpecificationsMissing => write!(f,
                "Apple's asset catalog has no screenshot specifications."),
            Self::StoreCatalogCategoryRejected => write!(f,
                "Apple's screenshot catalog rejects screenshot image assets."),
            Self::StoreCatalogDimensionsInvalid => write!(f,
                "Apple's screenshot specification has invalid dimensions."),
            Self::StoreCatalogSpecificationsUnavailable => write!(f,
                "Apple returned no usable iPhone or iPad screenshot specifications."),
            Self::CommandWait { program, source } => {
                write!(f, "Cannot wait for {}: {source}", program.to_string_lossy())
            }
            Self::CommandLogged { log, diagnostics, error } => {
                write!(f, "{error}. Full log: {}\n{diagnostics}", log.display())
            }
            Self::SetPermissions { path, source } => {
                write!(f, "Cannot make {} executable: {source}", path.display())
            }
            Self::IdentityMissing { team } => write!(f, "No identity is stored for team {team}."),
            Self::IdentityImportInvalid => {
                write!(f, "Paste the complete piston-identity: line from cargo piston backup.")
            }
            Self::IdentityTeamMismatch { expected, actual } => {
                write!(f, "This identity belongs to Apple team {actual}, not {expected}. \
                    Ask for that team's identity.")
            }
            Self::IdentityConflict { team } => write!(f,
                "Team {team} already has a different saved identity. Piston will not overwrite it. \
                 Use the matching identity or the intended computer."),
            Self::IdentityTeamInvalid { team } => {
                write!(f, "Apple Team ID {team:?} must contain 10 uppercase letters or digits.")
            }
            Self::IdentityRootMismatch => {
                write!(f, "Invalid team identity: the root key does not match.")
            }
            Self::IdentityOwnerMismatch => write!(f,
                "This identity does not own the project's developer configuration. Ask a teammate \
                 to run cargo piston backup on the computer that configured it, then run \
                 cargo piston restore \"secret data\"."),
            Self::HostAndroidSdkMissing { path } => write!(f,
                "Android SDK command-line tools are missing from {}.\n\n\
                 Install Android Studio or its command-line tools, then set ANDROID_HOME.\n\n{}",
                path.display(), host::Cli::link("Android tools", "https://developer.android.com/studio")),
            Self::HostJavaMissing { path, source } => write!(f,
                "Cannot locate Java at {}: {source}. Install JDK 17 or later and set JAVA_HOME.",
                path.display()),
            Self::HostBuildUnsupported => {
                write!(f, "Builds from Linux hosts are not implemented. Use a macOS host.")
            }
            Self::HostUsernameMissing => write!(f, "whoami returned no user name."),
            Self::XcodeSystemVersionInvalid { value } => {
                write!(f, "Cannot read the macOS version: {value:?}.")
            }
            Self::XcodeArchitectureUnsupported { architecture } => {
                write!(f, "Cannot select an Xcode installer for architecture {architecture:?}.")
            }
            Self::XcodeSelectedTooOld { version, minimum } => write!(f,
                "DEVELOPER_DIR explicitly selects Xcode {}.{}.{}; this build requires Xcode \
                 {minimum} or later. Unset DEVELOPER_DIR for automatic selection.",
                version[0], version[1], version[2]),
            Self::XcodeSdkUnavailable { developer, sdk, minimum, output } => write!(f,
                "Xcode at {} does not provide the required {sdk} SDK (minimum major version {minimum}). \
                 Install the full Xcode application, or unset DEVELOPER_DIR if it selects an incomplete \
                 installation.\n{}", developer.display(), String::from_utf8_lossy(&output.stderr)),
            Self::XcodeIncomplete { developer } => {
                write!(f, "{} is not a full Xcode installation.", developer.display())
            }
            Self::XcodeIncompatible { developer } => {
                write!(f, "{} is not an Xcode compatible with this macOS.", developer.display())
            }
            Self::XcodeDiscovery { path, error } => {
                write!(f, "Skipping Xcode at {}: {error}", path.display())
            }
            Self::XcodeCatalogUnavailable { source } => write!(f,
                "Cannot load the Xcode Releases catalog: {source}. Existing installations can be \
                 selected with DEVELOPER_DIR; retry setup when the catalog is available."),
            Self::XcodeSystemTooOld { sdk, minimum } => write!(f,
                "No compatible release provides the required {sdk} SDK on this Mac. Update macOS \
                 to at least {}.{}.{} and rerun this command.", minimum[0], minimum[1], minimum[2]),
            Self::XcodeReleaseUnavailable { architecture, sdk } => write!(f,
                "The release catalog has no supported Xcode installer for {architecture} and the \
                 required {sdk} SDK.\n\n{}",
                host::Cli::link("Xcode requirements", "https://developer.apple.com/xcode/system-requirements/")),
            Self::XcodeDestinationExists { path } => write!(f,
                "{} already exists but was not usable. Move it aside or repair it, then rerun. \
                 Piston will not replace an existing installation.", path.display()),
            Self::XcodeArchiveMismatch { version, build, staging } => write!(f,
                "The archive contains a different Xcode than {version} ({build}). Extracted files \
                 remain at {}.", staging.display()),
            Self::XcodeInstallMove { destination, staging, source } => write!(f,
                "Cannot install Xcode at {}: {source}. Extracted files remain at {}.",
                destination.display(), staging.display()),
            Self::XcodeLicenseIncomplete => write!(f, "Xcode license acceptance was not completed."),
            Self::XcodeAdministratorApprovalIncomplete => {
                write!(f, "Administrator approval was not completed.")
            }
            Self::LinuxVmNameInvalid { name } => write!(f,
                "Invalid Lima VM name {name:?}. Use an alphanumeric name with '.', '_' or '-'."),
            Self::LinuxVmHomeMissing => write!(f, "Set HOME or provide --lima-home."),
            Self::LinuxVmLocked { name, source } => {
                write!(f, "Another Piston session owns VM {name}: {source}")
            }
            Self::LinuxVmSignalHandler(source) => {
                write!(f, "Cannot install Linux session shutdown handler: {source}")
            }
            Self::LinuxVmUnavailable { name, state } => write!(f,
                "VM {name} is {}. Stop it first or select another --vm; Piston only shuts down its \
                 own sessions.", state.as_deref().unwrap_or("in an unknown state")),
            Self::LinuxVmTargetMissing => write!(f, "Linux rustc did not report a host target."),
            Self::LinuxVmTargetMismatch { actual, requested } => write!(f,
                "This VM builds {actual}; choose a VM matching the requested --target {requested}."),
            Self::LinuxVmDesktopTimeout => write!(f,
                "Linux desktop forwarding did not become ready. Check the VM desktop logs."),
            Self::LinuxVmStopFailed { name, home } => write!(f,
                "Could not stop VM {name}. Run limactl stop --force with LIMA_HOME={}",
                home.display()),
            Self::LinuxVmInterrupted => write!(f, "Linux development interrupted."),
            Self::ApplePlistRead { path, source } => {
                write!(f, "Cannot read property list {}: {source}", path.display())
            }
            Self::ApplePlistWrite { path, source } => {
                write!(f, "Cannot write property list {}: {source}", path.display())
            }
            Self::ApplePlistJson(source) => write!(f, "Invalid property list: {source}"),
            Self::ApplePlistField { key } => {
                write!(f, "The app Info.plist needs a nonempty {key} string.")
            }
            Self::ApplePlistInput(source) => {
                write!(f, "Cannot send the property list to plutil: {source}")
            }
            Self::ApplePlistWait(source) => write!(f, "Cannot wait for plutil: {source}"),
            Self::SigningCertificateMissing { name } => {
                write!(f, "No certificate matching '{name}' was found in Keychain.")
            }
            Self::SigningCertificateAmbiguous { name, teams } => write!(
                f, "Certificates named '{name}' belong to multiple teams: {}. Select a Team ID.",
                teams.join(", "),
            ),
            Self::PushKeyRead { path, source } => write!(
                f, "Cannot read the APNs key at {}: {source}\n\n\
                 Enter the path to your downloaded .p8 file. If it is lost, create a new key:\n{}",
                path.display(), host::Cli::link(
                    "Apple keys", "https://developer.apple.com/account/resources/authkeys/list",
                ),
            ),
            Self::PushKeyInRepository { path } => write!(
                f, "Move the APNs .p8 file outside the repository, for example to ~/.keys.\n\
                 Piston stores the imported credentials encrypted in AIR.\n\nCurrent path: {}",
                path.display(),
            ),
            Self::PushIdentifiersInvalid => write!(
                f, "APNs Key ID and Team ID must be 10 uppercase letters or digits.",
            ),
            Self::PushKeyIdMismatch { key_id, path } => write!(
                f, "APNs Key ID {key_id} does not match the .p8 filename {}.", path.display(),
            ),
            Self::PushKeyEncoding(source) => {
                write!(f, "The APNs .p8 file must contain an EC private key: {source}")
            }
            Self::PushKeySigning(source) => {
                write!(f, "The APNs .p8 file is not a valid P-256 signing key: {source}")
            }
            Self::PushIdentityMissing { team } => write!(
                f, "Developer identity for team {team} is missing. \
                 Run cargo piston setup or cargo piston restore.",
            ),
            Self::PushSetupTimeout => write!(
                f, "Notification setup timed out before confirmation. Check your connection \
                 and rerun; Piston will resume the same registration.",
            ),
            Self::PushLocationError(source) => {
                write!(f, "Cannot create notification location: {source}")
            }
            Self::PushContractError(source) => {
                write!(f, "Cannot create notification contract: {source}")
            }
            Self::PushInboxError(source) => write!(f, "Cannot create deployment inbox: {source}"),
            Self::PushRegistrationInvalid(source) => write!(
                f, "Invalid notification registration in the developer contract: {source}. Restore its backup.",
            ),
            Self::PushLocationMismatch => {
                write!(f, "The saved notification location does not match this app.")
            }
            Self::PushConnectionError(source) => write!(f, "Cannot connect to AIR: {source}"),
            Self::PushInitializationError(source) => {
                write!(f, "Cannot initialize notification contract: {source}")
            }
            Self::PushUpdateDenied => write!(
                f, "The deployment identity cannot update this notification contract.",
            ),
            Self::PushCredentialsInvalid(source) => write!(f,
                "Invalid APNs credentials in the developer contract: {source}."),
            Self::PushCredentialsUnconfirmed => {
                write!(f, "AIR did not confirm acceptance of the APNs credentials.")
            }
            Self::PushSharingDisconnected => write!(
                f, "AIR disconnected before the notification configuration was shared.",
            ),
            Self::PushInboxSignatureRejected => {
                write!(f, "AIR rejected the notification inbox signature.")
            }
            Self::PushConfirmationDisconnected => {
                write!(f, "AIR disconnected before confirming notification setup.")
            }
            Self::PushUpdateRejected => {
                write!(f, "AIR rejected the notification configuration update.")
            }
            Self::PushIpaApplicationCount { count } => {
                write!(f, "The exported IPA must contain one app; found {count}.")
            }
            Self::PushProfileMissing { topic, output } => write!(
                f, "Push notifications require an embedded provisioning profile for {topic}. \
                 Enable Push Notifications for this App ID and rebuild with automatic signing.\n{}",
                String::from_utf8_lossy(&output.stderr).trim(),
            ),
            Self::PushTopicMismatch { topic } => {
                write!(f, "The app bundle ID does not match APNs topic {topic}.")
            }
            Self::PushArchitectures { output } => write!(
                f, "Cannot inspect the app's signed architectures: {}",
                String::from_utf8_lossy(&output.stderr).trim(),
            ),
            Self::PushEntitlementsRead { architecture, output } => write!(
                f, "Cannot inspect notification entitlements for {architecture}: {}",
                String::from_utf8_lossy(&output.stderr).trim(),
            ),
            Self::PushEntitlementsParse(source) => {
                write!(f, "Invalid signed entitlements: {source}")
            }
            Self::PushProfileNotDictionary => {
                write!(f, "Invalid provisioning profile dictionary.")
            }
            Self::PushProfileEntitlementsMissing => {
                write!(f, "The provisioning profile contains no entitlements.")
            }
            Self::PushEntitlementsNotDictionary => {
                write!(f, "Invalid signed entitlements dictionary.")
            }
            Self::PushEnvironmentMismatch { key, environment, topic } => write!(
                f, "The signed app and its provisioning profile must both grant \
                 {key}={environment}. The notification server uses that environment. Enable Push \
                 Notifications for {topic} and rebuild using the matching signing profile.",
            ),
            Self::PushTeamMismatch { team } => write!(
                f, "The signed app and APNs credentials must both belong to Apple team {team}.",
            ),
            Self::PushProfileTopicMismatch { topic } => write!(
                f, "Push signing requires an explicit provisioning profile matching APNs topic {topic}.",
            ),
            Self::PushProfileExpired => write!(f, "The push provisioning profile has expired."),
            Self::AndroidArgumentsUnsupported => write!(
                f, "Android NativeActivity does not accept desktop command-line arguments.",
            ),
            Self::AndroidBridgeMissing => write!(
                f, "The selected Maverick dependency needs its android/ Gradle bridge. \
                    Use maverick_os 3.0.3 or later.",
            ),
            Self::AndroidNdkMissing { path } => {
                write!(f, "Configured Android NDK does not exist: {}", path.display())
            }
            Self::AndroidDeviceUnavailable { serial, state } => write!(
                f, "Android device {serial} is {state}. Unlock it, enable USB debugging and \
                    accept this computer's debugging prompt.",
            ),
            Self::AndroidDevicesAmbiguous { serials } => write!(
                f, "Multiple Android devices: {}. Select one with --device <serial>.",
                serials.join(", "),
            ),
            Self::AndroidTargetIncompatible { serial, target, abis } => write!(
                f, "Rust target {} cannot run on {serial}. Device ABIs: {}.",
                target.as_deref().unwrap_or("selected automatically"), abis.join(", "),
            ),
            Self::AndroidToolMissing { path } => {
                write!(f, "Required Android tool was not found at {}", path.display())
            }
            Self::AndroidBuildToolsMissing { path } => {
                write!(f, "No Android build tools are installed at {}", path.display())
            }
            Self::AndroidHostToolchainMissing { path } => {
                write!(f, "No Android NDK host toolchain is installed at {}", path.display())
            }
            Self::AndroidManifestMissing { path } => {
                write!(f, "AAPT2 did not produce a linked Android manifest at {}", path.display())
            }
            Self::AndroidSdkManagerMissing { path } => write!(
                f, "Android SDK command-line tools are missing from {}. \
                    Install them in Android Studio's SDK Manager.", path.display(),
            ),
            Self::AndroidEmulatorNotFound { selector, available } => write!(
                f, "No Android device or AVD matches {selector}. Available AVDs: {}",
                available.join(", "),
            ),
            Self::AndroidEmulatorPortsUnavailable => {
                write!(f, "No free Android emulator console ports.")
            }
            Self::AndroidEmulatorClosed { log } => {
                write!(f, "Android emulator closed before booting. See {}", log.display())
            }
            Self::AndroidBootTimeout => {
                write!(f, "Android did not finish booting within three minutes.")
            }
            Self::AndroidInterrupted => write!(f, "Android development interrupted."),
            Self::AndroidAppExited => write!(f, "Android app exited before log streaming began."),
            Self::AndroidEmulatorControl(source) => {
                write!(f, "Cannot stop or wait for the Android emulator: {source}")
            }
            Self::AndroidSignalHandler(source) => write!(f, "Cannot watch Android shutdown: {source}"),
            Self::MacResourcesInvalid => write!(f, "macos.resources must contain paths."),
            Self::MacResourceInvalid { path } => {
                write!(f, "Invalid macOS resource path: {}", path.display())
            }
            Self::LinuxAppImageTargetUnsupported { target } => write!(
                f, "Unsupported AppImage target: {target}. \
                    Supported architectures: x86_64, aarch64, arm, i686, ppc64le, riscv64.",
            ),
            Self::LinuxRuntimeMissing { path } => {
                write!(f, "AppImage runtime not found at {}", path.display())
            }
            Self::LinuxImageError(source) => write!(f, "Cannot package AppImage: {source}"),
            Self::LinuxSigningInput(source) => {
                write!(f, "Cannot send the signing passphrase to GPG: {source}")
            }
            Self::CommandSpawn { program, source } => {
                write!(f, "Could not start {}: {source}", program.to_string_lossy())
            }
            Self::CommandFailed { program, output } => write!(
                f, "{} failed ({}).\n{}{}", program.to_string_lossy(), output.status,
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr),
            ),
            Self::CommandStatus { program, status } => {
                write!(f, "{} failed: {status}", program.to_string_lossy())
            }
            Self::MissingEnvironment { name } => write!(f, "Set the {name} environment variable."),
            Self::MissingSetting { key } => write!(f, "Missing configuration setting: {key}."),
            Self::InvalidBundleId { value } => write!(f, "Invalid bundle identifier: {value}."),
            Self::PlistNotDictionary { path } => {
                write!(f, "{} must contain a property-list dictionary.", path.display())
            }
            Self::ArtifactNotProduced { package, target } => {
                write!(f, "Cargo produced no application artifact for {package}")?;
                if let Some(target) = target { write!(f, " ({target})")?; }
                Ok(())
            }
            Self::ReadFile { path, source } => {
                write!(f, "Could not read {}: {source}", path.display())
            }
            Self::SetupRequired { error, .. } => std::fmt::Display::fmt(error, f),
            Self::BuildError(err) => write!(f, "Failed to build the binary: {}", err),
            Self::CargoParseError(err) => {
                write!(f, "Could not read the Cargo project:\n{err}")?;
                if err.contains("Operation not permitted") || err.contains("Permission denied") {
                    write!(f, "\n\nAllow your terminal app to read the project folder. \
                        If it is in Documents, Desktop, or Downloads, check macOS System Settings \
                        > Privacy & Security > Files and Folders, or move the project to an \
                        accessible folder. Then rerun the same cargo piston command.")?;
                }
                Ok(())
            }
            Self::WriteFileError(err) => write!(f, "Failed to write file: {}", err),
            Self::WriteImageError(err) => write!(f, "Failed to write image: {}", err),
            Self::FileFlushError(err) => write!(f, "Failed to flush file: {}", err),
            Self::ParseManifestError(err) => {
                write!(f, "Failed to parse package.metadata.android value: {}", err)
            }
            Self::WriteManifestError(err) => {
                write!(f, "Failed to write AndroidManifest.xml contents: {}", err)
            }
            Self::UnsupportedTargetError(err) => {
                write!(f, "User Specified Target is Unsupported: {}", err)
            }
            Self::XcodeInstallError(err) => write!(f, "Xcode Installation not Found: {}", err),
            Self::XcodeBuildError(err) => {
                write!(f, "Xcode could not complete the operation: {err}")
            }
            Self::ADBDevicesError(err) => {
                write!(f, "Error Running 'ADB Devices', check installation and .env path: {}", err)
            }
            Self::ParseUTF8Error(err) => write!(f, "Error Parsing UTF8: {}", err),
            Self::XcrunDevicectlError(err) => f.write_str(err),
            Self::KeyChainUnlockError(err) => {
                write!(f, "Failed to unlock the security keychain: {}", err)
            }
            Self::KeyChainImportError(err) => {
                write!(f, "Failed to import profile to security keychain: {}", err)
            }
            Self::IntoJSONError(err) => write!(f, "Error converting into JSON: {}", err),
            Self::OpenSSLKeyGenError(err) => {
                write!(f, "Error Generating New Key with openssl: {}", err)
            }
            Self::OpenSSLCSRError(err) => write!(f, "Error Generating CSR with openssl: {}", err),
            Self::Base64DecodeError(err) => write!(f, "Error Decoding Base 64: {}", err),
            Self::ASCAuthenticationError(err) => err.fmt(f),
            Self::ASCClientJWTEncodeError(err) => write!(f, "Error encoding JWT: {}", err),
            Self::WritePlUtilError(err) => write!(f, "Failed to write Plutil contents: {}", err),
            Self::XcrunInstallError(err) => {
                write!(f, "Error installing app bundle to target device via xcrun: {}", err)
            }
            Self::XcrunLaunchError(err) => {
                write!(f, "Error launching app bundle on target device via xcrun: {}", err)
            }
            Self::CodesignError(err) => f.write_str(err),
            Self::DeviceProvisionError(err) => {
                write!(f, "Error provisioning target device with idp: {}", err)
            }
            Self::SecurityFindIdentityError(err) => write!(
                f,
                "Error checking for local identities with `security find-identity -v -p codesigning`: {}",
                err
            ),
            Self::PlutilConvertError(err) => {
                write!(f, "Error using plutil to convert filetype`: {}", err)
            }
            Self::UnsupportedOSError { os, target, .. } => {
                write!(f, "Host system: {:?} does not support the target: {:?}", os, target)
            }
            Self::LipoError { first_binary, second_binary, source } => write!(
                f,
                "Error making universal binary with Lipo first binary: {:?}, second binary: {:?}, source: {}",
                first_binary, second_binary, source
            ),
            Self::CopyFileError { input_path, output_path, .. } => {
                write!(f, "Failed to copy {:?} to {:?}", input_path, output_path)
            }
            Self::MacOSIconError { input_path, output_path, .. } => {
                write!(f, "Failed to format icon {:?} to {:?}", input_path, output_path)
            }
            Self::ReadDirError { path, .. } => write!(f, "Failed to read directory {:?}", path),
            Self::MapDirError { path, .. } => {
                write!(f, "Failed to map directory contents {:?}", path)
            }
            Self::QueryProvisionError { path, .. } => {
                write!(f, "Failed to query the security provision profile {:?}", path)
            }
            Self::RemoveSubdirError { path, .. } => {
                write!(f, "Failed to remove subdirectory {:?}", path)
            }
            Self::RemoveFileError { path, .. } => write!(f, "Failed to remove file {:?}", path),
            Self::CreateFileError { path, .. } => write!(f, "Failed to Create file {:?}", path),
            Self::RenameFileError { path, .. } => write!(f, "Failed to Rename file {:?}", path),
            Self::CreateDirAllError { path, .. } => {
                write!(f, "Failed to create dir all {:?}", path)
            }
            Self::OpenImageError { path, .. } => write!(f, "Failed to open image {:?}", path),
            Self::Generic(err) => f.write_str(err),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Parser)]
#[group(skip)]
struct CommonArgs {
    #[clap(flatten)]
    subcommand_args: cargo_subcommand::Args,
    #[clap(long = "release-external")]
    external: bool,
    #[clap(long = "release-appimage")]
    appimage: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Parser)]
#[group(skip)]
struct BuildArgs {
    #[clap(flatten)]
    common: CommonArgs,
    #[clap(long)]
    dry_run: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Parser)]
#[group(skip)]
struct RunArgs {
    #[clap(flatten)]
    common: CommonArgs,
    #[clap(long)]
    device: Option<String>,
}

#[derive(clap::Subcommand)]
enum PistonSubCmd {
    /// Choose or create this project's developer team identity.
    Setup,
    /// Select a team identity and print its private backup data.
    Backup,
    /// Restore the quoted private data printed by cargo piston backup.
    Restore { secret: String },
    /// Build and run on the host, a mobile device/emulator, or a Linux VM.
    Dev(crate::dev::DevelopmentArgs),
    /// Upload an iOS build to TestFlight or the App Store.
    Deploy(crate::ios::DeploymentArgs),
    /// Build an app bundle for a selected target.
    #[clap(visible_alias = "b")]
    Build(BuildArgs),
    /// Build and launch using the legacy platform runner.
    #[clap(visible_alias = "r")]
    Run(RunArgs),
    /// Show available connected devices.
    #[clap(visible_alias = "l")]
    ListDevices,
    /// Show the installed Piston version.
    #[clap(visible_alias = "v")]
    Version,
}

#[derive(clap::Subcommand)]
enum PistonCmd {
    #[command(
        about = "Build, run, and deploy Rust apps.",
        after_help = "Common commands:\n  \
            cargo piston dev                    Run on this computer\n  \
            cargo piston dev ios                Use an iPhone or iOS Simulator\n  \
            cargo piston dev android            Use an Android device or emulator\n  \
            cargo piston deploy testflight      Upload and wait for TestFlight\n\n\
            Run cargo piston to choose this project's developer identity.\n\
            Setup links open when clicked. Enter q at a prompt to cancel.\n\
            Use cargo piston <command> --help for command options."
    )]
    Piston {
        #[clap(subcommand)]
        cmd: Option<PistonSubCmd>,
    },
}

#[derive(Debug)]
enum Platform { Android, Ios, Linux, Windows, Macos, Unknown, }

impl Platform {
    fn from_target(target: &str) -> Self {
        match target.to_lowercase().as_str() {
            "macos" | "aarch64-apple-darwin" | "x86_64-apple-darwin" => Self::Macos,
            "ios" | "aarch64-apple-ios" | "x86_64-apple-ios" => Self::Ios,
            "linux" | "aarch64-unknown-linux-gnu" | "x86_64-unknown-linux-gnu" => Self::Linux,
            "windows" | "x86_64-pc-windows-gnu" => Self::Windows,
            "android" | "aarch64-linux-android" | "x86_64-linux-android" => Self::Android,
            _ => Self::Unknown,
        }
    }
}

#[test]
fn test_platform_from_target() {
    assert!(matches!(Platform::from_target("aarch64-apple-darwin"), Platform::Macos));
    assert!(matches!(Platform::from_target("aarch64-apple-ios"), Platform::Ios));
    assert!(matches!(Platform::from_target("aarch64-unknown-linux-gnu"), Platform::Linux));
    assert!(matches!(Platform::from_target("x86_64-pc-windows-gnu"), Platform::Windows));
    assert!(matches!(Platform::from_target("aarch64-linux-android"), Platform::Android));
    assert!(matches!(Platform::from_target("some-unknown-target"), Platform::Unknown));
}
