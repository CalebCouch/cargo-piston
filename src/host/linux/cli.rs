use std::collections::HashMap;
use std::path::PathBuf;
use crate::linux;
use crate::dev::DevelopmentArgs;
use crate::host::Host;
use crate::project::Project;
use crate::{PistonError, Result};

impl super::Cli {
    pub fn develop(args: DevelopmentArgs) -> Result<()> { linux::Cli::start(args) }
    pub fn develop_macos(_args: DevelopmentArgs) -> Result<()> {
        Err(PistonError::Generic("macOS development requires a Mac.".into()))
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
            return Err(PistonError::Generic(
                "Notification --setup requires macOS or a physical iOS device.".into(),
            ));
        }
        let project = Project::new(args.application, "linux", false)?;
        if project.args.cargo.target.as_deref().is_some_and(|target| !target.contains("-linux-")) {
            return Err(PistonError::Generic(
                "Select a Linux Rust target for Linux development.".into(),
            ));
        }
        if let Some(target) = &project.args.cargo.target { Project::rust_target(target)?; }
        println!("Building {} for Linux", project.package.name);
        let executable =
            project.compile(project.build_command(), |message| eprint!("{message}"))?;
        println!("Launching {}. App output follows; press Ctrl-C to stop.", project.package.name);
        project.launch(&executable, &args.app_args)
    }
}
