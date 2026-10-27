use toml_edit::{Item, Value};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use crate::Result;
use crate::apple::{notifications, xcode};
use crate::dev::DevelopmentArgs;
use crate::devices::IOSDevice;
use crate::host::{self, Host};
use crate::manifest::Manifest;
use crate::project::Project;
use crate::cli::ApplicationArgs;
use super::{IOSApplication, IOSBuilder};
use super::IOSRunner;
use super::development::IOSDevelopment;
use super::simulator::Simulator;

#[derive(Debug)]
pub enum Error {
    NotificationSetupDeviceRequired,
    DeviceTargetRequired,
    ExtendedAttributesFailed { output: Vec<u8> },
    IosDevicesAmbiguous { devices: Vec<(String, String)> },
    IosDeviceUnavailable { name: String },
    IosDeviceDisappeared { name: String },
    IosDeviceLocked { name: String },
    IosLockStateMissing,
    IosDeveloperModeDisabled { name: String, version: Option<String> },
    IosConnectionNotReady { name: String },
    IosDeviceListMissing,
    IosSimulatorNotFound { selector: String },
    IosSimulatorRuntimesMissing,
    IosSimulatorRuntimeUnavailable,
    IosSimulatorDeviceTypeMissing,
    IosMinimumVersion { value: String },
    IosStoreIdentifierMissing,
    IosStoreTargetInvalid,
    IosSimulatorTarget { target: String },
    IosTargetUnsupported { target: String },
    IosStoreIconMissing,
    IosStoreIconInvalid { path: PathBuf },
    IosResourceInvalid { path: PathBuf },
    IosResourceConflict { path: PathBuf },
    IosImageAssetCreate { path: PathBuf, source: std::io::Error },
    XcrunDevicectlError(String),
    XcrunInstallError(String),
    XcrunLaunchError(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotificationSetupDeviceRequired => write!(f,
                "Notification --setup requires a connected iPhone or iPad."),
            Self::DeviceTargetRequired =>
                write!(f, "Select a device target for the connected iPhone."),
            Self::ExtendedAttributesFailed { output } =>
                f.write_str(&String::from_utf8_lossy(output)),
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
            Self::IosMinimumVersion { value } => write!(f,
                "minimum_os {value:?} must be an iOS version of 17.0 or newer \
                 for CoreDevice development."),
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
            Self::XcrunDevicectlError(err) => f.write_str(err),
            Self::XcrunInstallError(err) => {
                write!(f, "Error installing app bundle to target device via xcrun: {}", err)
            }
            Self::XcrunLaunchError(err) => {
                write!(f, "Error launching app bundle on target device via xcrun: {}", err)
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IosImageAssetCreate { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub struct Cli;

impl Cli {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        let application = Self::application(args.application.clone(), false)?;
        let developer = host::Cli::prepare_xcode("iphoneos", 0)?;
        let mut session = IOSDevelopment::new(application, developer);
        let simulator = args.simulator
            || args.application.cargo.target.as_deref().is_some_and(|target| {
                matches!(target, "aarch64-apple-ios-sim" | "x86_64-apple-ios")
            });
        let device = if simulator { None } else {
            match session.select_device(args.device.as_deref()) {
                Ok(device) => device,
                Err(error) => {
                    if !host::Cli::interactive() { return Err(error); }
                    let Some(Error::IosDevicesAmbiguous { devices }) = error.downcast_ref()
                    else { return Err(error); };
                    eprintln!("\nSelect an iOS device:\n");
                    for (index, (name, udid)) in devices.iter().enumerate() {
                        eprintln!("  {}. {name} ({udid})", index + 1);
                    }
                    eprintln!();
                    loop {
                        let input = host::Cli::prompt("Device number", "")?;
                        let selected = input.parse::<usize>().ok()
                            .and_then(|number| number.checked_sub(1))
                            .and_then(|index| devices.get(index));
                        let Some((name, udid)) = selected else {
                            eprintln!("Enter a number from 1 to {}.", devices.len());
                            continue;
                        };
                        let device = session.select_device(Some(udid))?
                            .ok_or_else(|| Error::IosDeviceDisappeared { name: name.clone() })?;
                        break Some(device);
                    }
                }
            }
        };
        if let Some(mut device) = device {
            if !device.paired {
                println!(
                    "Pairing with {}. Unlock the phone and accept Trust This Computer.",
                    device.name
                );
                device = session.pair(&device)?;
            }
            println!("Connecting to {}", device.name);
            let device = session.connect(&device)?;
            if session.application.sdk() != "iphoneos" {
                return Err(Error::DeviceTargetRequired.into());
            }
            session.developer = Self::prepare(&session.application, false)?;
            let signing = xcode::Cli::development(session.application.team(), &session.developer)?;
            session.application.remember_team(signing.team())?;
            Self::notifications(&mut session.application, signing.team(), true, args.setup)?;
            println!(
                "Building {} for {} ({})",
                session.application.package().name,
                device.name,
                session.application.target()
            );
            let (bundle, identifier) =
                session.application.build(&session.developer, signing.team(), &mut |message| {
                    eprint!("{message}")
                })?;
            println!("Signing for development");
            let signed = signing.sign(
                &bundle,
                session.application.work(),
                &device.udid,
                session.application.entitlements().as_deref(),
            )?;
            session.application.verify_notifications(&signed)?;
            println!("Installing {} on {}", session.application.package().name, device.name);
            session.install(&signed, &device)?;
            loop {
                println!("Launching {identifier}. App output follows; press Ctrl-C to stop.");
                match session.launch(&identifier, &device, &args.app_args) {
                    Err(error) if host::Cli::interactive()
                        && matches!(error.downcast_ref::<Error>(),
                            Some(Error::IosDeviceLocked { .. })) =>
                    {
                        eprintln!("\n{} is locked. Unlock it and keep the screen awake.", device.name);
                        eprintln!("The app is installed; Piston will retry launching it.\n");
                        loop {
                            eprint!("Press Enter after unlocking, or q to cancel: ");
                            io::stderr().flush()?;
                            let mut input = String::new();
                            let count = io::stdin().read_line(&mut input)?;
                            if count == 0 || input.trim().eq_ignore_ascii_case("q") {
                                println!("Launch cancelled. The app is still installed.");
                                return Ok(());
                            }
                            if input.trim().is_empty() { break; }
                            eprintln!("Press Enter to retry, or type q to cancel.");
                        }
                    }
                    result => return result,
                }
            }
        } else {
            if args.setup {
                return Err(Error::NotificationSetupDeviceRequired.into());
            }
            println!("Using the iOS Simulator.");
            session.application.simulator_target()?;
            session.developer = Self::prepare(&session.application, false)?;
            let simulator = Simulator::select(&session.developer, args.device.as_deref())?;
            println!("Opening {}", simulator.name);
            simulator.boot(&session.developer)?;
            let team = session.application.team().unwrap_or("simulator").to_owned();
            println!(
                "Building {} for {} ({})",
                session.application.package().name,
                simulator.name,
                session.application.target()
            );
            let (bundle, identifier) =
                session
                    .application
                    .build(&session.developer, &team, &mut |message| eprint!("{message}"))?;
            simulator.install(&session.developer, &bundle)?;
            println!("Launching {identifier}. App output follows; press Ctrl-C to stop.");
            simulator.launch(&session.developer, &identifier, &args.app_args)
        }
    }

    pub fn application(args: ApplicationArgs, store: bool) -> Result<IOSApplication> {
        let manifest = Project::manifest(&args.cargo)?;
        let root = manifest.path().parent().unwrap();
        for (name, explanation) in [
            ("team", "10-character Apple Team ID"),
            ("bundle_id", "bundle identifier, such as com.example.app"),
            ("minimum_os", "minimum iOS version, 17.0 or newer"),
            ("build_number", "numeric build number with one to three components"),
            ("info_plist", "path to an existing Info.plist dictionary"),
            ("entitlements", "path to an existing entitlements plist dictionary"),
            ("resources", "TOML array of existing resource paths, such as [\"ios/LaunchScreen.storyboard\"]"),
        ] {
            if match name {
                "team" => args.team.is_some(),
                "bundle_id" => args.bundle_id.is_some(),
                "info_plist" => args.info_plist.is_some(),
                "entitlements" => args.entitlements.is_some(),
                _ => false,
            } { continue; }
            let current = manifest.setting(&["ios", name])?;
            let required = store && name == "bundle_id";
            if current.as_ref().map_or(!required, |value| {
                IOSApplication::valid_setting(name, value, root)
            }) { continue; }
            eprintln!("\niOS setting needs attention: {name}\n\nEnter a {explanation}.");
            if let Some(current) = &current { eprintln!("Current value: {current}"); }
            if !required {
                eprintln!("Or enter remove to use the default. Type q to cancel.\n");
            }
            loop {
                let input = host::Cli::prompt(name, "")?;
                if !required && input == "remove" {
                    manifest.remove(&["ios"], name)?;
                    break;
                }
                let value = if name == "resources" {
                    match input.parse::<Value>() {
                        Ok(value) => value,
                        Err(error) => { eprintln!("Enter a TOML array of paths: {error}"); continue; }
                    }
                } else { Value::from(input) };
                if !IOSApplication::valid_setting(name, &Item::Value(value.clone()), root) {
                    eprintln!("Enter a valid {explanation}.");
                    continue;
                }
                manifest.save(&["ios"], &[(name, value)])?;
                break;
            }
        }
        IOSApplication::new(args)
    }
    pub fn build(
        release: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>,
        device_target: Option<IOSDevice>,
    ) -> Result<(PathBuf, String)> {
        println!("building for iOS");
        Host::require_apple_tools(&target)?;

        let developer = host::Cli::prepare_xcode("iphoneos", 0)?;
        let mut op = IOSBuilder::new(release, target, cwd, env_vars, device_target, developer)?;
        println!("Preparing ios application resources");
        op.pre_build()?;
        println!("Compiling ios application");
        op.build()?;
        if op.keystore_path.is_none() || op.asc_api_key.is_none() {
            println!(
                "Keystore path or App Store Connect key missing; legacy automated signing is unavailable."
            );
        }
        println!("Packaging and signing the application");
        op.post_build()?;
        println!("iOS app bundle available at {}", op.output_path.display());
        if let Some(ipa) = &op.ipa_path { println!("IPA available at {}", ipa.display()); }

        Ok((op.ipa_path.unwrap(), op.bundle_id))
    }
    pub fn run(release: bool, cwd: PathBuf, env_vars: HashMap<String, String>, device: &IOSDevice) -> Result<()> {
        println!("running for IOS");
        let target_string = "aarch64-apple-ios".to_string();
        Host::require_apple_tools(&target_string)?;
        let builder = Self::build(release, target_string, cwd, env_vars, Some(device.clone()))?;
        let developer = host::Cli::prepare_xcode("iphoneos", 0)?;
        IOSRunner::deploy_usb(
            &device.id,
            &builder.0.display().to_string(),
            &builder.1,
            &developer,
        )?;

        Ok(())
    }
    pub fn prepare(application: &IOSApplication, store: bool) -> Result<PathBuf> {
        let developer = host::Cli::prepare_xcode(
            if store { "iphoneos" } else { application.sdk() },
            if store { 26 } else { 0 },
        )?;
        println!("Checking Rust libraries for {}", application.target());
        application.prepare_target(application.target())?;
        Ok(developer)
    }

    pub fn notifications(application: &mut IOSApplication, team: &str, sandbox: bool, replace: bool) -> Result<()> {
        let manifest = Manifest::new(application.package().manifest_path.as_std_path())?;
        if notifications::Cli::enabled(&manifest)? {
            let setup = application.notification_setup(team, sandbox)?;
            let id = notifications::Cli::configure(&setup, replace)?;
            application.use_notifications(setup, id)?;
        }
        Ok(())
    }
}
