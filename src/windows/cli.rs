use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::PathBuf;
use crate::Result;
use super::WindowsBuilder;

#[derive(Debug)]
pub enum Error {
    FileFlushError(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileFlushError(err) => write!(f, "Failed to flush file: {}", err),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self { Self::FileFlushError(source) => Some(source), }
    }
}

pub struct Cli;

impl Cli {
    pub fn build(release: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<()> {
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
