use std::path::Path;
use crate::{host, linux};
use crate::{PistonError, Result};
use crate::dev::DevelopmentArgs;
use crate::linux::macos::LinuxDevelopment;
use crate::project::Project;

impl linux::Cli {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        let manifest = Project::manifest(&args.application.cargo)?;
        let mut development = loop {
            match LinuxDevelopment::new(args.clone()) {
                Ok(development) => break development,
                Err(error @ PistonError::LinuxVmNameInvalid { .. }) if args.vm.is_none() => {
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
                Err(error @ (PistonError::LinuxVmHomeInvalid { .. }
                    | PistonError::LinuxVmHomeAccess { .. }
                    | PistonError::ManifestStringRequired { .. })) if args.lima_home.is_none() =>
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
                Err(crate::PistonError::Generic(format!("{error}\n{cleanup}")))
            }
            (Err(error), _) | (_, Err(error)) => Err(error),
            _ => Ok(()),
        }
    }
}
