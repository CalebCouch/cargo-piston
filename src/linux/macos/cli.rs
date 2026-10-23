use crate::Result;
use crate::dev::DevelopmentArgs;
use crate::host::cli::HostCli;
use crate::linux::macos::LinuxDevelopment;
use crate::linux::cli::LinuxCli;

impl LinuxCli {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        let mut development = LinuxDevelopment::new(args)?;
        println!("Preparing Linux development VM {}", development.name());
        let result = (|| {
            if development.viewer_needs_build() {
                let developer = HostCli::prepare_xcode("macosx", 0)?;
                development.compile_viewer(&developer)?;
            }
            development.prepare_viewer()?;
            development.start_vm()?;
            println!("Preparing Linux build and desktop dependencies");
            development.prepare_guest()?;
            println!("Synchronizing application and local dependencies");
            development.sync()?;
            println!("Building the Linux application");
            let executable = development.compile(|message| eprint!("{message}"))?;
            println!("Opening Linux VM. Close its window or press Ctrl-C to stop the VM.");
            development.show(&executable)
        })();
        if development.owns_vm() {
            println!("Stopping Linux development VM {}", development.name());
        }
        let stopped = development.stop();
        match (result, stopped) {
            (Err(error), Err(cleanup)) => {
                Err(crate::PistonError::Generic(format!("{error}\n{cleanup}")))
            }
            (Err(error), _) | (_, Err(error)) => Err(error),
            _ => Ok(()),
        }
    }
}
