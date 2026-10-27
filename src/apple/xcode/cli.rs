use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use crate::host;
use crate::Result;
use super::XcodeSigning;

#[derive(Debug)]
pub enum Error {
    TeamSelectionRequired { teams: Vec<(String, String)> },
    TeamMenuWrite(io::Error),
    TeamSelectionRead(io::Error),
    TeamSelectionCancelled,
    StoreTeamMissing,
    DevelopmentTeamMissing,
    SigningExportMissing { path: PathBuf },
    SigningExportAppCount { count: usize },
    SigningBundleName { path: PathBuf },
    SigningExecutableMissing { path: PathBuf },
    SigningProvisionedAppMissing { bundle: PathBuf, log: PathBuf },
    SigningSignatureInvalid { output: std::process::Output, log: PathBuf },
    SigningAccountRejected { team: String, log: PathBuf },
    SigningAccountMissing { team: String, log: PathBuf },
    SigningAgreementRequired { team: String, log: PathBuf },
    SigningTeamUnavailable { team: String, log: PathBuf },
    SigningBundleUnavailable { team: String, log: PathBuf },
    SigningAppIdLimit { team: String, log: PathBuf },
    SigningFreeAppLimit { team: String, log: PathBuf },
    SigningDeviceLimit { team: String, log: PathBuf },
    SigningKeychainLocked { team: String, log: PathBuf },
    SigningCertificateUnavailable { team: String, log: PathBuf },
    SigningPushUnavailable { team: String, log: PathBuf },
    SigningProfileUnavailable { team: String, log: PathBuf },
    SigningBuildFailed { log: PathBuf, details: Vec<String> },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TeamSelectionRequired { teams } => {
                writeln!(f, "Choose an Xcode signing team:\n")?;
                for (index, (id, name)) in teams.iter().enumerate() {
                    writeln!(f, "  {}. {name} ({id})", index + 1)?;
                }
                write!(f, "\nRerun in an interactive terminal to use the menu.\n\
                    Or pass --team <TEAM_ID>.")
            }
            Self::TeamMenuWrite(source) => write!(f, "Cannot display the team menu: {source}"),
            Self::TeamSelectionRead(source) => write!(f, "Cannot read the team selection: {source}"),
            Self::TeamSelectionCancelled => write!(f, "Team selection cancelled."),
            Self::StoreTeamMissing => write!(f,
                "No App Store signing team is selected.\n\n\
                 1. Find the 10-character Team ID in Membership details:\n   {}\n\n\
                 2. Rerun with --team <TEAM_ID>.\n\n\
                 Use the paid Developer Program team associated with your App Store Connect key.",
                host::Cli::link("Apple Developer account", "https://developer.apple.com/account")),
            Self::DevelopmentTeamMissing => write!(f,
                "No Xcode signing team is configured.\n\n\
                 1. Open Xcode > Settings > Accounts.\n\
                    Add or select your Apple Account and finish sign-in.\n\n\
                 2. Confirm that your team appears, then rerun your dev command.\n\n\
                 If several teams appear, Piston will show a selection menu."),
            Self::SigningExportMissing { path } => {
                write!(f, "Xcode produced no app or installer in {}.", path.display())
            }
            Self::SigningExportAppCount { count } => write!(
                f, "The exported macOS installer must contain one matching app; found {count}.",
            ),
            Self::SigningBundleName { path } => {
                write!(f, "The app bundle needs a UTF-8 filename: {}", path.display())
            }
            Self::SigningExecutableMissing { path } => {
                write!(f, "The app executable is missing: {}", path.display())
            }
            Self::SigningProvisionedAppMissing { bundle, log } => write!(
                f, "Xcode did not produce a provisioned app at {}. See {}.",
                bundle.display(), log.display(),
            ),
            Self::SigningSignatureInvalid { output, log } => write!(
                f, "Invalid development signature: {}\nXcode log: {}",
                String::from_utf8_lossy(&output.stderr).trim(), log.display(),
            ),
            Self::SigningAccountRejected { team, log }
            | Self::SigningAccountMissing { team, log }
            | Self::SigningAgreementRequired { team, log }
            | Self::SigningTeamUnavailable { team, log }
            | Self::SigningBundleUnavailable { team, log }
            | Self::SigningAppIdLimit { team, log }
            | Self::SigningFreeAppLimit { team, log }
            | Self::SigningDeviceLimit { team, log }
            | Self::SigningKeychainLocked { team, log }
            | Self::SigningCertificateUnavailable { team, log }
            | Self::SigningPushUnavailable { team, log }
            | Self::SigningProfileUnavailable { team, log } => {
                let (cause, action) = match self {
                    Self::SigningAccountRejected { .. } => (
                        "Xcode could not sign in to your Apple Account.",
                        "Open Xcode > Settings > Accounts, select the Apple Account for the team \
                         below, and sign in again. Complete password or verification prompts.",
                    ),
                    Self::SigningAccountMissing { .. } => (
                        "Xcode has no Apple Account available for signing.",
                        "Open Xcode > Settings > Accounts, add your Apple Account, and complete \
                         sign-in. Confirm that the team below is listed for that account.",
                    ),
                    Self::SigningAgreementRequired { .. } => (
                        "Apple requires an updated developer agreement.",
                        "Open your Apple Developer account and review the agreement shown there.\n   \
                         For an organization, ask its Account Holder to accept it.",
                    ),
                    Self::SigningTeamUnavailable { .. } => (
                        "Xcode cannot use the selected development team.",
                        "Open Xcode > Settings > Accounts and confirm your account belongs to the \
                         team below and can sign apps. Select an available team with --team \
                         <TEAM_ID>, or ask that team's administrator to grant access.",
                    ),
                    Self::SigningBundleUnavailable { .. } => (
                        "Apple cannot register this bundle identifier for the selected team.",
                        "Choose a unique development identifier with --bundle-id \
                         <unique.identifier>, or select the team that owns the existing \
                         identifier with --team <TEAM_ID>.",
                    ),
                    Self::SigningAppIdLimit { .. } => (
                        "The Apple team has reached its App ID registration limit.",
                        "Reuse an app identifier registered to this team, wait until Apple's \
                         registration limit resets, or select another eligible team with \
                         --team <TEAM_ID>.",
                    ),
                    Self::SigningFreeAppLimit { .. } => (
                        "The iPhone has reached its limit for apps signed with free development \
                         profiles.",
                        "Choose an unused development app to remove from the iPhone, or use a \
                         team whose membership permits this installation. Keep the current \
                         bundle identifier when updating an app already installed.",
                    ),
                    Self::SigningDeviceLimit { .. } => (
                        "The Apple team has reached its device registration limit.",
                        "Ask the Account Holder to review the device limit in Apple Developer.\n   \
                         Or select another eligible team with --team <TEAM_ID>.\n   \
                         Re-pairing the phone does not reset the limit.",
                    ),
                    Self::SigningKeychainLocked { .. } => (
                        "Code signing could not access the signing key in Keychain.",
                        "Open Keychain Access and unlock the keychain containing your \
                         development certificate. When macOS asks whether codesign may use \
                         its private key, allow it.",
                    ),
                    Self::SigningCertificateUnavailable { .. } => (
                        "Xcode could not find a usable development signing certificate and \
                         private key.",
                        "Open Xcode > Settings > Accounts, select your account and team, and \
                         open Manage Certificates. Create an Apple Development certificate if \
                         needed, or import your existing certificate with its private key.",
                    ),
                    Self::SigningPushUnavailable { .. } => (
                        "Apple could not provision Push Notifications for this app.",
                        "Use an Apple Developer Program team with Push Notifications access. \
                         Open the Identifiers link below, select this app's bundle ID and enable \
                         Push Notifications. Confirm the APNs key belongs to the same team \
                         and permits this topic and environment, then rerun so Xcode can \
                         regenerate the provisioning profile.",
                    ),
                    Self::SigningProfileUnavailable { .. } => (
                        "Xcode could not obtain a development provisioning profile for this app.",
                        "Open Xcode > Settings > Accounts and confirm the team below is \
                         available. Verify that the bundle identifier belongs to that team \
                         and that it can register this device and the app's capabilities. \
                         Piston requests automatic provisioning; no distribution profile is needed.",
                    ),
                    _ => unreachable!(),
                };
                write!(f, "{cause}\n\nApple team: {team}\n\n\
                    1. {action}\n\n2. Rerun your Piston command with any corrected options.\n\n\
                    Xcode log: {}", log.display())?;
                if matches!(self, Self::SigningAgreementRequired { .. } | Self::SigningDeviceLimit { .. }) {
                    write!(f, "\n\n{}", host::Cli::link(
                        "Apple Developer account", "https://developer.apple.com/account",
                    ))?;
                }
                Ok(())
            }
            Self::SigningBuildFailed { log, details } => {
                writeln!(f, "Xcode failed while preparing the development app.\n")?;
                if details.is_empty() {
                    writeln!(f, "Xcode did not report a recognizable cause in its output.")?;
                } else {
                    writeln!(f, "{}", details.join("\n"))?;
                }
                write!(
                    f, "\n1. Open the Xcode log below and address the reported Xcode error.\n\
                     2. Rerun your dev command. Cargo will reuse its existing build cache.\n\n\
                     Xcode log: {}", log.display(),
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TeamMenuWrite(source) | Self::TeamSelectionRead(source) => Some(source),
            _ => None,
        }
    }
}

pub struct Cli;

impl Cli {
    pub fn development(team: Option<&str>, developer: &Path) -> Result<XcodeSigning> {
        Self::select(team, developer, SigningPurpose::Development)
    }

    pub fn store(team: Option<&str>, developer: &Path) -> Result<XcodeSigning> {
        Self::select(team, developer, SigningPurpose::Store)
    }
    fn select(team: Option<&str>, developer: &Path, purpose: SigningPurpose) -> Result<XcodeSigning> {
        if let Some(team) = team.filter(|team| !team.trim().is_empty()) {
            return Ok(XcodeSigning::new(team, developer));
        }

        let mut teams = XcodeSigning::teams()?;
        if teams.is_empty() { return Err(Self::missing_team(developer, purpose)); }
        let selected = if teams.len() == 1 {
            0
        } else {
            if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
                return Err(Error::TeamSelectionRequired { teams }.into());
            }
            let choices = teams
                .iter()
                .enumerate()
                .map(|(index, (id, name))| format!("  {}. {name} ({id})", index + 1))
                .collect::<Vec<_>>()
                .join("\n");
            eprintln!("Choose a signing team for this app:\n\n{choices}\n");
            loop {
                eprint!("Team [1-{}, q to cancel]: ", teams.len());
                io::stderr().flush().map_err(Error::TeamMenuWrite)?;
                let mut input = String::new();
                let read = io::stdin().read_line(&mut input).map_err(Error::TeamSelectionRead)?;
                if read == 0 || input.trim().eq_ignore_ascii_case("q") {
                    return Err(Error::TeamSelectionCancelled.into());
                }
                if let Some(index) = input
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|number| number.checked_sub(1))
                    .filter(|index| *index < teams.len())
                { eprintln!("Using {} ({})", teams[index].1, teams[index].0); break index; }
                eprintln!("Enter a number from 1 to {}, or q to cancel.", teams.len());
            }
        };
        Ok(XcodeSigning::new(&teams.remove(selected).0, developer))
    }
    fn missing_team(developer: &Path, purpose: SigningPurpose) -> Box<dyn std::error::Error> {
        if let SigningPurpose::Store = purpose { return Error::StoreTeamMissing.into(); }
        crate::cli::Error::SetupRequired {
            error: Error::DevelopmentTeamMissing.into(),
            page: crate::cli::SetupPage::XcodeAccounts(developer.to_path_buf()),
        }.into()
    }
}

enum SigningPurpose { Development, Store, }
