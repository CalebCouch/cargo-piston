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
            let directory = fs::canonicalize(&directory).map_err(|source| {
                PistonError::StoreScreenshotFolder { path: directory.clone(), source }
            })?;
            let entries = fs::read_dir(&directory).map_err(|source| {
                PistonError::ReadDirError { path: directory.clone(), source }
            })?;
            let mut paths = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|source| {
                    PistonError::MapDirError { path: directory.clone(), source }
                })?;
                if entry.file_name() == ".DS_Store" { continue; }
                let path = entry.path();
                let kind = entry.file_type().map_err(|error| {
                    PistonError::ReadFile { path: path.clone(), source: error }
                })?;
                if !kind.is_file() {
                    return Err(PistonError::StoreScreenshotNotFile {
                        path, directory: directory.clone(),
                    });
                }
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !matches!(extension.as_str(), "png" | "jpg" | "jpeg") {
                    return Err(PistonError::StoreScreenshotFormat { path });
                }
                paths.push(path);
            }
            paths.sort();
            if !(1..=10).contains(&paths.len()) {
                return Err(PistonError::StoreScreenshotCount {
                    path: directory, count: paths.len(),
                });
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
                return Err(PistonError::StoreScreenshotSizesMissing {
                    family: family.directory().into(),
                });
            }
            let directory = output.join(family.directory());
            fs::create_dir_all(&directory).map_err(|source| {
                PistonError::CreateDirAllError { path: directory.clone(), source }
            })?;
            let directory = fs::canonicalize(&directory).map_err(|source| {
                PistonError::ReadFile { path: directory.clone(), source }
            })?;
            if self.inputs.iter().any(|(_, path)| directory.starts_with(path.parent().unwrap())) {
                return Err(PistonError::StoreScreenshotOutputOverlap { path: directory });
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
            .map_err(|error| PistonError::ReadFile { path: source.into(), source: error })?;
        if !matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)) {
            return Err(PistonError::StoreScreenshotFormat { path: source.into() });
        }
        let mut decoder = reader.into_decoder().map_err(|error| {
            PistonError::StoreScreenshotDecode { path: source.into(), source: error }
        })?;
        let orientation = decoder.orientation().map_err(|error| {
            PistonError::StoreScreenshotOrientation { path: source.into(), source: error }
        })?;
        let profile = decoder.icc_profile().map_err(|error| {
            PistonError::StoreScreenshotColorProfile { path: source.into(), source: error }
        })?;
        let mut image = DynamicImage::from_decoder(decoder).map_err(|error| {
            PistonError::StoreScreenshotDecode { path: source.into(), source: error }
        })?;
        image.apply_orientation(orientation);
        if image.has_alpha() && image.to_rgba16().pixels().any(|pixel| pixel[3] != u16::MAX) {
            return Err(PistonError::StoreScreenshotTransparent { path: source.into() });
        }
        let original = (image.width(), image.height());
        let dimensions = if accepted.contains(&original) { Some(original) } else {
            accepted.iter().copied().filter(|&(width, height)| {
                width <= original.0 && height <= original.1
                    && u64::from(width) * u64::from(original.1)
                        == u64::from(height) * u64::from(original.0)
            }).max_by_key(|&(width, height)| u64::from(width) * u64::from(height))
        }.ok_or_else(|| PistonError::StoreScreenshotDimensions {
            path: source.into(), original, accepted: accepted.to_vec(),
        })?;
        let image = DynamicImage::ImageRgb8(image.into_rgb8());
        let image = if dimensions == original {
            image
        } else {
            image.resize_exact(dimensions.0, dimensions.1, FilterType::Lanczos3)
        };
        if path.symlink_metadata().is_ok() {
            fs::remove_file(&path).map_err(|source| {
                PistonError::RemoveFileError { path: path.clone(), source }
            })?;
        }
        let file = File::create_new(&path).map_err(|source| {
            PistonError::CreateFileError { path: path.clone(), source }
        })?;
        let mut writer = BufWriter::new(file);
        let mut encoder = PngEncoder::new(&mut writer);
        if let Some(profile) = profile.filter(|data| data.get(16..20) == Some(b"RGB ")) {
            encoder.set_icc_profile(profile).map_err(|error| {
                PistonError::StoreScreenshotColorProfile {
                    path: source.into(), source: image::ImageError::Unsupported(error),
                }
            })?;
        }
        image.write_with_encoder(encoder).map_err(|source| {
            PistonError::StoreScreenshotWrite { path: path.clone(), source }
        })?;
        writer.flush().map_err(|source| {
            PistonError::StoreScreenshotFlush { path: path.clone(), source }
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
