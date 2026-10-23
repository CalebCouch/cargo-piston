use cargo_subcommand::Subcommand;
use clap::Parser;
use std::env;
use std::path::PathBuf;
use crate::android::cli::AndroidCli;
use crate::dev::{DevelopmentCli, Platform as DevPlatform};
use crate::devices::cli::DevicesCli;
use crate::host::Host;
use crate::host::cli::HostCli;
use crate::ios::cli::IOSCli;
use crate::ios::deployment::DeploymentCli;
use crate::linux::cli::LinuxCli;
use crate::macos::cli::MacCli;
use crate::project::Project;
use crate::windows::cli::WindowsCli;
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
            eprintln!("error: {error}");
            if let PistonError::SetupRequired { page, .. } = &error {
                match page {
                    crate::error::SetupPage::Url(url) => HostCli::open_url(url),
                    crate::error::SetupPage::XcodeAccounts(developer) => {
                        HostCli::open_accounts(developer);
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
        let Some(cmd) = cmd else { return HostCli::setup_identity(true); };
        match &cmd {
            PistonSubCmd::Setup => return HostCli::setup_identity(true),
            PistonSubCmd::Share => return HostCli::share_identity(),
            PistonSubCmd::Restore => return HostCli::restore_identity(),
            PistonSubCmd::Version | PistonSubCmd::ListDevices => (),
            PistonSubCmd::Dev(args) if matches!(
                args.platform, Some(DevPlatform::Android | DevPlatform::Linux),
            ) => (),
            PistonSubCmd::Build(args) if args.dry_run || !matches!(
                Platform::from_target(
                    args.common.subcommand_args.target.as_deref().unwrap_or(Host::default_target()),
                ),
                Platform::Ios | Platform::Macos,
            ) => (),
            PistonSubCmd::Run(args) if args.device.as_deref() == Some("android") || !matches!(
                Platform::from_target(
                    args.common.subcommand_args.target.as_deref().unwrap_or(Host::default_target()),
                ),
                Platform::Ios | Platform::Macos,
            ) => (),
            _ => HostCli::setup_identity(false)?,
        }

        if let PistonSubCmd::Dev(args) = cmd { return DevelopmentCli::start(args); }
        if let PistonSubCmd::Deploy(args) = cmd { return DeploymentCli::start(args); }
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
            | PistonSubCmd::Setup | PistonSubCmd::Share | PistonSubCmd::Restore => {
                unreachable!()
            }
            PistonSubCmd::Build(args) => {
                if args.dry_run {
                    println!("Dry run: would build with {:?}", args.common);
                    return Ok(());
                }
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
                match platform {
                    Platform::Android => {
                        println!(
                            "build orders received for Android targeting {target:?}, \
                            release is set to {release:?}"
                        );
                        AndroidCli::build(release_override, target_string, cwd, env_vars)?;
                    }
                    Platform::Ios => {
                        println!(
                            "build orders received for IOS targeting {target:?}, \
                            release is set to {release:?}"
                        );
                        IOSCli::build(release_override, target_string, cwd, env_vars, None)?;
                    }
                    Platform::Linux => {
                        println!(
                            "build orders received for Linux targeting {target:?}, \
                            release is set to {release:?}"
                        );
                        LinuxCli::build(release_override, appimage, target_string, cwd, env_vars)?;
                    }
                    Platform::Windows => {
                        println!(
                            "build orders received for Windows targeting {target:?}, \
                            release is set to {release:?}"
                        );
                        WindowsCli::build(release_override, target_string, cwd, env_vars)?;
                    }
                    Platform::Macos => {
                        println!(
                            "build orders received for Macos targeting {target:?}, \
                            release is set to {release:?}"
                        );
                        MacCli::build(release_override, external, target_string, cwd, env_vars)?;
                    }
                    Platform::Unknown => {
                        return Err(PistonError::Generic(format!(
                            "Unknown or unsupported target: {target:?}"
                        )));
                    }
                }
            }
            PistonSubCmd::Run(args) => {
                let cmd = Subcommand::new(args.common.subcommand_args).map_err(|error| {
                    PistonError::Generic(format!("Error parsing subcommand :{error}"))
                })?;
                let release = cmd.args().release;
                if let Some(device) = args.device {
                    let target = device.trim();
                    println!("run orders received for a target device: {device}");
                    let devices = DevicesCli::list(env_vars.clone(), true)?;
                    let android = devices.android.iter().find(|device| device.id == target);
                    let ios = devices.ios.iter().find(|device| device.id == target);
                    if target == "ios" && !devices.ios.is_empty() {
                        let device = &devices.ios[0];
                        println!("general IOS runner target: {device:?}");
                        IOSCli::run(release, cwd, env_vars, device)?;
                    } else if target == "android" && !devices.android.is_empty() {
                        let device = &devices.android[0];
                        println!("general Android runner target: {device:?}");
                        AndroidCli::run(release, cwd, env_vars, device)?;
                    } else if let Some(device) = android {
                        println!("explicit Android runner target: {android:?}");
                        AndroidCli::run(release, cwd, env_vars, device)?;
                    } else if let Some(device) = ios {
                        println!("explicit IOS runner target: {ios:?}");
                        IOSCli::run(release, cwd, env_vars, device)?;
                    } else {
                        return Err(PistonError::Generic("Device not found".into()));
                    }
                } else {
                    println!("run orders received with no device, run locally");
                    HostCli::run_local(release, cwd, env_vars)?;
                }
            }
            PistonSubCmd::ListDevices => {
                println!("list all available connected devices and relevant information");
                DevicesCli::list(env_vars, false)?;
            }
        }
        Ok(())
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
    /// Create or restore a developer identity for an Apple team.
    Setup,
    /// Select a team identity and print its private copy-and-paste transfer value.
    Share,
    /// Restore a developer identity from cargo piston share.
    Restore,
    /// Build and run on the host, a mobile device/emulator, or a Linux VM.
    Dev(crate::dev::DevelopmentArgs),
    /// Upload an iOS build to TestFlight or the App Store.
    Deploy(crate::ios::DeploymentArgs),
    #[clap(visible_alias = "b")]
    Build(BuildArgs),
    #[clap(visible_alias = "r")]
    Run(RunArgs),
    #[clap(visible_alias = "l")]
    ListDevices,
    #[clap(visible_alias = "v")]
    Version,
}

#[derive(clap::Subcommand)]
enum PistonCmd { Piston { #[clap(subcommand)] cmd: Option<PistonSubCmd>, }, }

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
