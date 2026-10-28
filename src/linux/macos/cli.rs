use std::fmt;
use std::path::{Path, PathBuf};
use crate::Result;
use crate::{host, linux};
use crate::dev::DevelopmentArgs;
use crate::linux::macos::LinuxDevelopment;
use crate::project::Project;

#[derive(Debug)]
pub enum Error {
    LinuxVmHomeAccess { path: PathBuf, source: std::io::Error },
    LinuxVmHomeInvalid { path: PathBuf },
    ManifestStringRequired { path: PathBuf, key: String },
    LinuxVmNameInvalid { name: String },
    LinuxVmHomeMissing,
    LinuxVmLocked { name: String, source: Box<dyn std::error::Error> },
    LinuxVmSignalHandler(ctrlc::Error),
    LinuxVmUnavailable { name: String, state: Option<String> },
    LinuxVmTargetMissing,
    LinuxVmTargetMismatch { actual: String, requested: String },
    LinuxVmDesktopTimeout,
    LinuxVmStopFailed { name: String, home: PathBuf },
    LinuxVmInterrupted,
    WritePlUtil(String),
    Shutdown { source: Box<dyn std::error::Error>, cleanup: Box<dyn std::error::Error> },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LinuxVmHomeAccess { path, source } => write!(f,
                "Cannot use Linux VM storage directory {}: {source}", path.display()),
            Self::LinuxVmHomeInvalid { path } => write!(f,
                "Saved Linux VM storage directory {} is unavailable. \
                Mount its drive or choose an existing directory.",
                path.display()),
            Self::ManifestStringRequired { path, key } => write!(f,
                "package.metadata.piston.{key} in {} must be a string.", path.display()),
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
            Self::WritePlUtil(err) => write!(f, "Failed to write Plutil contents: {}", err),
            Self::Shutdown { source, cleanup } => write!(f, "{source}\n{cleanup}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LinuxVmHomeAccess { source, .. } => Some(source),
            Self::LinuxVmLocked { source, .. } => Some(source.as_ref()),
            Self::LinuxVmSignalHandler(source) => Some(source),
            Self::Shutdown { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl linux::Cli {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        let manifest = Project::manifest(&args.application.cargo)?;
        let mut development = loop {
            match LinuxDevelopment::new(args.clone()) {
                Ok(development) => break development,
                Err(error) if args.vm.is_none()
                    && matches!(error.downcast_ref::<Error>(), Some(Error::LinuxVmNameInvalid { .. })) =>
                {
                    eprintln!("{error}");
                    let value = loop {
                        let value = host::Cli::prompt("Linux VM name", "")?;
                        match LinuxDevelopment::validate_name(&value) {
                            Ok(()) => break value,
                            Err(error) => eprintln!("{error}"),
                        }
                    };
                    manifest.save(&["linux"], &[("vm", value)])?;
                }
                Err(error) if args.lima_home.is_none() && matches!(error.downcast_ref::<Error>(),
                    Some(Error::LinuxVmHomeInvalid { .. } | Error::LinuxVmHomeAccess { .. }
                        | Error::ManifestStringRequired { .. })) =>
                {
                    eprintln!("{error}");
                    let value = loop {
                        let value = host::Cli::prompt("Linux VM storage directory (absolute path)", "")?;
                        match LinuxDevelopment::validate_home(Path::new(&value)) {
                            Ok(()) => break value,
                            Err(error) => eprintln!("{error}"),
                        }
                    };
                    manifest.save(&["linux"], &[("lima_home", value)])?;
                }
                Err(error) => return Err(error),
            }
        };
        println!("Preparing Linux development VM {}", development.name());
        let result = (|| {
            if development.viewer_needs_build() {
                let developer = host::Cli::prepare_xcode("macosx", 0)?;
                development.compile_viewer(&developer)?;
            }
            development.prepare_viewer("Piston Linux")?;
            development.start_vm()?;
            println!("Preparing Linux build and desktop dependencies");
            development.prepare_guest()?;
            println!("Synchronizing application and local dependencies");
            development.sync()?;
            println!("Building the Linux application");
            let executable = development.compile(|message| eprint!("{message}"))?;
            println!("Opening Linux VM. Close its window or press Ctrl-C to stop the VM.");
            development.show(&executable, "Piston Linux — ", "Close Linux VM", "Quit")
        })();
        if development.owns_vm() {
            println!("Stopping Linux development VM {}", development.name());
        }
        let stopped = development.stop();
        match (result, stopped) {
            (Err(error), Err(cleanup)) => {
                Err(Error::Shutdown { source: error, cleanup }.into())
            }
            (Err(error), _) | (_, Err(error)) => Err(error),
            _ => Ok(()),
        }
    }
}
