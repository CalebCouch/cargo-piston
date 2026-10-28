use clap::{Parser, ValueEnum};
use std::fmt;
use std::path::PathBuf;
use crate::Result;
use crate::{android, developer, host, ios, linux};
use crate::project::Project;
use crate::cli::ApplicationArgs;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Platform { Macos, Ios, Android, Linux, }

#[derive(Debug)]
pub enum Error { NotificationSetup, Device, VirtualMachine }

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotificationSetup =>
                "Notification --setup requires macOS or a physical iOS device.",
            Self::Device => "Use --device or --simulator with dev ios or dev android.",
            Self::VirtualMachine => "Use --vm, --lima, and --lima-home with dev linux.",
        })
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, Parser)]
pub struct DevelopmentArgs {
    /// Platform to develop for. Omit to run natively on the host.
    #[arg(value_enum)]
    pub platform: Option<Platform>,
    #[command(flatten)]
    pub application: ApplicationArgs,
    /// Import or replace the app's APNs credentials in its AIR developer contract (Apple only).
    #[arg(long)]
    pub setup: bool,
    /// Phone identifier/name, Android serial, or simulator/AVD name.
    #[arg(long)]
    pub device: Option<String>,
    /// Use a simulator/emulator even when a phone is connected.
    #[arg(long)]
    pub simulator: bool,
    /// Existing Lima VM to use for Linux development; saved in Cargo.toml.
    #[arg(long)]
    pub vm: Option<String>,
    /// Lima executable, if it is not on PATH; saved in Cargo.toml.
    #[arg(long)]
    pub lima: Option<PathBuf>,
    /// Lima state directory; saved in Cargo.toml.
    #[arg(long)]
    pub lima_home: Option<PathBuf>,
    /// Arguments passed to the application.
    #[arg(last = true)]
    pub app_args: Vec<String>,
}

pub struct Cli;

impl Cli {
    pub fn start(mut args: DevelopmentArgs) -> Result<()> {
        if args.setup && (args.simulator
            || matches!(args.platform, Some(Platform::Android | Platform::Linux)))
        {
            return Err(Error::NotificationSetup.into());
        }
        if (args.device.is_some() || args.simulator)
            && !matches!(args.platform, Some(Platform::Ios | Platform::Android))
        {
            return Err(Error::Device.into());
        }
        if (args.vm.is_some() || args.lima.is_some() || args.lima_home.is_some())
            && !matches!(args.platform, Some(Platform::Linux))
        {
            return Err(Error::VirtualMachine.into());
        }
        let manifest = Project::manifest(&args.application.cargo)?;
        let (_, team) = developer::Cli::credentials(&manifest, args.application.team.as_deref(), false)?;
        args.application.team = Some(team);
        match args.platform {
            None => host::Cli::develop(args),
            Some(Platform::Macos) => host::Cli::develop_macos(args),
            Some(Platform::Ios) => ios::Cli::start(args),
            Some(Platform::Android) => android::Cli::start(args),
            Some(Platform::Linux) => linux::Cli::start(args),
        }
    }
}
