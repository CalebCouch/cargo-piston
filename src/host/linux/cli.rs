use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use crate::{host, linux};
use crate::dev::DevelopmentArgs;
use crate::host::Host;
use crate::project::Project;
use crate::Result;

#[derive(Debug)]
pub enum Error {
    HostAndroidSdkMissing { path: PathBuf },
    HostJavaMissing { path: PathBuf, source: std::io::Error },
    HostBuildUnsupported,
    UnsupportedOS { os: String, target: String },
    MacDevelopmentUnsupported,
    NotificationSetupUnsupported,
    DevelopmentTargetInvalid,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
            Self::UnsupportedOS { os, target, .. } => {
                write!(f, "Host system: {:?} does not support the target: {:?}", os, target)
            }
            Self::MacDevelopmentUnsupported => write!(f, "macOS development requires a Mac."),
            Self::NotificationSetupUnsupported => write!(f,
                "Notification --setup requires macOS or a physical iOS device."),
            Self::DevelopmentTargetInvalid => write!(f,
                "Select a Linux Rust target for Linux development."),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::HostJavaMissing { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl super::Cli {
    pub fn develop(args: DevelopmentArgs) -> Result<()> { linux::Cli::start(args) }
    pub fn develop_macos(_args: DevelopmentArgs) -> Result<()> {
        Err(Error::MacDevelopmentUnsupported.into())
    }
    pub fn prepare_xcode(sdk: &str, _minimum: u32) -> Result<PathBuf> {
        Host::require_apple_tools(sdk)?;
        unreachable!()
    }
    pub fn run_local(release: bool, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<()> {
        println!("Running for linux");
        Host::run_local(release, cwd, env_vars, None)
    }
}

impl linux::Cli {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        if args.setup {
            return Err(Error::NotificationSetupUnsupported.into());
        }
        let project = Project::new(args.application, "linux", false)?;
        if project.args.cargo.target.as_deref().is_some_and(|target| !target.contains("-linux-")) {
            return Err(Error::DevelopmentTargetInvalid.into());
        }
        if let Some(target) = &project.args.cargo.target { Project::rust_target(target)?; }
        println!("Building {} for Linux", project.package.name);
        let executable =
            project.compile(project.build_command(), |message| eprint!("{message}"))?;
        println!("Launching {}. App output follows; press Ctrl-C to stop.", project.package.name);
        project.launch(&executable, &args.app_args)
    }
}
