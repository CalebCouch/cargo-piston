use std::collections::HashMap;
use std::path::PathBuf;
use crate::PistonError;
use super::{GPGSigner, LinuxBuilder};

pub struct Cli;

impl Cli {
    pub fn build(release: bool, appimage: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<(), PistonError> {
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
