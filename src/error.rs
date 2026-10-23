use image::ImageError;
use std::error::Error as StdError;
use std::fmt;
use std::io::Error as IoError;
use std::path::PathBuf;

#[derive(Debug)]
pub enum SetupPage { Url(String), XcodeAccounts(PathBuf), }

#[derive(Debug)]
pub enum PistonError {
    SetupRequired { error: Box<PistonError>, page: SetupPage },
    BuildError(String),
    CargoParseError(String),
    WriteFileError(String),
    FileFlushError(String),
    WriteImageError(ImageError),
    SaveImageError(String),
    ZigbuildMissingError(String),
    HomebrewMissingError(String),
    ParseManifestError(String),
    CreateManifestError(String),
    WriteManifestError(String),
    AndroidConfigError(String),
    ProtoLinkError(String),
    UnsupportedTargetError(String),
    XcodeInstallError(String),
    XcodeSelectInstallError(String),
    XcodeSelectPathingError(String),
    XcodeBuildError(String),
    IOSSdkMissingError(String),
    ADBDevicesError(String),
    ParseUTF8Error(String),
    XcrunDevicectlError(String),
    KeyChainUnlockError(String),
    KeyChainImportError(String),
    IntoJSONError(String),
    OpenSSLKeyGenError(String),
    OpenSSLCSRError(String),
    ReadCSRError(String),
    Base64DecodeError(String),
    ASCClientParseEncodingKeyError(String),
    ASCClientJWTEncodeError(String),
    ASCAuthenticationError(String),
    WritePlUtilError(String),
    XcrunInstallError(String),
    XcrunLaunchError(String),
    CodesignError(String),
    DeviceProvisionError(String),
    SecurityFindIdentityError(String),
    PlutilConvertError(String),
    ExtractAPKError(String),
    InstallAPKError(String),
    RunAPKError(String),
    WhoAmIError(String),
    KeyToolError(String),
    APKSignerError(String),
    UnsupportedOSError { os: String, target: String },
    CopyFileError { input_path: PathBuf, output_path: PathBuf, source: IoError },
    LipoError { first_binary: PathBuf, second_binary: PathBuf, source: String },
    MacOSIconError { input_path: PathBuf, output_path: PathBuf, source: IoError },
    ReadDirError { path: PathBuf, source: IoError },
    MapDirError { path: PathBuf, source: IoError },
    QueryProvisionError { path: PathBuf, source: IoError },
    RemoveSubdirError { path: PathBuf, source: IoError },
    RemoveFileError { path: PathBuf, source: IoError },
    CreateFileError { path: PathBuf, source: IoError },
    RenameFileError { path: PathBuf, source: IoError },
    CreateDirAllError { path: PathBuf, source: IoError },
    OpenImageError { path: PathBuf, source: ImageError },
    ASCClientUreqError { endpoint: String, e: String },
    Generic(String),
}

impl fmt::Display for PistonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SetupRequired { error, .. } => std::fmt::Display::fmt(error, f),
            Self::BuildError(err) => write!(f, "Failed to build the binary: {}", err),
            Self::CargoParseError(err) => write!(f, "Could not read the Cargo project:\n{err}"),
            Self::WriteFileError(err) => write!(f, "Failed to write file: {}", err),
            Self::WriteImageError(err) => write!(f, "Failed to write image: {}", err),
            Self::SaveImageError(err) => write!(f, "Failed to save image: {}", err),
            Self::FileFlushError(err) => write!(f, "Failed to flush file: {}", err),
            Self::ZigbuildMissingError(err) => {
                write!(f, "Failed to find zigbuild path in .env file: {}", err)
            }
            Self::HomebrewMissingError(err) => {
                write!(f, "Failed to find homebrew bin path in .env file: {}", err)
            }
            Self::ParseManifestError(err) => {
                write!(f, "Failed to parse package.metadata.android value: {}", err)
            }
            Self::CreateManifestError(err) => {
                write!(f, "Failed to create AndroidManifest.xml file: {}", err)
            }
            Self::WriteManifestError(err) => {
                write!(f, "Failed to write AndroidManifest.xml contents: {}", err)
            }
            Self::AndroidConfigError(err) => {
                write!(f, "Failed to read android config path from .env: {}", err)
            }
            Self::ProtoLinkError(err) => {
                write!(f, "Failed to proto link android build directory: {}", err)
            }
            Self::UnsupportedTargetError(err) => {
                write!(f, "User Specified Target is Unsupported: {}", err)
            }
            Self::XcodeInstallError(err) => write!(f, "Xcode Installation not Found: {}", err),
            Self::XcodeSelectInstallError(err) => {
                write!(f, "Xcode-select Installation verification failed: {}", err)
            }
            Self::XcodeSelectPathingError(err) => {
                write!(f, "Xcode-select pathing does not match expected value: {}", err)
            }
            Self::XcodeBuildError(err) => {
                write!(f, "Xcode could not complete the operation: {err}")
            }
            Self::IOSSdkMissingError(err) => {
                write!(f, "IOS SDK missing from xcodebuild sdks: {}", err)
            }
            Self::ADBDevicesError(err) => {
                write!(f, "Error Running 'ADB Devices', check installation and .env path: {}", err)
            }
            Self::ParseUTF8Error(err) => write!(f, "Error Parsing UTF8: {}", err),
            Self::XcrunDevicectlError(err) => f.write_str(err),
            Self::KeyChainUnlockError(err) => {
                write!(f, "Failed to unlock the security keychain: {}", err)
            }
            Self::KeyChainImportError(err) => {
                write!(f, "Failed to import profile to security keychain: {}", err)
            }
            Self::IntoJSONError(err) => write!(f, "Error converting into JSON: {}", err),
            Self::OpenSSLKeyGenError(err) => {
                write!(f, "Error Generating New Key with openssl: {}", err)
            }
            Self::OpenSSLCSRError(err) => write!(f, "Error Generating CSR with openssl: {}", err),
            Self::ReadCSRError(err) => write!(f, "Error Reading CSR: {}", err),
            Self::Base64DecodeError(err) => write!(f, "Error Decoding Base 64: {}", err),
            Self::ASCClientParseEncodingKeyError(err) => {
                write!(f, "Error parsing encoding key: {}", err)
            }
            Self::ASCAuthenticationError(err) => f.write_str(err),
            Self::ASCClientJWTEncodeError(err) => write!(f, "Error encoding JWT: {}", err),
            Self::WritePlUtilError(err) => write!(f, "Failed to write Plutil contents: {}", err),
            Self::XcrunInstallError(err) => {
                write!(f, "Error installing app bundle to target device via xcrun: {}", err)
            }
            Self::XcrunLaunchError(err) => {
                write!(f, "Error launching app bundle on target device via xcrun: {}", err)
            }
            Self::CodesignError(err) => f.write_str(err),
            Self::DeviceProvisionError(err) => {
                write!(f, "Error provisioning target device with idp: {}", err)
            }
            Self::SecurityFindIdentityError(err) => write!(
                f,
                "Error checking for local identities with `security find-identity -v -p codesigning`: {}",
                err
            ),
            Self::PlutilConvertError(err) => {
                write!(f, "Error using plutil to convert filetype`: {}", err)
            }
            Self::ExtractAPKError(err) => {
                write!(f, "Error extracting APK from AAB with bundletool: {}", err)
            }
            Self::InstallAPKError(err) => {
                write!(f, "Error Installing APK with bundletool: {}", err)
            }
            Self::RunAPKError(err) => write!(f, "Error Running APK with ADB: {}", err),
            Self::WhoAmIError(err) => write!(f, "Error running 'whoami': {}", err),
            Self::KeyToolError(err) => write!(f, "Error running 'keytool': {}", err),
            Self::APKSignerError(err) => write!(f, "Error running 'apksigner': {}", err),
            Self::UnsupportedOSError { os, target, .. } => {
                write!(f, "Host system: {:?} does not support the target: {:?}", os, target)
            }
            Self::LipoError { first_binary, second_binary, source } => write!(
                f,
                "Error making universal binary with Lipo first binary: {:?}, second binary: {:?}, source: {}",
                first_binary, second_binary, source
            ),
            Self::CopyFileError { input_path, output_path, .. } => {
                write!(f, "Failed to copy {:?} to {:?}", input_path, output_path)
            }
            Self::MacOSIconError { input_path, output_path, .. } => {
                write!(f, "Failed to format icon {:?} to {:?}", input_path, output_path)
            }
            Self::ReadDirError { path, .. } => write!(f, "Failed to read directory {:?}", path),
            Self::MapDirError { path, .. } => {
                write!(f, "Failed to map directory contents {:?}", path)
            }
            Self::QueryProvisionError { path, .. } => {
                write!(f, "Failed to query the security provision profile {:?}", path)
            }
            Self::RemoveSubdirError { path, .. } => {
                write!(f, "Failed to remove subdirectory {:?}", path)
            }
            Self::RemoveFileError { path, .. } => write!(f, "Failed to remove file {:?}", path),
            Self::CreateFileError { path, .. } => write!(f, "Failed to Create file {:?}", path),
            Self::RenameFileError { path, .. } => write!(f, "Failed to Rename file {:?}", path),
            Self::CreateDirAllError { path, .. } => {
                write!(f, "Failed to create dir all {:?}", path)
            }
            Self::OpenImageError { path, .. } => write!(f, "Failed to open image {:?}", path),
            Self::ASCClientUreqError { endpoint, e } => {
                write!(f, "ASC API error at endpoint: {:?}, Error message: {}", endpoint, e)
            }
            Self::Generic(err) => f.write_str(err),
        }
    }
}

impl StdError for PistonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CopyFileError { source, .. }
            | Self::MacOSIconError { source, .. }
            | Self::ReadDirError { source, .. }
            | Self::MapDirError { source, .. }
            | Self::QueryProvisionError { source, .. }
            | Self::RemoveSubdirError { source, .. }
            | Self::RemoveFileError { source, .. }
            | Self::CreateFileError { source, .. }
            | Self::CreateDirAllError { source, .. }
            | Self::RenameFileError { source, .. } => Some(source),
            Self::OpenImageError { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl PistonError {
    pub fn setup(self, page: SetupPage) -> Self {
        Self::SetupRequired { error: Box::new(self), page }
    }
}
