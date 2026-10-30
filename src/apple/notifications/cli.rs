use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use crate::developer;
use crate::host::{self, Host};
use crate::manifest::Manifest;
use crate::Result;
use super::NotificationSetup;

#[derive(Debug)]
pub enum Error {
    UsageNotConfigured,
    SetupIncomplete { topic: String, sandbox: bool },
    InputClosed,
    SetupCancelled,
    PushKeyRead { path: PathBuf, source: std::io::Error },
    PushKeyInRepository { path: PathBuf },
    PushIdentifiersInvalid,
    PushKeyIdMismatch { key_id: String, path: PathBuf },
    PushKeyEncoding(jsonwebtoken::errors::Error),
    PushKeySigning(jsonwebtoken::errors::Error),
    PushKeyRejected { key_id: String, topic: String, sandbox: bool, status: u16, reason: String },
    PushAccessUnavailable { sandbox: bool, output: std::process::Output },
    PushAccessResponseInvalid,
    PushIdentityMissing { team: String },
    PushSetupTimeout,
    PushLocation(String),
    PushContract(String),
    PushInbox(String),
    PushRegistrationInvalid(serde_json::Error),
    PushLocationMismatch,
    PushConnection(String),
    PushInitialization(String),
    PushUpdateDenied,
    PushCredentialsUnconfirmed,
    PushCredentialsInvalid(serde_json::Error),
    PushSharingDisconnected,
    PushInboxSignatureRejected,
    PushConfirmationDisconnected,
    PushUpdateRejected,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UsageNotConfigured => write!(f,
                "Push notification usage has not been set up.\n\n\
                 Rerun this command in a terminal to answer the setup question.\n\
                 Or set enabled = true or false in [package.metadata.piston.notifications]."),
            Self::SetupIncomplete { topic, sandbox } => {
                let environment = if *sandbox { "development" } else { "production" };
                write!(f, "APNs setup is incomplete for {topic} ({environment}).\n\n\
                    Rerun this command in a terminal to import an APNs key.\n\
                    To use setup from another computer, restore that team's identity first:\n\
                      cargo piston restore \"secret data\"\n\n{}",
                    host::Cli::link("Apple developer keys",
                        "https://developer.apple.com/account/resources/authkeys/list"))
            }
            Self::InputClosed => write!(f, "Notification setup cancelled: input closed."),
            Self::SetupCancelled => write!(f, "Notification setup cancelled."),
            Self::PushKeyRead { path, source } => write!(
                f, "Cannot read the APNs key at {}: {source}\n\n\
                 Enter the path to your downloaded .p8 file. If it is lost, create a new key:\n{}",
                path.display(), host::Cli::link(
                    "Apple keys", "https://developer.apple.com/account/resources/authkeys/list",
                ),
            ),
            Self::PushKeyInRepository { path } => write!(
                f, "Move the APNs .p8 file outside the repository, for example to ~/.keys.\n\
                 Piston stores the imported credentials encrypted in AIR.\n\nCurrent path: {}",
                path.display(),
            ),
            Self::PushIdentifiersInvalid => write!(
                f, "APNs Key ID and Team ID must be 10 uppercase letters or digits.",
            ),
            Self::PushKeyIdMismatch { key_id, path } => write!(
                f, "APNs Key ID {key_id} does not match the .p8 filename {}.", path.display(),
            ),
            Self::PushKeyEncoding(source) => {
                write!(f, "The APNs .p8 file must contain an EC private key: {source}")
            }
            Self::PushKeySigning(source) => {
                write!(f, "The APNs .p8 file is not a valid P-256 signing key: {source}")
            }
            Self::PushKeyRejected { key_id, topic, sandbox, status, reason } => {
                let environment = if *sandbox { "Sandbox" } else { "Production" };
                write!(f,
                    "Apple rejected APNs key {key_id} for {topic} ({environment}): \
                     HTTP {status}, {reason}.\n\n")?;
                if matches!(status, 400 | 403) {
                    if !sandbox {
                        writeln!(f, "TestFlight and App Store deployments require Production access. \
                            Sandbox-only keys can be used with cargo piston dev.")?;
                    }
                    write!(f,
                        "1. Open the key's details:\n   {}\n\n\
                         2. Under Apple Push Notification service, confirm {environment} access\n\
                            and permission for {topic}, or create a key with that access.\n\
                         3. Import the matching .p8 file and Key ID when Piston asks.\n\n\
                         Rerun this command in a terminal to replace the rejected key.",
                        host::Cli::link("Apple developer keys",
                            "https://developer.apple.com/account/resources/authkeys/list"))
                } else {
                    write!(f, "Apple could not verify APNs access. Retry this command later.\n\
                        Piston has not accepted or changed these credentials.")
                }
            }
            Self::PushAccessUnavailable { sandbox, output } => {
                let environment = if *sandbox { "sandbox" } else { "production" };
                write!(f,
                    "Could not check APNs {environment} access ({}).\n\
                     Check your network connection and retry. Piston requires a successful check.\n{}",
                    output.status, String::from_utf8_lossy(&output.stderr).trim())
            }
            Self::PushAccessResponseInvalid => write!(f,
                "Apple returned an unexpected APNs credential-check response.\n\
                 Access has not been verified; retry this command."),
            Self::PushIdentityMissing { team } => write!(
                f, "Developer identity for team {team} is missing. \
                 Run cargo piston setup or cargo piston restore.",
            ),
            Self::PushSetupTimeout => write!(
                f, "Notification setup timed out before confirmation. Check your connection \
                 and rerun; Piston will resume the same registration.",
            ),
            Self::PushLocation(source) => {
                write!(f, "Cannot create notification location: {source}")
            }
            Self::PushContract(source) => {
                write!(f, "Cannot create notification contract: {source}")
            }
            Self::PushInbox(source) => write!(f, "Cannot create deployment inbox: {source}"),
            Self::PushRegistrationInvalid(source) => write!(
                f, "Invalid notification registration in the developer contract: {source}. Restore its backup.",
            ),
            Self::PushLocationMismatch => {
                write!(f, "The saved notification location does not match this app.")
            }
            Self::PushConnection(source) => write!(f, "Cannot connect to AIR: {source}"),
            Self::PushInitialization(source) => {
                write!(f, "Cannot initialize notification contract: {source}")
            }
            Self::PushUpdateDenied => write!(
                f, "The deployment identity cannot update this notification contract.",
            ),
            Self::PushCredentialsInvalid(source) => write!(f,
                "Invalid APNs credentials in the developer contract: {source}."),
            Self::PushCredentialsUnconfirmed => {
                write!(f, "AIR did not confirm acceptance of the APNs credentials.")
            }
            Self::PushSharingDisconnected => write!(
                f, "AIR disconnected before the notification configuration was shared.",
            ),
            Self::PushInboxSignatureRejected => {
                write!(f, "AIR rejected the notification inbox signature.")
            }
            Self::PushConfirmationDisconnected => {
                write!(f, "AIR disconnected before confirming notification setup.")
            }
            Self::PushUpdateRejected => {
                write!(f, "AIR rejected the notification configuration update.")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PushRegistrationInvalid(source) => Some(source),
            Self::PushCredentialsInvalid(source) => Some(source),
            Self::PushKeyRead { source, .. } => Some(source),
            Self::PushKeyEncoding(source) => Some(source),
            Self::PushKeySigning(source) => Some(source),
            _ => None,
        }
    }
}

pub struct Cli;

impl Cli {
    pub fn enabled(manifest: &Manifest) -> Result<bool> {
        if let Some(enabled) = NotificationSetup::enabled(manifest)? { return Ok(enabled); }
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(Error::UsageNotConfigured.into());
        }
        eprintln!("\nPush notifications\nThis choice is saved in Cargo.toml.");
        loop {
            let answer = Self::prompt("Does this app use push notifications? (y/n)", "")?;
            let enabled = match answer.to_ascii_lowercase().as_str() {
                "y" | "yes" => true,
                "n" | "no" => false,
                _ => { eprintln!("Enter y for yes or n for no."); continue; }
            };
            manifest.save(&["notifications"], &[("enabled", enabled)])?;
            return Ok(enabled);
        }
    }

    pub fn configure(setup: &NotificationSetup, replace: bool) -> Result<String> {
        let (mut store, _) = developer::Cli::credentials(
            &setup.manifest, Some(&setup.signing.team), false,
        )?;
        let interactive = io::stdin().is_terminal() && io::stderr().is_terminal();
        let environment = if setup.signing.sandbox { "development" } else { "production" };
        println!("Checking {environment} notifications for {}", setup.signing.topic);
        if !replace {
            match setup.configure(None, &mut store) {
                Ok(Some(application)) => return Ok(application),
                Ok(None) => {}
                Err(error) if interactive && matches!(error.downcast_ref::<Error>(), Some(
                    Error::PushKeyRejected { status: 400 | 403, .. }
                    | Error::PushKeyEncoding(_) | Error::PushKeySigning(_)
                    | Error::PushIdentifiersInvalid | Error::PushCredentialsInvalid(_)
                )) => eprintln!("\n{error}"),
                Err(error) => return Err(error),
            }
        }
        if !interactive {
            return Err(Error::SetupIncomplete {
                topic: setup.signing.topic.clone(), sandbox: setup.signing.sandbox,
            }.into());
        }
        println!(
            "\nPush notification setup\n  App: {}\n  Environment: {environment}\n  Apple team: {}\n",
            setup.signing.topic, setup.signing.team,
        );
        println!("{}", host::Cli::link(
            "Apple developer keys", "https://developer.apple.com/account/resources/authkeys/list",
        ));
        println!(
            "\n1. Create or select a key with Apple Push Notification service enabled.\n\
             2. Set its scope for the team, app, and environment shown above.\n\
             3. Download the .p8 file and keep it outside your repository.\n\n\
             Apple allows one download. If your copy is lost, create a new key.\n\
             App Store Connect API keys do not provide push access.\n\n\
             Piston saves the key encrypted in AIR and shares it with the notification service.\n\
             Back up the team's identity with cargo piston backup. Enter q to cancel."
        );
        if setup.signing.sandbox {
            println!("\nDevelopment requires Sandbox access; a key supporting both environments works.");
        } else {
            println!("\nDeployment requires Production access. Sandbox-only keys will be rejected.");
        }
        loop {
            let input = Self::prompt("APNs Key ID or .p8 file path", "")?;
            let (path, key_id) = if NotificationSetup::valid_id(&input) {
                let paths = Host::find_files(&format!("AuthKey_{input}.p8"))?;
                let path = match paths.as_slice() {
                    [path] => {
                        println!("Found {}", path.display());
                        path.to_string_lossy().into_owned()
                    }
                    [] => {
                        eprintln!("AuthKey_{input}.p8 was not found. Locate your downloaded copy.");
                        Self::prompt("Path to the APNs .p8 file", "")?
                    }
                    _ => {
                        println!("\nChoose the downloaded APNs key:");
                        for (index, path) in paths.iter().enumerate() {
                            println!("  {}. {}", index + 1, path.display());
                        }
                        loop {
                            let choice = Self::prompt("File number", "")?;
                            if let Some(path) = choice
                                .parse::<usize>()
                                .ok()
                                .and_then(|index| index.checked_sub(1))
                                .and_then(|index| paths.get(index))
                            { break path.to_string_lossy().into_owned(); }
                            eprintln!("Choose a listed file number.");
                        }
                    }
                };
                (path, input)
            } else {
                let path = match setup.key_path(&input) {
                    Ok(path) => path,
                    Err(error) => { eprintln!("{error}"); continue; }
                };
                let id = match NotificationSetup::key_id(&path) {
                    Some(id) => id.to_owned(),
                    None => Self::prompt("APNs Key ID from Apple's Keys page", "")?,
                };
                (path.to_string_lossy().into_owned(), id)
            };
            let path = match setup.key_path(&path) {
                Ok(path) => path,
                Err(error) => { eprintln!("{error}"); continue; }
            };
            if !NotificationSetup::valid_id(&key_id) {
                eprintln!("The Key ID must contain 10 uppercase letters or digits.");
                continue;
            }
            println!(
                "\nChecking APNs {environment} access, then waiting for AIR confirmation..."
            );
            let application = match setup.configure(Some((&path, &key_id)), &mut store) {
                Ok(application) => application.unwrap(),
                Err(error) if matches!(error.downcast_ref::<Error>(), Some(
                    Error::PushKeyRead { .. } | Error::PushKeyEncoding(_)
                    | Error::PushKeySigning(_) | Error::PushKeyIdMismatch { .. }
                    | Error::PushIdentifiersInvalid
                    | Error::PushKeyRejected { status: 400 | 403, .. }
                )) => {
                    eprintln!("{error}"); continue;
                }
                Err(error) => return Err(error),
            };
            println!(
                "\nNotifications configured.\n\
                 Registration confirmed in AIR and shared with the notification service.\n\
                 Public settings saved in Cargo.toml.\n\
                 APNs credential preflight passed for {environment}.\n\
                 Notification delivery still requires a valid device registration."
            );
            return Ok(application);
        }
    }

    fn prompt(label: &str, default: &str) -> Result<String> {
        eprint!("\n{label}");
        if !default.is_empty() { eprint!(" [{default}]"); }
        eprint!(" (q to cancel): ");
        io::stderr().flush()?;
        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 { return Err(Error::InputClosed.into()); }
        let input = input.trim();
        if input.eq_ignore_ascii_case("q") { return Err(Error::SetupCancelled.into()); }
        Ok(if input.is_empty() { default.to_owned() } else { input.to_owned() })
    }
}
