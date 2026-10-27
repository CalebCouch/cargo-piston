use std::path::PathBuf;
use crate::Result;
use super::{AppStoreRelease, ScreenshotFamily, StoreScreenshot};

#[derive(Debug)]
pub enum Error {
    StoreCatalogEmpty,
    StoreScreenshotsMissing,
    StoreScreenshotGroupCount { group: String, count: usize, maximum: usize },
    StoreScreenshotFileSize { path: PathBuf, size: u64, maximum: u64 },
    StoreScreenshotSync {
        source: Box<dyn std::error::Error>,
        recovery: Vec<(String, Box<dyn std::error::Error>)>,
    },
    StoreScreenshotGroupMismatch { images: Vec<(PathBuf, u32, u32)> },
    StoreVersionNotEditable { version: String, state: String },
    StoreUploadUrlsRejected,
    StoreUploadOperationsMissing { image_id: String },
    StoreUploadRangeInvalid,
    StoreUploadRangeTooLarge,
    StoreUploadRangeDiscontinuous,
    StoreUploadRangeIncomplete,
    StoreUploadInsecureUrl,
    StoreUploadAuthorization,
    StoreScreenshotUpload { image_id: String, status: Option<u16> },
    StoreImageProcessingTimeout { image_id: String },
    StoreImageSpecificationMismatch { image_id: String, group: String, spec: String },
    StoreImageRejected { image_id: String, state: String, details: serde_json::Value },
    StoreScreenshotOrder { group: String },
    StoreCatalogFeatureMissing,
    StoreCatalogPolicyMissing,
    StoreCatalogSpecificationsMissing,
    StoreCatalogCategoryRejected,
    StoreCatalogDimensionsInvalid,
    StoreCatalogSpecificationsUnavailable,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StoreCatalogEmpty => write!(f, "Apple returned an empty asset specification catalog."),
            Self::StoreScreenshotsMissing => write!(f,
                "Add at least one iPhone screenshot before deploying."),
            Self::StoreScreenshotGroupCount { group, count, maximum } => write!(f,
                "{group} has {count} screenshots; Apple allows at most {maximum}. \
                 Remove screenshots and rerun."),
            Self::StoreScreenshotFileSize { path, size, maximum } => write!(f,
                "{} is {size} bytes; Apple requires a nonempty image of at most {maximum} bytes.",
                path.display()),
            Self::StoreScreenshotSync { source, recovery } => {
                writeln!(f, "Screenshot synchronization failed: {source}")?;
                if recovery.is_empty() {
                    writeln!(f, "Previous screenshot placements and order were restored.")?;
                } else {
                    writeln!(f, "Could not fully restore previous placements:")?;
                    for (group, error) in recovery { writeln!(f, "{group}: {error}")?; }
                    writeln!(f, "Open the version's Media Manager in App Store Connect \
                        and check the order before retrying.")?;
                }
                write!(f, "Original image assets remain in Asset Library.")
            }
            Self::StoreScreenshotGroupMismatch { images } => {
                write!(f, "These screenshots do not fit one Apple device display group:")?;
                for (path, width, height) in images {
                    write!(f, " {} ({width}x{height})", path.display())?;
                }
                write!(f, ". Capture this device family's screenshots \
                    at the same supported resolution, then rerun.")
            }
            Self::StoreVersionNotEditable { version, state } => write!(f,
                "App Store version {version} is {state}, so Apple does not allow screenshot changes. \
                 Resolve its review state in App Store Connect, or increase package.version \
                 in Cargo.toml for a new release."),
            Self::StoreUploadUrlsRejected => write!(f,
                "Apple rejected freshly issued screenshot upload URLs. Check your network or proxy \
                 and retry later. Existing placements are unchanged."),
            Self::StoreUploadOperationsMissing { image_id } => write!(f,
                "Apple supplied no upload operations for image {image_id}."),
            Self::StoreUploadRangeInvalid => write!(f, "Apple returned an invalid upload byte range."),
            Self::StoreUploadRangeTooLarge => write!(f,
                "Apple's upload byte range exceeds the screenshot size."),
            Self::StoreUploadRangeDiscontinuous => write!(f,
                "Apple's upload ranges overlap or leave a gap."),
            Self::StoreUploadRangeIncomplete => write!(f,
                "Apple's upload ranges do not cover the screenshot."),
            Self::StoreUploadInsecureUrl => write!(f, "Apple returned an insecure asset upload URL."),
            Self::StoreUploadAuthorization => write!(f,
                "Unexpected authorization header in asset upload instructions."),
            Self::StoreScreenshotUpload { image_id, status } => {
                write!(f, "Screenshot {image_id} upload failed (")?;
                match status {
                    Some(status) => write!(f, "HTTP {status}")?,
                    None => write!(f, "network connection or timeout")?,
                }
                write!(f, "). Check your network and rerun cargo piston deploy to resume. \
                    Existing screenshot placements are unchanged.")
            }
            Self::StoreImageProcessingTimeout { image_id } => write!(f,
                "Apple is still processing screenshot {image_id}. Rerun cargo piston deploy later; \
                 it will reuse the uploaded image. Existing screenshot placements are unchanged."),
            Self::StoreImageSpecificationMismatch { image_id, group, spec } => write!(f,
                "Apple processed image {image_id} as specification {spec}, incompatible with {group}."),
            Self::StoreImageRejected { image_id, state, details } => write!(f,
                "Apple image {image_id} is {state}: {details}. \
                 Check this image in App Store Connect's Asset Library."),
            Self::StoreScreenshotOrder { group } => write!(f,
                "Apple returned a different screenshot order for {group}."),
            Self::StoreCatalogFeatureMissing => write!(f,
                "Apple's asset catalog has no App Store version feature."),
            Self::StoreCatalogPolicyMissing => write!(f,
                "Apple's asset catalog has no screenshot policy."),
            Self::StoreCatalogSpecificationsMissing => write!(f,
                "Apple's asset catalog has no screenshot specifications."),
            Self::StoreCatalogCategoryRejected => write!(f,
                "Apple's screenshot catalog rejects screenshot image assets."),
            Self::StoreCatalogDimensionsInvalid => write!(f,
                "Apple's screenshot specification has invalid dimensions."),
            Self::StoreCatalogSpecificationsUnavailable => write!(f,
                "Apple returned no usable iPhone or iPad screenshot specifications."),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StoreScreenshotSync { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

pub struct Cli;

impl Cli {
    pub fn validate(release: &AppStoreRelease, screenshots: &[StoreScreenshot]) -> Result<()> {
        let preserved = release.validate(screenshots)?;
        if !preserved.is_empty() {
            eprintln!(
                "Keeping existing screenshot groups {}. These can override Apple's scaled \
                 screenshots for those displays; review them in App Store Connect's Media Manager.",
                preserved.join(", ")
            );
        }
        Ok(())
    }
    pub fn upload(release: &mut AppStoreRelease, screenshots: &[StoreScreenshot]) -> Result<()> {
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
