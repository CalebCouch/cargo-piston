use std::collections::HashMap;
use std::path::PathBuf;
use crate::{cli, Result};
use crate::host;
use crate::dev::DevelopmentArgs;
use crate::devices::AndroidDevice;
use crate::project::Project;
use super::{AndroidBuilder, AndroidDevelopment};
use super::AndroidRunner;

#[derive(Debug)]
pub enum Error {
    AndroidArgumentsUnsupported,
    AndroidBridgeMissing,
    AndroidDeviceUnavailable { serial: String, state: String },
    AndroidDevicesAmbiguous { serials: Vec<String> },
    AndroidTargetIncompatible { serial: String, target: Option<String>, abis: Vec<String> },
    AndroidToolMissing { path: PathBuf },
    AndroidBuildToolsMissing { path: PathBuf },
    AndroidHostToolchainMissing { path: PathBuf },
    AndroidManifestMissing { path: PathBuf },
    AndroidSdkManagerMissing { path: PathBuf },
    AndroidEmulatorNotFound { selector: String, available: Vec<String> },
    AndroidEmulatorPortsUnavailable,
    AndroidEmulatorClosed { log: PathBuf },
    AndroidBootTimeout,
    AndroidInterrupted,
    AndroidAppExited,
    AndroidLaunchFailed { output: Vec<u8> },
    AndroidEmulatorControl(std::io::Error),
    AndroidSignalHandler(ctrlc::Error),
    ParseManifest(String),
    WriteManifest(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AndroidArgumentsUnsupported => write!(
                f, "Android NativeActivity does not accept desktop command-line arguments.",
            ),
            Self::AndroidBridgeMissing => write!(
                f, "The selected Maverick dependency needs its android/ Gradle bridge. \
                    Use maverick_os 3.0.3 or later.",
            ),
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
            Self::AndroidLaunchFailed { output } => f.write_str(&String::from_utf8_lossy(output)),
            Self::AndroidEmulatorControl(source) => {
                write!(f, "Cannot stop or wait for the Android emulator: {source}")
            }
            Self::AndroidSignalHandler(source) => write!(f, "Cannot watch Android shutdown: {source}"),
            Self::ParseManifest(err) => {
                write!(f, "Failed to parse package.metadata.android value: {}", err)
            }
            Self::WriteManifest(err) => {
                write!(f, "Failed to write AndroidManifest.xml contents: {}", err)
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AndroidEmulatorControl(source) => Some(source),
            Self::AndroidSignalHandler(source) => Some(source),
            _ => None,
        }
    }
}

pub struct Cli;

impl Cli {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        let manifest = Project::manifest(&args.application.cargo)?;
        let mut session = loop {
            match AndroidDevelopment::new(&args) {
                Ok(session) => break session,
                Err(error) if args.application.bundle_id.is_none()
                    && matches!(error.downcast_ref::<cli::Error>(),
                        Some(cli::Error::InvalidBundleId { .. })) =>
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
