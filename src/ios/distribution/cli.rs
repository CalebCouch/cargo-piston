use std::path::PathBuf;
use crate::Result;
use crate::apple::xcode::cli::SigningCli;
use crate::ios::cli::IOSCli;
use super::IOSStore;

pub struct DistributionCli;

impl DistributionCli {
    pub fn build(store: &mut IOSStore<'_>) -> Result<PathBuf> {
        let signing = SigningCli::store(store.application.team(), &store.developer)?;
        store.application.remember_team(signing.team())?;
        IOSCli::notifications(&mut store.application, signing.team(), false)?;
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
