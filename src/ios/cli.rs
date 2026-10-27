use toml_edit::{Item, Value};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use crate::apple::{notifications, xcode};
use crate::dev::DevelopmentArgs;
use crate::devices::IOSDevice;
use crate::host::{self, Host};
use crate::manifest::Manifest;
use crate::project::Project;
use crate::cli::ApplicationArgs;
use crate::{PistonError, Result};
use super::{IOSApplication, IOSBuilder};
use super::IOSRunner;
use super::development::IOSDevelopment;
use super::simulator::Simulator;

pub struct Cli;

impl Cli {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        let application = Self::application(args.application.clone(), false)?;
        let developer = host::Cli::prepare_xcode("iphoneos", 0)?;
        let mut session = IOSDevelopment::new(application, args.device.clone(), developer);
        let simulator = args.simulator
            || args.application.cargo.target.as_deref().is_some_and(|target| {
                matches!(target, "aarch64-apple-ios-sim" | "x86_64-apple-ios")
            });
        let device = if simulator { None } else { session.select_device()? };
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
                return Err(PistonError::UnsupportedTargetError(
                    "Select a device target for the connected iPhone.".into(),
                ));
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
                    Err(PistonError::IosDeviceLocked { name }) if host::Cli::interactive() => {
                        eprintln!("\n{name} is locked. Unlock it and keep the screen awake.");
                        eprintln!("The app is installed; Piston will retry launching it.\n");
                        loop {
                            eprint!("Press Enter after unlocking, or q to cancel: ");
                            io::stderr().flush()
                                .map_err(|error| PistonError::Generic(error.to_string()))?;
                            let mut input = String::new();
                            let count = io::stdin().read_line(&mut input)
                                .map_err(|error| PistonError::Generic(error.to_string()))?;
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
                return Err(PistonError::Generic(
                    "Notification --setup requires a connected iPhone or iPad.".into(),
                ));
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
