use std::path::PathBuf;
use crate::host;

#[derive(Debug)]
pub enum Error {
    StoreTemporaryKey(std::io::Error),
    StoreKeyInvalid { key_id: String, source: jsonwebtoken::errors::Error },
    StoreKeyUnreadable { path: PathBuf, source: std::io::Error },
    StoreResponseField { field: String },
    StoreReleaseExists { requested: String, released: String },
    StoreBuildNumberOverflow,
    StoreResponseCollection { path: String },
    StorePaginationUrl { url: String },
    StorePaginationLimit,
    StoreResponseJson { method: String, path: String, source: std::io::Error },
    StoreRequestFailed { method: String, path: String, status: u16, response: serde_json::Value },
    StoreConnectionFailure { method: String, path: String, source: ureq::Transport },
    StoreHttp { method: &'static str, path: String, source: ureq::Error },
    StoreHttpStatus { method: &'static str, path: String, status: u16, response: String },
    StoreCertificateMissing { name: String },
    StoreVersionInvalid { version: String },
    StoreAppMissing { bundle_id: String, testflight: bool },
    StoreScreenshotFolder { path: PathBuf, source: std::io::Error },
    StoreScreenshotNotFile { path: PathBuf, directory: PathBuf },
    StoreScreenshotFormat { path: PathBuf },
    StoreScreenshotCount { path: PathBuf, count: usize },
    StoreScreenshotSizesMissing { family: String },
    StoreScreenshotOutputOverlap { path: PathBuf },
    StoreScreenshotDecode { path: PathBuf, source: image::ImageError },
    StoreScreenshotOrientation { path: PathBuf, source: image::ImageError },
    StoreScreenshotColorProfile { path: PathBuf, source: image::ImageError },
    StoreScreenshotTransparent { path: PathBuf },
    StoreScreenshotDimensions { path: PathBuf, original: (u32, u32), accepted: Vec<(u32, u32)> },
    StoreScreenshotWrite { path: PathBuf, source: image::ImageError },
    StoreScreenshotFlush { path: PathBuf, source: std::io::Error },
    Base64Decode(String),
    ASCClientJWTEncode(String),
    ASCAuthentication(Box<dyn std::error::Error>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StoreTemporaryKey(source) => write!(f,
                "Cannot prepare the temporary App Store Connect key for Xcode: {source}"),
            Self::StoreKeyInvalid { key_id, source } => write!(f,
                "Invalid App Store Connect private key {key_id}: {source}\n\n\
                 Rerun deployment with --setup and import the matching Team API key.\n\n{}",
                host::Cli::link("Team API keys", "https://appstoreconnect.apple.com/access/integrations/api")),
            Self::StoreKeyUnreadable { path, source } => write!(f,
                "Cannot read the API key at {}: {source}. Choose the downloaded .p8 file.",
                path.display()),
            Self::StoreResponseField { field } => write!(f, "Apple's response is missing {field}."),
            Self::StoreReleaseExists { requested, released } => write!(f,
                "Cannot upload iOS version {requested}: App Store Connect already released {released}. \
                 Choose a higher package.version for a new public release. An unreleased version can \
                 be reused with a new build number; a released version cannot."),
            Self::StoreBuildNumberOverflow => write!(f,
                "The iOS build number is too large to increment."),
            Self::StoreResponseCollection { path } => write!(f,
                "Apple returned an invalid collection for {path}."),
            Self::StorePaginationUrl { url } => write!(f,
                "Apple returned an unexpected pagination URL: {url}"),
            Self::StorePaginationLimit => write!(f,
                "Apple's response exceeded 100 pages; narrow the query."),
            Self::StoreResponseJson { method, path, source } => write!(f,
                "Apple returned invalid JSON for {method} {path}: {source}"),
            Self::StoreRequestFailed { method, path, status, response } => {
                write!(f, "App Store Connect {method} {path} failed (HTTP {status}).")?;
                for error in response["errors"].as_array().into_iter().flatten() {
                    write!(f, " {}: {}", error["code"].as_str().unwrap_or("API_ERROR"),
                        error["detail"].as_str().or(error["title"].as_str())
                            .unwrap_or("No detail provided"))?;
                }
                let help = match status {
                    401 => "Rerun deployment with --setup to replace the API credentials. \
                        Also check your Mac's clock.",
                    403 => "Use an API key with Admin or App Manager access to this app \
                        and confirm the team's agreements are current.",
                    409 => "Check the reported resource and its current state \
                        in App Store Connect, then rerun.",
                    429 => "Apple is rate limiting requests. Wait and rerun the command.",
                    _ => "Rerun after resolving this App Store Connect error.",
                };
                write!(f, "\n{help}")
            }
            Self::StoreConnectionFailure { method, path, source } => write!(f,
                "Could not reach App Store Connect for {method} {path}: {source}. Check your network \
                 and rerun. Apple may have received the request; \
                 check its status before retrying an upload."),
            Self::StoreHttp { method, path, source } => {
                write!(f, "App Store Connect {method} {path} failed: {source}")?;
                if path == "/v1/bundleIds" && matches!(source, ureq::Error::Status(409, _)) {
                    write!(f, ". Choose a new, unique bundle ID in Cargo.toml.")?;
                }
                Ok(())
            }
            Self::StoreHttpStatus { method, path, status, response } => write!(f,
                "App Store Connect {method} {path} failed (HTTP {status}): {response}"),
            Self::StoreCertificateMissing { name } => write!(f,
                "Could not find a Distribution certificate matching '{name}'."),
            Self::StoreVersionInvalid { version } => write!(f,
                "Cannot compare App Store version {version:?}; \
                 expected one to three numeric components."),
            Self::StoreAppMissing { bundle_id, testflight } => {
                let command = if *testflight {
                    "cargo piston deploy testflight"
                } else {
                    "cargo piston deploy"
                };
                write!(f, "No App Store Connect app is accessible for {bundle_id}.\n\n\
                    1. Open Apps and choose + > New App:\n   {}\n\n\
                    2. Select this bundle ID and enter the name, primary language, and SKU.\n\
                    3. Rerun {command}.\n\n\
                    Already created the app? Check the API key's team and access to it.",
                    host::Cli::link("App Store Connect apps", "https://appstoreconnect.apple.com/apps"))
            }
            Self::StoreScreenshotFolder { path, source } => write!(f,
                "Cannot open screenshot folder {}: {source}\n\
                 Add 1–10 PNG or JPEG screenshots in this folder.",
                path.display()),
            Self::StoreScreenshotNotFile { path, directory } => write!(f,
                "Screenshot {} is not a regular file. Put PNG or JPEG files directly in {} \
                 without subfolders or symbolic links.", path.display(), directory.display()),
            Self::StoreScreenshotFormat { path } => write!(f,
                "Unsupported screenshot {}. Export it as PNG or JPEG (.png, .jpg, or .jpeg); \
                 changing the filename extension is not enough.", path.display()),
            Self::StoreScreenshotCount { path, count } => write!(f,
                "Screenshot folder {} contains {count} images. Provide 1–10 screenshots.",
                path.display()),
            Self::StoreScreenshotSizesMissing { family } => write!(f,
                "Apple's screenshot catalog contains no accepted dimensions for {family}. \
                 Refresh the catalog before preparing screenshots."),
            Self::StoreScreenshotOutputOverlap { path } => write!(f,
                "Screenshot output {} overlaps the original screenshot folders. \
                 Choose an output folder outside store/apple/iphone and store/apple/ipad.",
                path.display()),
            Self::StoreScreenshotDecode { path, source } => write!(f,
                "Cannot decode screenshot {}: {source}\nExport a new PNG or JPEG copy.", path.display()),
            Self::StoreScreenshotOrientation { path, source } => write!(f,
                "Cannot read screenshot orientation for {}: {source}", path.display()),
            Self::StoreScreenshotColorProfile { path, source } => write!(f,
                "Cannot read or preserve screenshot color profile for {}: {source}", path.display()),
            Self::StoreScreenshotTransparent { path } => write!(f,
                "Screenshot {} contains transparent pixels. Apple requires opaque screenshots. \
                 Flatten it against your intended background and export a new PNG or JPEG copy.",
                path.display()),
            Self::StoreScreenshotDimensions { path, original, accepted } => {
                let choices = accepted.iter().map(|(width, height)| format!("{width}×{height}"))
                    .collect::<Vec<_>>().join(", ");
                write!(f, "Screenshot {} is {}×{} pixels and cannot be prepared without upscaling \
                    or changing its aspect ratio. Capture or export a screenshot in one of Apple's \
                    accepted sizes: {choices}. Piston does not crop or stretch screenshots.",
                    path.display(), original.0, original.1)
            }
            Self::StoreScreenshotWrite { path, source } => write!(f,
                "Cannot save prepared screenshot {}: {source}", path.display()),
            Self::StoreScreenshotFlush { path, source } => write!(f,
                "Cannot finish prepared screenshot {}: {source}", path.display()),
            Self::Base64Decode(err) => write!(f, "Error Decoding Base 64: {}", err),
            Self::ASCAuthentication(err) => err.fmt(f),
            Self::ASCClientJWTEncode(err) => write!(f, "Error encoding JWT: {}", err),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StoreTemporaryKey(source) => Some(source),
            Self::StoreKeyInvalid { source, .. } => Some(source),
            Self::StoreKeyUnreadable { source, .. } => Some(source),
            Self::StoreResponseJson { source, .. } => Some(source),
            Self::StoreScreenshotFolder { source, .. } => Some(source),
            Self::StoreScreenshotFlush { source, .. } => Some(source),
            Self::ASCAuthentication(source) => Some(source.as_ref()),
            Self::StoreConnectionFailure { source, .. } => Some(source),
            Self::StoreHttp { source, .. } => Some(source),
            Self::StoreScreenshotDecode { source, .. } => Some(source),
            Self::StoreScreenshotOrientation { source, .. } => Some(source),
            Self::StoreScreenshotColorProfile { source, .. } => Some(source),
            Self::StoreScreenshotWrite { source, .. } => Some(source),
            _ => None,
        }
    }
}
