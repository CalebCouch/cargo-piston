use std::collections::HashMap;
use std::path::PathBuf;
use crate::PistonError;
use super::LinuxBuilder;

pub struct LinuxCli;

impl LinuxCli {
    pub fn build(release: bool, appimage: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<(), PistonError> {
        println!("building for linux");
        let op = LinuxBuilder::new(release, appimage, target, cwd, env_vars)?;
        println!("Preparing linux application resources");
        op.pre_build()?;
        println!("Compiling linux application");
        op.build()?;
        for message in op.post_build()? { println!("{message}"); }
        Ok(())
    }
}
