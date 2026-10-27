use std::collections::HashMap;
use std::path::PathBuf;
use crate::PistonError;
use super::WindowsBuilder;

pub struct Cli;

impl Cli {
    pub fn build(release: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<(), PistonError> {
        println!("Building for Windows");
        let op = WindowsBuilder::new(release, target, cwd, env_vars)?;
        println!("Preparing windows application resources");
        op.pre_build()?;
        println!("Compiling windows application");
        op.build()?;
        println!("Preparing final windows bundle");
        op.post_build()?;
        println!("App bundle available at {}", op.output_path.display());
        Ok(())
    }
}
