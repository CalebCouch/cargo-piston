use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use crate::apple::xcode;
use crate::host;
use crate::ios;
use crate::Result;
use super::IOSStore;

pub struct Cli;

impl Cli {
    pub fn build(store: &mut IOSStore<'_>, setup: bool) -> Result<PathBuf> {
        let signing = xcode::Cli::store(store.application.team(), &store.developer)?;
        store.application.remember_team(signing.team())?;
        ios::Cli::notifications(&mut store.application, signing.team(), false, setup)?;
        println!(
            "Building {} for the App Store (aarch64-apple-ios, release)",
            store.application.package().name
        );
        let bundle =
            store.application.build_store(&store.developer, signing.team(), &mut |message| {
                eprint!("{message}")
            })?;
        println!(
            "Archiving the App Store app. Log: {}",
            store.application.work().join("store/archive.log").display()
        );
        store.archive(&signing, &bundle)?;
        println!(
            "Exporting the signed app. Log: {}",
            store.application.work().join("store/export.log").display()
        );
        store.export()
    }

    pub fn upload(store: &IOSStore<'_>) -> Result<()> {
        println!(
            "Uploading to App Store Connect. Log: {}",
            store.application.work().join("store/upload.log").display()
        );
        store.upload()?;
        println!("Upload accepted by App Store Connect; Apple will process the build.");
        Ok(())
    }
}

impl Cli {
    pub fn failure(f: &mut fmt::Formatter<'_>, path: &Path, log: &str, status: &ExitStatus) -> fmt::Result {
        let lower = log.to_lowercase();
        let guidance = if lower.contains("redundant binary upload")
            || lower.contains("previously used build number")
            || lower.contains("bundle version must be higher")
            || lower.contains("bundle version has already been used")
            || lower.contains("already uploaded a build with build number")
        {
            "Apple already has this build number.\n\n\
             Rerun deployment to select a new number.\n\
             To replace a pending TestFlight upload, run:\n\
               cargo piston deploy testflight --new-build".into()
        } else if lower.contains("agreement")
            && (lower.contains("accept")
                || lower.contains("updated")
                || lower.contains("required")
                || lower.contains("expired"))
        {
            format!(
                "1. Ask the team's Account Holder to accept the current agreements:\n   {}\n\n\
                 2. Rerun deployment.",
                host::Cli::link("Apple Developer account", "https://developer.apple.com/account"),
            )
        } else if lower.contains("401")
            || lower.contains("unauthorized")
            || lower.contains("invalid api key")
            || lower.contains("invalid authentication")
            || lower.contains("unable to authenticate")
            || lower.contains("unable to load authentication key")
        {
            format!(
                "1. Check the active team API key, Key ID, and Issuer ID:\n   {}\n\n\
                 2. Rerun your deploy command with --setup to replace its credentials.",
                host::Cli::link(
                    "App Store Connect API keys",
                    "https://appstoreconnect.apple.com/access/integrations/api",
                ),
            )
        } else if lower.contains("sdk version issue")
            || lower.contains("unsupported sdk")
            || lower.contains("unsupported toolchain")
            || lower.contains("must be built with")
            || lower.contains("sdk or later")
        {
            format!(
                "1. Download a compatible Xcode release:\n   {}\n\n\
                 2. If DEVELOPER_DIR is set, point it to the updated Xcode.\n\n\
                 3. Rerun deployment. Piston selects a compatible installed version.",
                host::Cli::link(
                    "Xcode downloads", "https://developer.apple.com/download/all/?q=Xcode",
                ),
            )
        } else if lower.contains("team has no devices")
            || lower.contains("no devices from which to generate")
        {
            "Xcode needs a development profile for the archive step.\n\n\
             1. Connect an iPhone and run cargo piston dev ios --team <TEAM_ID>.\n\n\
             2. Retry deployment with the same team.\n\n\
             You can also select a team that already has an eligible profile.\n\
             The final App Store export uses distribution signing.".into()
        } else if lower.contains("push notifications") || lower.contains("aps-environment") {
            format!(
                "1. Enable Push Notifications for this bundle ID:\n   {}\n\n\
                 2. Confirm the APNs key matches the team, app topic, and production environment.\n\n\
                 3. Rerun deployment to regenerate the signing profile.",
                host::Cli::link(
                    "Apple app identifiers",
                    "https://developer.apple.com/account/resources/identifiers/list",
                ),
            )
        } else if lower.contains("no profiles for")
            || lower.contains("provisioning profile")
            || lower.contains("signing certificate")
            || lower.contains("no account for team")
            || lower.contains("could not find team")
            || lower.contains("not a member")
            || lower.contains("permission")
            || lower.contains("not authorized")
        {
            "1. Confirm --team selects the paid Developer Program team that owns the API key.\n\n\
             2. Ask an Admin to check the key's role and access to certificates, identifiers,\n\
                profiles, and cloud-managed distribution signing.".into()
        } else {
            "Resolve the Xcode error below, then rerun deployment.\n\
             Apple's complete diagnostic is in the full log.".into()
        };
        let details = log
            .lines()
            .filter(|line| {
                let line = line.to_lowercase();
                line.contains("error:")
                    || line.contains("error itms")
                    || line.contains("error domain=")
            })
            .take(6)
            .map(|line| line.chars().take(500).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        write!(
            f, "App Store build failed ({status}).\n\n{guidance}\n\n{details}\n\nFull Xcode log: {}",
            path.display(),
        )
    }
}
