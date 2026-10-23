use std::collections::HashMap;
use std::path::PathBuf;
use crate::Result;
use crate::devices::AndroidDevice;
use super::AndroidBuilder;
use super::AndroidRunner;

pub struct AndroidCli;

impl AndroidCli {
    pub fn build(release: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<(PathBuf, String, String)> {
        println!("building for android");
        let op = AndroidBuilder::new(release, target, cwd, env_vars)?;

        println!("Preparing android application resources");
        op.pre_build()?;
        println!("Compiling and packaging Android app bundle");
        let aab_path = op.build()?;
        op.post_build(aab_path)?;

        Ok((op.output_path, op.app_name, op.manifest.package))
    }
    pub fn run(release: bool, cwd: PathBuf, env_vars: HashMap<String, String>, device: &AndroidDevice) -> Result<()> {
        println!("Running for Android");
        let target_string = "aarch64-linux-android".to_string();
        let env_vars_bind = env_vars.clone();
        let builder = Self::build(release, target_string, cwd.clone(), env_vars)?;

        AndroidRunner::deploy_usb(
            device.id.as_ref(),
            builder.0,
            builder.1,
            cwd,
            builder.2,
            env_vars_bind,
        )
    }
}
