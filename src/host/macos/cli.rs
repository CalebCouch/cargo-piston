use std::collections::HashMap;
use std::path::PathBuf;
use crate::Result;
use crate::dev::DevelopmentArgs;
use crate::host::Host;
use crate::host::platform::apple::cli::XcodeCli;
use crate::macos::MacDevelopment;
use crate::host::cli::HostCli;

impl HostCli {
    pub fn develop(args: DevelopmentArgs) -> Result<()> { MacDevelopment::start(args) }
    pub fn develop_macos(args: DevelopmentArgs) -> Result<()> { Self::develop(args) }
    pub fn prepare_xcode(sdk: &str, minimum: u32) -> Result<PathBuf> {
        XcodeCli::prepare(sdk, minimum)
    }
    pub fn run_local(release: bool, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<()> {
        let developer = Self::prepare_xcode("macosx", 0)?;
        println!("Running for macos");
        Host::run_local(release, cwd, env_vars, Some(&developer))
    }
}
