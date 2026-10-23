use std::collections::HashMap;
use std::path::PathBuf;
use crate::apple::NotificationCli;
use crate::apple::xcode::cli::SigningCli;
use crate::dev::DevelopmentArgs;
use crate::devices::IOSDevice;
use crate::host::Host;
use crate::host::cli::HostCli;
use crate::manifest::Manifest;
use crate::{PistonError, Result};
use super::{IOSApplication, IOSBuilder};
use super::IOSRunner;
use super::development::IOSDevelopment;
use super::simulator::Simulator;

pub struct IOSCli;

impl IOSCli {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        let application = IOSApplication::new(args.application.clone())?;
        let developer = HostCli::prepare_xcode("iphoneos", 0)?;
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
            let signing = SigningCli::development(session.application.team(), &session.developer)?;
            session.application.remember_team(signing.team())?;
            Self::notifications(&mut session.application, signing.team(), true)?;
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
            println!("Launching {identifier}. App output follows; press Ctrl-C to stop.");
            session.launch(&identifier, &device, &args.app_args)
        } else {
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
    pub fn build(
        release: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>,
        device_target: Option<IOSDevice>,
    ) -> Result<(PathBuf, String)> {
        println!("building for iOS");
        Host::require_apple_tools(&target)?;

        let developer = HostCli::prepare_xcode("iphoneos", 0)?;
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
        for message in op.post_build()? { println!("{message}"); }

        Ok((op.ipa_path.unwrap(), op.bundle_id))
    }
    pub fn run(release: bool, cwd: PathBuf, env_vars: HashMap<String, String>, device: &IOSDevice) -> Result<()> {
        println!("running for IOS");
        let target_string = "aarch64-apple-ios".to_string();
        Host::require_apple_tools(&target_string)?;
        let builder = Self::build(release, target_string, cwd, env_vars, Some(device.clone()))?;
        let developer = HostCli::prepare_xcode("iphoneos", 0)?;
        IOSRunner::deploy_usb(
            &device.id,
            &builder.0.display().to_string(),
            &builder.1,
            &developer,
        )?;

        Ok(())
    }
    pub fn prepare(application: &IOSApplication, store: bool) -> Result<PathBuf> {
        let developer = HostCli::prepare_xcode(
            if store { "iphoneos" } else { application.sdk() },
            if store { 26 } else { 0 },
        )?;
        println!("Checking Rust libraries for {}", application.target());
        application.prepare_target(application.target())?;
        Ok(developer)
    }

    pub fn notifications(application: &mut IOSApplication, team: &str, sandbox: bool) -> Result<()> {
        let manifest = Manifest::new(application.package().manifest_path.as_std_path())?;
        if NotificationCli::enabled(&manifest)? {
            let setup = application.notification_setup(team, sandbox)?;
            let id = NotificationCli::configure(&setup)?;
            application.use_notifications(setup, id)?;
        }
        Ok(())
    }
}
