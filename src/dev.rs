use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use crate::android::AndroidDevelopment;
use crate::host::cli::HostCli;
use crate::ios::cli::IOSCli;
use crate::linux::cli::LinuxCli;
use crate::cli::ApplicationArgs;
use crate::{PistonError, Result};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Platform { Macos, Ios, Android, Linux, }

#[derive(Clone, Debug, Parser)]
pub struct DevelopmentArgs {
    /// Platform to develop for. Omit to run natively on the host.
    #[arg(value_enum)]
    pub platform: Option<Platform>,
    #[command(flatten)]
    pub application: ApplicationArgs,
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

pub struct DevelopmentCli;

impl DevelopmentCli {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        if (args.device.is_some() || args.simulator)
            && !matches!(args.platform, Some(Platform::Ios | Platform::Android))
        {
            return Err(PistonError::Generic(
                "Use --device or --simulator with dev ios or dev android.".into(),
            ));
        }
        if (args.vm.is_some() || args.lima.is_some() || args.lima_home.is_some())
            && !matches!(args.platform, Some(Platform::Linux))
        {
            return Err(PistonError::Generic(
                "Use --vm, --lima, and --lima-home with dev linux.".into(),
            ));
        }
        match args.platform {
            None => HostCli::develop(args),
            Some(Platform::Macos) => HostCli::develop_macos(args),
            Some(Platform::Ios) => IOSCli::start(args),
            Some(Platform::Android) => AndroidDevelopment::start(args),
            Some(Platform::Linux) => LinuxCli::start(args),
        }
    }
}
