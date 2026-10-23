use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use crate::{PistonError, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenshotFamily { Iphone, Ipad, }

impl ScreenshotFamily {
    fn directory(self) -> &'static str {
        match self { Self::Iphone => "iphone", Self::Ipad => "ipad", }
    }
}

#[derive(Debug)]
pub struct StoreScreenshot {
    family: ScreenshotFamily,
    source: PathBuf,
    path: PathBuf,
    width: u32,
    height: u32,
}

impl StoreScreenshot {
    pub fn family(&self) -> ScreenshotFamily { self.family }
    pub fn source(&self) -> &Path { &self.source }
    pub fn path(&self) -> &Path { &self.path }
    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
}

pub struct IOSArtwork {
    inputs: Vec<(ScreenshotFamily, PathBuf)>,
}

impl IOSArtwork {
    pub fn new(package_root: &Path) -> Result<Self> {
        let mut inputs = Vec::new();
        for family in [ScreenshotFamily::Iphone, ScreenshotFamily::Ipad] {
            let directory = package_root.join("store/apple").join(family.directory());
            let directory = fs::canonicalize(&directory).map_err(|error| {
                PistonError::Generic(format!(
                    "Cannot open screenshot folder {}: {error}\n\
                     Add 1–10 PNG or JPEG screenshots for {} in this folder.",
                    directory.display(),
                    family.directory(),
                ))
            })?;
            let entries = fs::read_dir(&directory).map_err(|error| {
                PistonError::Generic(format!(
                    "Cannot read screenshot folder {}: {error}",
                    directory.display(),
                ))
            })?;
            let mut paths = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|error| {
                    PistonError::Generic(format!(
                        "Cannot read an entry in {}: {error}",
                        directory.display(),
                    ))
                })?;
                if entry.file_name() == ".DS_Store" { continue; }
                let path = entry.path();
                let kind = entry.file_type().map_err(|error| {
                    PistonError::Generic(format!("Cannot inspect {}: {error}", path.display()))
                })?;
                if !kind.is_file() {
                    return Err(PistonError::Generic(format!(
                        "Screenshot {} is not a regular file. Put PNG or JPEG files directly \
                         in {} without subfolders or symbolic links.",
                        path.display(),
                        directory.display(),
                    )));
                }
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !matches!(extension.as_str(), "png" | "jpg" | "jpeg") {
                    return Err(PistonError::Generic(format!(
                        "Unsupported screenshot file {}. Export it as PNG or JPEG \
                         (.png, .jpg, or .jpeg), or move it out of the screenshot folder.",
                        path.display(),
                    )));
                }
                paths.push(path);
            }
            paths.sort();
            if !(1..=10).contains(&paths.len()) {
                return Err(PistonError::Generic(format!(
                    "Screenshot folder {} contains {} images. Provide 1–10 screenshots for {}.",
                    directory.display(),
                    paths.len(),
                    family.directory(),
                )));
            }
            inputs.extend(paths.into_iter().map(|path| (family, path)));
        }
        Ok(Self { inputs })
    }

    pub fn prepare(&self, output: &Path, sizes: &[(ScreenshotFamily, u32, u32)]) -> Result<Vec<StoreScreenshot>> {
        let mut screenshots = Vec::new();
        for family in [ScreenshotFamily::Iphone, ScreenshotFamily::Ipad] {
            let mut accepted: Vec<_> = sizes
                .iter()
                .filter(|(kind, width, height)| *kind == family && *width > 0 && *height > 0)
                .map(|(_, width, height)| (*width, *height))
                .collect();
            accepted.sort_unstable();
            accepted.dedup();
            if accepted.is_empty() {
                return Err(PistonError::Generic(format!(
                    "Apple's screenshot catalog contains no accepted dimensions for {}. \
                     Refresh the catalog before preparing screenshots.",
                    family.directory(),
                )));
            }
            let directory = output.join(family.directory());
            fs::create_dir_all(&directory).map_err(|error| {
                PistonError::Generic(format!(
                    "Cannot create screenshot output folder {}: {error}",
                    directory.display(),
                ))
            })?;
            let directory = fs::canonicalize(&directory).map_err(|error| {
                PistonError::Generic(format!(
                    "Cannot open screenshot output folder {}: {error}",
                    directory.display(),
                ))
            })?;
            if self.inputs.iter().any(|(_, path)| directory.starts_with(path.parent().unwrap())) {
                return Err(PistonError::Generic(format!(
                    "Screenshot output {} overlaps the original screenshot folders. \
                     Choose an output folder outside store/apple/iphone and store/apple/ipad.",
                    directory.display(),
                )));
            }
            for (index, (_, source)) in
                self.inputs.iter().filter(|(kind, _)| *kind == family).enumerate()
            {
                let path = directory.join(format!("{:02}.png", index + 1));
                screenshots.push(Self::prepare_file(family, source, path, &accepted)?);
            }
        }
        Ok(screenshots)
    }
}

impl IOSArtwork {
    fn prepare_file(family: ScreenshotFamily, source: &Path, path: PathBuf, accepted: &[(u32, u32)]) -> Result<StoreScreenshot> {
        let reader = ImageReader::open(source)
            .and_then(|reader| reader.with_guessed_format())
            .map_err(|error| {
                PistonError::Generic(format!(
                    "Cannot read screenshot {}: {error}",
                    source.display()
                ))
            })?;
        if !matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)) {
            return Err(PistonError::Generic(format!(
                "Screenshot {} does not contain a PNG or JPEG image. \
                 Export it as PNG or JPEG; changing the filename extension is not enough.",
                source.display(),
            )));
        }
        let mut decoder = reader.into_decoder().map_err(|error| {
            PistonError::Generic(format!(
                "Cannot decode screenshot {}: {error}\nExport a new PNG or JPEG copy.",
                source.display(),
            ))
        })?;
        let orientation = decoder.orientation().map_err(|error| {
            PistonError::Generic(format!(
                "Cannot read screenshot orientation for {}: {error}",
                source.display(),
            ))
        })?;
        let profile = decoder.icc_profile().map_err(|error| {
            PistonError::Generic(format!(
                "Cannot read screenshot color profile for {}: {error}",
                source.display(),
            ))
        })?;
        let mut image = DynamicImage::from_decoder(decoder).map_err(|error| {
            PistonError::Generic(format!(
                "Cannot decode screenshot {}: {error}\nExport a new PNG or JPEG copy.",
                source.display(),
            ))
        })?;
        image.apply_orientation(orientation);
        if image.has_alpha() && image.to_rgba16().pixels().any(|pixel| pixel[3] != u16::MAX) {
            return Err(PistonError::Generic(format!(
                "Screenshot {} contains transparent pixels. Apple requires opaque screenshots. \
                 Flatten it against your intended background and export a new PNG or JPEG copy.",
                source.display(),
            )));
        }
        let original = (image.width(), image.height());
        let dimensions = if accepted.contains(&original) { Some(original) } else {
            accepted.iter().copied().filter(|&(width, height)| {
                width <= original.0 && height <= original.1
                    && u64::from(width) * u64::from(original.1)
                        == u64::from(height) * u64::from(original.0)
            }).max_by_key(|&(width, height)| u64::from(width) * u64::from(height))
        }.ok_or_else(|| {
            let choices = accepted.iter().map(|(width, height)| format!("{width}×{height}"))
                .collect::<Vec<_>>().join(", ");
            PistonError::Generic(format!(
                "Screenshot {} is {}×{} pixels and cannot be prepared without upscaling \
                 or changing its aspect ratio. Capture or export a {} screenshot in one \
                 of Apple's accepted sizes: {choices}. Piston does not crop or stretch screenshots.",
                source.display(), original.0, original.1, family.directory(),
            ))
        })?;
        let image = DynamicImage::ImageRgb8(image.into_rgb8());
        let image = if dimensions == original {
            image
        } else {
            image.resize_exact(dimensions.0, dimensions.1, FilterType::Lanczos3)
        };
        if path.symlink_metadata().is_ok() {
            fs::remove_file(&path).map_err(|error| {
                PistonError::Generic(format!(
                    "Cannot replace prepared screenshot {}: {error}",
                    path.display(),
                ))
            })?;
        }
        let file = File::create_new(&path).map_err(|error| {
            PistonError::Generic(format!(
                "Cannot create prepared screenshot {}: {error}",
                path.display(),
            ))
        })?;
        let mut writer = BufWriter::new(file);
        let mut encoder = PngEncoder::new(&mut writer);
        if let Some(profile) = profile.filter(|data| data.get(16..20) == Some(b"RGB ")) {
            encoder.set_icc_profile(profile).map_err(|error| {
                PistonError::Generic(format!(
                    "Cannot preserve the color profile from {}: {error}",
                    source.display(),
                ))
            })?;
        }
        image.write_with_encoder(encoder).map_err(|error| {
            PistonError::Generic(format!(
                "Cannot save prepared screenshot {}: {error}",
                path.display(),
            ))
        })?;
        writer.flush().map_err(|error| {
            PistonError::Generic(format!(
                "Cannot finish prepared screenshot {}: {error}",
                path.display(),
            ))
        })?;
        Ok(StoreScreenshot {
            family,
            source: source.to_path_buf(),
            path,
            width: dimensions.0,
            height: dimensions.1,
        })
    }
}
