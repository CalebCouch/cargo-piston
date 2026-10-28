use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use crate::Result;
use super::{GPGSigner, LinuxBuilder};

#[derive(Debug)]
pub enum Error {
    AppImageTargetUnsupported { target: String },
    RuntimeMissing { path: PathBuf },
    Image(String),
    SigningInput(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AppImageTargetUnsupported { target } => write!(
                f, "Unsupported AppImage target: {target}. \
                    Supported architectures: x86_64, aarch64, arm, i686, ppc64le, riscv64.",
            ),
            Self::RuntimeMissing { path } => {
                write!(f, "AppImage runtime not found at {}", path.display())
            }
            Self::Image(source) => write!(f, "Cannot package AppImage: {source}"),
            Self::SigningInput(source) => {
                write!(f, "Cannot send the signing passphrase to GPG: {source}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SigningInput(source) => Some(source),
            _ => None,
        }
    }
}

pub struct Cli;

impl Cli {
    pub fn build(release: bool, appimage: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<()> {
        println!("building for linux");
        let op = LinuxBuilder::new(release, appimage, target, cwd, env_vars)?;
        println!("Preparing linux application resources");
        op.pre_build()?;
        println!("Compiling linux application");
        op.build()?;
        println!("Packaging Linux application");
        let bundle = op.post_build()?;
        if GPGSigner::gpg_valid(op.key_id.clone(), op.gpg_path.clone()) {
            println!("Signing Linux application");
            match GPGSigner::gpg_sign(
                op.key_id.clone(), op.key_pass.clone(), op.gpg_path.clone(), &bundle,
            ) {
                Ok(signature) => println!("Signature available at {}", signature.display()),
                Err(error) => eprintln!("Signing failed: {error}"),
            }
        }
        println!("Linux app bundle available at {}", bundle.display());
        Ok(())
    }
}
