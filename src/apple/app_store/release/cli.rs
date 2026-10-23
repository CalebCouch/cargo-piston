use crate::PistonError;
use super::{AppStoreRelease, ScreenshotFamily, StoreScreenshot};

pub struct ReleaseCli;

impl ReleaseCli {
    pub fn validate(release: &AppStoreRelease, screenshots: &[StoreScreenshot]) -> Result<(), PistonError> {
        for warning in release.validate(screenshots)? { eprintln!("{warning}"); }
        Ok(())
    }
    pub fn upload(release: &mut AppStoreRelease, screenshots: &[StoreScreenshot]) -> Result<(), PistonError> {
        Self::validate(release, screenshots)?;
        release.prepare_localization()?;
        let mut staged = Vec::new();
        for family in [ScreenshotFamily::Iphone, ScreenshotFamily::Ipad] {
            let images: Vec<_> =
                screenshots.iter().filter(|image| image.family() == family).collect();
            if images.is_empty() { continue; }
            let group = release.group(family, &images)?;
            eprintln!("Using Apple display group {} for {} screenshots", group.id, images.len());
            let mut asset_ids = Vec::new();
            for image in images {
                eprintln!("Preparing App Store screenshot {}", image.source().display());
                asset_ids.push(release.upload_image(image, group)?);
            }
            staged.push((group.id.clone(), asset_ids));
        }
        release.commit(&staged)?;
        eprintln!(
            "Selected App Store screenshot groups synchronized for {} ({})",
            release.version, release.locale
        );
        Ok(())
    }
}
