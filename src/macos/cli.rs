use toml_edit::Item;
use std::collections::HashMap;
use std::path::PathBuf;
use crate::Result;
use crate::apple::{notifications, xcode, NotificationSetup, PushBuild, PushPlatform, PushSigning};
use crate::host::{self, Host};
use crate::manifest::Manifest;
use super::MacOSBuilder;

pub struct Cli;

impl Cli {
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

    fn notifications(op: &mut MacOSBuilder) -> Result<()> {
        let manifest = Manifest::new(&op.manifest)?;
        let developer = manifest.settings(&["developer"])?;
        let saved = manifest.setting(&["macos", "team"])?;
        let team = op.team_id.as_deref()
            .or(developer.get("team").map(String::as_str))
            .or(saved.as_ref().filter(|value| {
                MacOSBuilder::valid_setting("team", value, manifest.path().parent().unwrap())
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
