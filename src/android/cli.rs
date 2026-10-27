use std::collections::HashMap;
use std::path::PathBuf;
use crate::{host, PistonError, Result};
use crate::dev::DevelopmentArgs;
use crate::devices::AndroidDevice;
use crate::project::Project;
use super::{AndroidBuilder, AndroidDevelopment};
use super::AndroidRunner;

pub struct Cli;

impl Cli {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        let manifest = Project::manifest(&args.application.cargo)?;
        let mut session = loop {
            match AndroidDevelopment::new(&args) {
                Ok(session) => break session,
                Err(error @ PistonError::InvalidBundleId { .. })
                    if args.application.bundle_id.is_none() =>
                {
                    eprintln!("{error}");
                    let value = loop {
                        let value = host::Cli::prompt("Android application ID", "")?;
                        match AndroidDevelopment::validate_bundle_id(&value) {
                            Ok(()) => break value,
                            Err(error) => eprintln!("{error}"),
                        }
                    };
                    manifest.save(&["android"], &[("bundle_id", value)])?;
                }
                Err(error) => return Err(error),
            }
        };
        let result = (|| {
            println!("Preparing Android tools");
            session.prepare()?;
            println!("Selecting an Android device or emulator");
            if !session.select_device(&args)? {
                println!("Opening an Android emulator");
                session.start_emulator(args.device.as_deref())?;
            }
            println!("Connecting to {}", session.serial());
            session.wait_for_boot()?;
            session.select_target()?;
            println!(
                "Building {} for {} ({})", session.project().package.name, session.serial(),
                session.project().args.cargo.target.as_ref().unwrap()
            );
            let library = session.compile()?;
            println!("Packaging Android app with Maverick's activity");
            let apk = session.package(&library)?;
            println!("Installing {}", session.package_id());
            session.install(&apk)?;
            println!(
                "Launching {}. App output follows; press Ctrl-C to stop.", session.package_id()
            );
            session.launch()
        })();
        let cleanup = session.stop();
        match (result, cleanup) {
            (Err(error), cleanup) => {
                if let Err(cleanup) = cleanup {
                    eprintln!("Android cleanup also failed: {cleanup}");
                }
                Err(error)
            }
            (Ok(()), cleanup) => cleanup,
        }
    }

    pub fn build(release: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<(PathBuf, String, String)> {
        println!("building for android");
        let op = AndroidBuilder::new(release, target, cwd, env_vars)?;

        println!("Preparing android application resources");
        op.pre_build()?;
        println!("Compiling and packaging Android app bundle");
        let aab_path = op.build()?;
        op.post_build(aab_path)?;

        Ok((op.output_path, op.app_name, op.manifest.package))
    }
    pub fn run(release: bool, cwd: PathBuf, env_vars: HashMap<String, String>, device: &AndroidDevice) -> Result<()> {
        println!("Running for Android");
        let target_string = "aarch64-linux-android".to_string();
        let env_vars_bind = env_vars.clone();
        let builder = Self::build(release, target_string, cwd.clone(), env_vars)?;

        AndroidRunner::deploy_usb(
            device.id.as_ref(),
            builder.0,
            builder.1,
            cwd,
            builder.2,
            env_vars_bind,
        )
    }
}
