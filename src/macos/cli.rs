use std::collections::HashMap;
use std::path::PathBuf;
use crate::Result;
use crate::apple::xcode::cli::SigningCli;
use crate::apple::{NotificationCli, PushBuild};
use crate::apple::{NotificationSetup, PushPlatform, PushSigning};
use crate::host::Host;
use crate::host::cli::HostCli;
use crate::manifest::Manifest;
use super::MacOSBuilder;

pub struct MacCli;

impl MacCli {
    pub fn build(release: bool, external: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<()> {
        println!("building for MacOS");
        Host::require_apple_tools(&target)?;
        let developer = HostCli::prepare_xcode("macosx", 0)?;
        let mut op = MacOSBuilder::new(release, external, target, cwd, env_vars, developer)?;
        if NotificationCli::enabled(&Manifest::new(&op.manifest)?)? {
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
        for message in op.post_build()? { println!("{message}"); }
        Ok(())
    }
    fn notifications(op: &mut MacOSBuilder) -> Result<()> {
        let manifest = Manifest::new(&op.manifest)?;
        let settings = manifest.settings(&["macos"])?;
        let team = op.team_id.as_deref().or(settings.get("team").map(String::as_str));
        let signing = if op.release {
            SigningCli::store(team, &op.developer)?
        } else {
            SigningCli::development(team, &op.developer)?
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
        let application = NotificationCli::configure(&setup)?;
        op.notifications = Some(PushBuild { application, signing: setup.signing });
        Ok(())
    }
}
