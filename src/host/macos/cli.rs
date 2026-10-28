use toml_edit::{Item, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use crate::Result;
use crate::apple::{notifications, xcode, PushBuild};
use crate::dev::DevelopmentArgs;
use crate::host::Host;
use crate::macos::MacOSBuilder;
use crate::manifest::Manifest;
use crate::project::Project;
use super::{apple, MacDevelopment};

#[derive(Debug)]
pub enum Error {
    MacResourcesInvalid,
    MacResourceInvalid { path: PathBuf },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MacResourcesInvalid => write!(f, "macos.resources must contain paths."),
            Self::MacResourceInvalid { path } => {
                write!(f, "Invalid macOS resource path: {}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {}

impl super::Cli {
    pub fn develop(args: DevelopmentArgs) -> Result<()> {
        let project = Project::new(args.application, "macos", false)?;
        Self::configure(&project)?;
        let developer = Self::prepare_xcode("macosx", 0)?;
        let mut session = MacDevelopment::new(project, args.app_args, developer)?;
        let manifest = Manifest::new(session.project().package.manifest_path.as_std_path())?;
        if notifications::Cli::enabled(&manifest)? {
            let team = manifest.setting(&["macos", "team"])?;
            let signing = xcode::Cli::development(
                session.project().args.team.as_deref()
                    .or(team.as_ref().and_then(Item::as_str)),
                session.developer(),
            )?;
            let setup = session.notification_setup(signing.team())?;
            let application = notifications::Cli::configure(&setup, args.setup)?;
            session.notifications(PushBuild { application, signing: setup.signing });
        }
        println!(
            "Building {} for macOS ({})", session.project().package.name,
            session.project().args.cargo.target.as_ref().unwrap(),
        );
        let executable =
            session.project().compile(session.build_command(), |message| eprint!("{message}"))?;
        let bundle = session.bundle(&executable)?;
        println!(
            "Opening {}. App output follows; press Ctrl-C to stop.",
            session.project().package.name
        );
        session.launch(&bundle)
    }

    pub fn develop_macos(args: DevelopmentArgs) -> Result<()> { Self::develop(args) }
    pub fn prepare_xcode(sdk: &str, minimum: u32) -> Result<PathBuf> {
        apple::Cli::prepare(sdk, minimum)
    }
    pub fn run_local(release: bool, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<()> {
        let developer = Self::prepare_xcode("macosx", 0)?;
        println!("Running for macos");
        Host::run_local(release, cwd, env_vars, Some(&developer))
    }

    fn configure(project: &Project) -> Result<()> {
        let manifest = Manifest::new(project.package.manifest_path.as_std_path())?;
        for (name, explanation) in [
            ("team", "10-character Apple Team ID"),
            ("bundle_id", "bundle identifier, such as com.example.app"),
            ("info_plist", "path to an existing Info.plist dictionary"),
            ("entitlements", "path to an existing entitlements plist dictionary"),
            ("resources", "TOML array of existing resource paths, such as [\"assets/logo.png\"]"),
        ] {
            if match name {
                "team" => project.args.team.is_some(),
                "bundle_id" => project.args.bundle_id.is_some(),
                "info_plist" => project.args.info_plist.is_some(),
                "entitlements" => project.args.entitlements.is_some(),
                _ => false,
            } { continue; }
            let mut current = manifest.setting(&["macos", name])?;
            let mut platform = "macos";
            if name == "bundle_id" && current.is_none() {
                current = manifest.setting(&["ios", name])?;
                platform = "ios";
            }
            let Some(current) = current else { continue; };
            if MacOSBuilder::valid_setting(name, &current, project.root()) { continue; }
            eprintln!("\nApp setting needs attention: {platform}.{name}\n\nEnter a {explanation}.");
            eprintln!("Current value: {current}");
            eprintln!("Or enter remove to use the default. Type q to cancel.\n");
            loop {
                let input = Self::prompt(name, "")?;
                if input == "remove" {
                    manifest.remove(&[platform], name)?;
                    if name == "bundle_id" && platform == "macos"
                        && let Some(fallback) = manifest.setting(&["ios", name])?
                        && !MacOSBuilder::valid_setting(name, &fallback, project.root()) {
                        platform = "ios";
                        eprintln!("The shared iOS bundle identifier also needs correction.");
                        eprintln!("Current value: {fallback}");
                        continue;
                    }
                    break;
                }
                let value = if name == "resources" {
                    match input.parse::<Value>() {
                        Ok(value) => value,
                        Err(error) => { eprintln!("Enter a TOML array of paths: {error}"); continue; }
                    }
                } else { Value::from(input) };
                if !MacOSBuilder::valid_setting(name, &Item::Value(value.clone()), project.root()) {
                    eprintln!("Enter a valid {explanation}.");
                    continue;
                }
                manifest.save(&[platform], &[(name, value)])?;
                break;
            }
        }
        Ok(())
    }
}
