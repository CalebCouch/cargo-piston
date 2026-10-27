use std::path::PathBuf;

#[derive(Debug)]
pub enum Error {
    ApplePlistRead { path: PathBuf, source: plist::Error },
    ApplePlistWrite { path: PathBuf, source: plist::Error },
    ApplePlistJson(serde_json::Error),
    ApplePlistField { key: String },
    ApplePlistInput(std::io::Error),
    ApplePlistWait(std::io::Error),
    SigningCertificateMissing { name: String },
    SigningCertificateAmbiguous { name: String, teams: Vec<String> },
    PushIpaApplicationCount { count: usize },
    PushProfileMissing { topic: String, output: std::process::Output },
    PushTopicMismatch { topic: String },
    PushArchitectures { output: std::process::Output },
    PushEntitlementsRead { architecture: String, output: std::process::Output },
    PushEntitlementsParse(plist::Error),
    PushProfileNotDictionary,
    PushProfileEntitlementsMissing,
    PushEntitlementsNotDictionary,
    PushEnvironmentMismatch { key: &'static str, environment: &'static str, topic: String },
    PushTeamMismatch { team: String },
    PushProfileTopicMismatch { topic: String },
    PushProfileExpired,
    PlistNotDictionary { path: PathBuf },
    KeyChainUnlockError(String),
    KeyChainImportError(String),
    OpenSSLKeyGenError(String),
    OpenSSLCSRError(String),
    CodesignError(String),
    DeviceProvisionError(String),
    SecurityFindIdentityError(String),
    PlutilConvertError(String),
    QueryProvisionError { path: PathBuf, source: std::io::Error },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApplePlistRead { path, source } => {
                write!(f, "Cannot read property list {}: {source}", path.display())
            }
            Self::ApplePlistWrite { path, source } => {
                write!(f, "Cannot write property list {}: {source}", path.display())
            }
            Self::ApplePlistJson(source) => write!(f, "Invalid property list: {source}"),
            Self::ApplePlistField { key } => {
                write!(f, "The app Info.plist needs a nonempty {key} string.")
            }
            Self::ApplePlistInput(source) => {
                write!(f, "Cannot send the property list to plutil: {source}")
            }
            Self::ApplePlistWait(source) => write!(f, "Cannot wait for plutil: {source}"),
            Self::SigningCertificateMissing { name } => {
                write!(f, "No certificate matching '{name}' was found in Keychain.")
            }
            Self::SigningCertificateAmbiguous { name, teams } => write!(
                f, "Certificates named '{name}' belong to multiple teams: {}. Select a Team ID.",
                teams.join(", "),
            ),
            Self::PushIpaApplicationCount { count } => {
                write!(f, "The exported IPA must contain one app; found {count}.")
            }
            Self::PushProfileMissing { topic, output } => write!(
                f, "Push notifications require an embedded provisioning profile for {topic}. \
                 Enable Push Notifications for this App ID and rebuild with automatic signing.\n{}",
                String::from_utf8_lossy(&output.stderr).trim(),
            ),
            Self::PushTopicMismatch { topic } => {
                write!(f, "The app bundle ID does not match APNs topic {topic}.")
            }
            Self::PushArchitectures { output } => write!(
                f, "Cannot inspect the app's signed architectures: {}",
                String::from_utf8_lossy(&output.stderr).trim(),
            ),
            Self::PushEntitlementsRead { architecture, output } => write!(
                f, "Cannot inspect notification entitlements for {architecture}: {}",
                String::from_utf8_lossy(&output.stderr).trim(),
            ),
            Self::PushEntitlementsParse(source) => {
                write!(f, "Invalid signed entitlements: {source}")
            }
            Self::PushProfileNotDictionary => {
                write!(f, "Invalid provisioning profile dictionary.")
            }
            Self::PushProfileEntitlementsMissing => {
                write!(f, "The provisioning profile contains no entitlements.")
            }
            Self::PushEntitlementsNotDictionary => {
                write!(f, "Invalid signed entitlements dictionary.")
            }
            Self::PushEnvironmentMismatch { key, environment, topic } => write!(
                f, "The signed app and its provisioning profile must both grant \
                 {key}={environment}. The notification server uses that environment. Enable Push \
                 Notifications for {topic} and rebuild using the matching signing profile.",
            ),
            Self::PushTeamMismatch { team } => write!(
                f, "The signed app and APNs credentials must both belong to Apple team {team}.",
            ),
            Self::PushProfileTopicMismatch { topic } => write!(
                f, "Push signing requires an explicit provisioning profile matching APNs topic {topic}.",
            ),
            Self::PushProfileExpired => write!(f, "The push provisioning profile has expired."),
            Self::PlistNotDictionary { path } => {
                write!(f, "{} must contain a property-list dictionary.", path.display())
            }
            Self::KeyChainUnlockError(err) => {
                write!(f, "Failed to unlock the security keychain: {}", err)
            }
            Self::KeyChainImportError(err) => {
                write!(f, "Failed to import profile to security keychain: {}", err)
            }
            Self::OpenSSLKeyGenError(err) => {
                write!(f, "Error Generating New Key with openssl: {}", err)
            }
            Self::OpenSSLCSRError(err) => write!(f, "Error Generating CSR with openssl: {}", err),
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
            Self::QueryProvisionError { path, .. } => {
                write!(f, "Failed to query the security provision profile {:?}", path)
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ApplePlistRead { source, .. } => Some(source),
            Self::ApplePlistWrite { source, .. } => Some(source),
            Self::PushEntitlementsParse(source) => Some(source),
            Self::ApplePlistJson(source) => Some(source),
            Self::ApplePlistInput(source) => Some(source),
            Self::ApplePlistWait(source) => Some(source),
            Self::QueryProvisionError { source, .. } => Some(source),
            _ => None,
        }
    }
}
