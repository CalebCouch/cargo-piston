use toml_edit::{Item, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use crate::apple::{notifications, xcode, NotificationSetup, PushBuild, PushPlatform, PushSigning};
use crate::dev::DevelopmentArgs;
use crate::host::{self, Host};
use crate::manifest::Manifest;
use crate::project::Project;
use crate::Result;
use super::{MacDevelopment, MacOSBuilder};

pub struct Cli;

impl Cli {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        let project = Project::new(args.application, "macos", false)?;
        Self::configure(&project)?;
        let developer = host::Cli::prepare_xcode("macosx", 0)?;
        let mut session = MacDevelopment::new(project, args.app_args, developer)?;
        let manifest = Manifest::new(session.project().package.manifest_path.as_std_path())?;
        if notifications::Cli::enabled(&manifest)? {
            let team = manifest.setting(&["macos", "team"])?;
            let signing = xcode::Cli::development(
                session
                    .project()
                    .args
                    .team
                    .as_deref()
                    .or(team.as_ref().and_then(toml_edit::Item::as_str)),
                session.developer(),
            )?;
            let setup = session.notification_setup(signing.team())?;
            let application = notifications::Cli::configure(&setup, args.setup)?;
            session.notifications(PushBuild { application, signing: setup.signing });
        }
        println!(
            "Building {} for macOS ({})",
            session.project().package.name,
            session.project().args.cargo.target.as_ref().unwrap()
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

    pub fn build(release: bool, external: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<()> {
        println!("building for MacOS");
        Host::require_apple_tools(&target)?;
        let developer = host::Cli::prepare_xcode("macosx", 0)?;
        let mut op = MacOSBuilder::new(release, external, target, cwd, env_vars, developer)?;
        if notifications::Cli::enabled(&Manifest::new(&op.manifest)?)? {
            Self::notifications(&mut op)?;
        }
        println!("Preparing macos application resources");
        op.pre_build()?;
        println!("Compiling macos application");
        op.build()?;
        if op.notifications.is_none() && (op.keystore_path.is_none() || op.asc_api_key.is_none()) {
            println!(
                "Keystore path or App Store Connect key missing; legacy automated signing is unavailable."
            );
        }
        println!("Packaging and signing the application");
        op.post_build()?;
        println!("macOS app bundle available at {}", op.output_path.display());
        Ok(())
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
            if MacDevelopment::valid_setting(name, &current, project.root()) { continue; }
            eprintln!("\nApp setting needs attention: {platform}.{name}\n\nEnter a {explanation}.");
            eprintln!("Current value: {current}");
            eprintln!("Or enter remove to use the default. Type q to cancel.\n");
            loop {
                let input = host::Cli::prompt(name, "")?;
                if input == "remove" {
                    manifest.remove(&[platform], name)?;
                    if name == "bundle_id" && platform == "macos" {
                        if let Some(fallback) = manifest.setting(&["ios", name])? {
                            if !MacDevelopment::valid_setting(name, &fallback, project.root()) {
                                platform = "ios";
                                eprintln!("The shared iOS bundle identifier also needs correction.");
                                eprintln!("Current value: {fallback}");
                                continue;
                            }
                        }
                    }
                    break;
                }
                let value = if name == "resources" {
                    match input.parse::<Value>() {
                        Ok(value) => value,
                        Err(error) => { eprintln!("Enter a TOML array of paths: {error}"); continue; }
                    }
                } else { Value::from(input) };
                if !MacDevelopment::valid_setting(name, &Item::Value(value.clone()), project.root()) {
                    eprintln!("Enter a valid {explanation}.");
                    continue;
                }
                manifest.save(&[platform], &[(name, value)])?;
                break;
            }
        }
        Ok(())
    }

    fn notifications(op: &mut MacOSBuilder) -> Result<()> {
        let manifest = Manifest::new(&op.manifest)?;
        let developer = manifest.settings(&["developer"])?;
        let saved = manifest.setting(&["macos", "team"])?;
        let team = op.team_id.as_deref()
            .or(developer.get("team").map(String::as_str))
            .or(saved.as_ref().filter(|value| {
                MacDevelopment::valid_setting("team", value, manifest.path().parent().unwrap())
            }).and_then(Item::as_str));
        let signing = if op.release {
            xcode::Cli::store(team, &op.developer)?
        } else {
            xcode::Cli::development(team, &op.developer)?
        };
        manifest.save(&["macos"], &[("team", signing.team())])?;
        let setup = NotificationSetup::new(
            &op.manifest,
            PushSigning {
                platform: PushPlatform::Macos,
                team: signing.team().into(),
                topic: op.bundle_id.clone(),
                sandbox: !op.release,
            },
        )?;
        let application = notifications::Cli::configure(&setup, false)?;
        op.notifications = Some(PushBuild { application, signing: setup.signing });
        Ok(())
    }
}
