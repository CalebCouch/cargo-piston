use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use crate::apple::app_store;
use crate::{developer, host};
use crate::manifest::Manifest;
use crate::Result;
use super::{AccountSetup, AscApiKey};

#[derive(Debug)]
pub enum Error {
    InteractiveRequired,
    CredentialsMissing,
    InputRead(io::Error),
    Cancelled,
    StoreCredentialsInvalid(serde_json::Error),
    StoreKeyIdInvalid,
    StoreIssuerInvalid,
    StoreKeyMissing { key_id: String },
    StoreKeyAmbiguous { key_id: String, paths: Vec<PathBuf> },
    StoreKeySearchFallback(Box<dyn std::error::Error>),
    StoreKeySearchDenied { path: PathBuf },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InteractiveRequired => write!(f,
                "--setup needs an interactive terminal.\n\n\
                 Rerun this deployment command in a terminal to replace the team's API key."),
            Self::CredentialsMissing => write!(f,
                "App Store Connect credentials have not been set up.\n\n\
                 Rerun this command in a terminal to import a Team API key.\n\
                 For unattended setup, set ASC_ISSUER_ID and ASC_KEY_ID or ASC_KEY_PATH."),
            Self::InputRead(source) => write!(f, "Cannot read setup input: {source}"),
            Self::Cancelled => write!(f, "App Store Connect setup cancelled."),
            Self::StoreCredentialsInvalid(source) => write!(f,
                "Invalid App Store Connect credentials in the developer contract: {source}. \
                 Rerun deployment with --setup to replace them."),
            Self::StoreKeyIdInvalid => write!(f,
                "Provide a 10-character Team API Key ID or an AuthKey_<KEY_ID>.p8 file path."),
            Self::StoreIssuerInvalid => write!(f,
                "Enter the Issuer ID shown above Team Keys (ASC_ISSUER_ID), in UUID format."),
            Self::StoreKeyMissing { key_id } => write!(f,
                "Cannot find AuthKey_{key_id}.p8 using system search or the standard key folders. \
                 Set ASC_KEY_PATH or use the interactive setup."),
            Self::StoreKeyAmbiguous { key_id, paths } => {
                writeln!(f, "Multiple files match Key ID {key_id}. Set ASC_KEY_PATH to choose one:")?;
                for path in paths { writeln!(f, "{}", path.display())?; }
                Ok(())
            }
            Self::StoreKeySearchFallback(source) => write!(f,
                "{source} Checking standard key folders directly."),
            Self::StoreKeySearchDenied { path } => write!(f,
                "Cannot search {}: permission denied. Supply the key file path if stored there.",
                path.display()),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InputRead(source) => Some(source),
            Self::StoreCredentialsInvalid(source) => Some(source),
            Self::StoreKeySearchFallback(source) => Some(source.as_ref()),
            _ => None,
        }
    }
}

pub struct Cli;

impl Cli {
    pub fn from_project(root: &Path, requested_team: Option<&str>, reconfigure: bool) -> Result<(AscApiKey, String)> {
        let manifest = Manifest::new(&root.join("Cargo.toml"))?;
        let (store, team) = developer::Cli::credentials(&manifest, requested_team, false)?;
        let mut setup = AccountSetup::new(root, team, store)?;
        let interactive = io::stdin().is_terminal() && io::stderr().is_terminal();
        if reconfigure && !interactive {
            return Err(Error::InteractiveRequired.into());
        }
        eprintln!("Loading App Store Connect credentials from AIR for team {}", setup.team);
        let saved = match setup.load(reconfigure) {
            Ok(saved) => saved,
            Err(error) if interactive && matches!(
                error.downcast_ref::<Error>(), Some(Error::StoreCredentialsInvalid(_)),
            ) => {
                eprintln!("{error}");
                None
            }
            Err(error) => return Err(error),
        };
        let mut settings = HashMap::new();
        if let Some(key) = saved {
            settings.insert("asc_issuer_id".into(), key.issuer_id().into());
            settings.insert("asc_key_id".into(), key.key_id().into());
            match key.authorize() {
                Ok(key) => {
                    setup.remember()?;
                    return Ok((key, setup.team));
                }
                Err(error) if interactive && matches!(
                    error.downcast_ref::<app_store::cli::Error>(),
                    Some(app_store::cli::Error::ASCAuthentication(_)
                        | app_store::cli::Error::StoreKeyInvalid { .. }),
                ) => {
                    eprintln!("{error}");
                }
                Err(error) => return Err(error),
            }
        }
        settings.extend(std::mem::take(&mut setup.settings));
        if !reconfigure && !settings.is_empty() {
            match AscApiKey::configured(&setup.root, &settings, &mut |error| eprintln!("{error}"))
                .and_then(AscApiKey::authorize)
            {
                Ok(key) => {
                    setup.save(&key)?;
                    eprintln!("App Store Connect credentials saved in AIR for team {}.", setup.team);
                    return Ok((key, setup.team));
                }
                Err(error) if interactive => eprintln!("{error}"),
                Err(error) => return Err(error),
            }
        }
        if !interactive {
            return Err(Error::CredentialsMissing.into());
        }
        eprintln!(
            "\nApp Store Connect setup\n  Project: {}\n  Apple team: {}\n",
            setup.root.display(), setup.team,
        );
        eprintln!("{}", host::Cli::link(
            "Team API keys", "https://appstoreconnect.apple.com/access/integrations/api",
        ));
        eprintln!(
            "\n1. Under Team Keys, create a key with Admin access.\n\
             2. Copy the Issuer ID (the account) and Key ID (the key).\n\
             3. Click Download API Key beside the key to save its .p8 file.\n\n\
             Apple allows one download. If the button is missing, use your existing file.\n\
             If the file is lost, create a new key. The Issuer ID is not stored in the file.\n\n\
             Apple Developer Program membership and API access are required.\n\
             If API access is disabled, ask the Account Holder to enable it.\n\n\
             Keep the file outside your repository. Credentials are saved encrypted in AIR.\n\
             Enter accepts a [default]; q cancels. Enter a file path, never private key contents."
        );
        loop {
            let issuer = Self::prompt(
                "Issuer ID",
                settings.get("asc_issuer_id").map(String::as_str).unwrap_or(""),
            )?;
            settings.insert("asc_issuer_id".into(), issuer);
            let default = settings
                .get("asc_key_path")
                .or(settings.get("asc_key_id"))
                .map(String::as_str)
                .unwrap_or("");
            let input = Self::prompt("Team API Key ID or .p8 file path", default)?;
            if AscApiKey::valid_id(&input) {
                if settings.get("asc_key_id") != Some(&input) { settings.remove("asc_key_path"); }
                settings.insert("asc_key_id".into(), input);
            } else {
                let path = AscApiKey::key_path(&setup.root, &input)?;
                let key_id = if let Some(key_id) = AscApiKey::id_from_path(&path) {
                    eprintln!("Using Key ID {key_id} from the filename.");
                    key_id.into()
                } else {
                    eprintln!("The filename has no Key ID. Copy it from the Team Keys page.");
                    Self::prompt("Team API Key ID", "")?
                };
                settings.insert("asc_key_id".into(), key_id);
                settings.insert("asc_key_path".into(), path.to_string_lossy().into_owned());
            }
            let key_id = &settings["asc_key_id"];
            if !AscApiKey::valid_id(key_id) {
                eprintln!("A Team API Key ID must contain 10 letters or digits.");
                continue;
            }
            if settings.get("asc_key_path").is_none_or(|path| path.is_empty()) {
                let paths = AscApiKey::find_key_paths(&setup.root, key_id, &mut |error| {
                    eprintln!("{error}")
                })?;
                let path = match paths.as_slice() {
                    [] => {
                        eprintln!(
                            "\nAuthKey_{key_id}.p8 was not found.\n\
                             Download the file beside Key ID {key_id}, or locate your existing copy.\n\
                             This prompt can stay open while you find it."
                        );
                        let input = Self::prompt(
                            "Path to the downloaded .p8 file",
                            &format!("~/Downloads/AuthKey_{key_id}.p8"),
                        )?;
                        AscApiKey::key_path(&setup.root, &input)?
                    }
                    [path] => { eprintln!("Found API key: {}", path.display()); path.clone() }
                    _ => {
                        eprintln!("\nSeveral files match Key ID {key_id}. Choose the file to use:");
                        for (index, path) in paths.iter().enumerate() {
                            eprintln!("  {}. {}", index + 1, path.display());
                        }
                        loop {
                            let input = Self::prompt(&format!("File [1-{}]", paths.len()), "")?;
                            if let Some(path) = input
                                .parse::<usize>()
                                .ok()
                                .and_then(|number| number.checked_sub(1))
                                .and_then(|index| paths.get(index))
                            { break path.clone(); }
                            eprintln!("Choose a number from 1 to {}.", paths.len());
                        }
                    }
                };
                settings.insert("asc_key_path".into(), path.to_string_lossy().into_owned());
            }
            loop {
                eprintln!("\nChecking App Store Connect API access...");
                match AscApiKey::configured(&setup.root, &settings, &mut |error| eprintln!("{error}"))
                    .and_then(AscApiKey::authorize)
                {
                    Ok(key) => {
                        setup.save(&key)?;
                        eprintln!(
                            "\nApp Store Connect is ready.\n\
                             Credentials saved in AIR for team {}.\n\
                             Team selection saved in {}.",
                            setup.team, setup.manifest.path().display(),
                        );
                        return Ok((key, setup.team));
                    }
                    Err(error) => eprintln!("\n{error}\n\nAccount setup has not been saved."),
                }
                let action = Self::prompt("Retry the check (r), or edit credentials", "edit")?;
                if !action.eq_ignore_ascii_case("r") { break; }
            }
        }
    }
    fn prompt(label: &str, default: &str) -> Result<String> {
        loop {
            eprint!("\n{label}");
            if !default.is_empty() { eprint!(" [{default}]"); }
            eprint!(" (q to cancel): ");
            io::stderr().flush()?;
            let mut input = String::new();
            let count = io::stdin().read_line(&mut input).map_err(Error::InputRead)?;
            if count == 0 || input.trim().eq_ignore_ascii_case("q") {
                return Err(Error::Cancelled.into());
            }
            let input = input.trim();
            if !input.is_empty() { return Ok(input.into()); }
            if !default.is_empty() { return Ok(default.into()); }
            eprintln!("A value is required.");
        }
    }
}
