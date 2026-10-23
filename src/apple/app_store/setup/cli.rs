use std::io::{self, IsTerminal, Write};
use std::path::Path;
use crate::host::cli::HostCli;
use crate::{PistonError, Result};
use super::{AccountSetup, AscApiKey};

pub struct AccountCli;

impl AccountCli {
    pub fn from_project(root: &Path, reconfigure: bool) -> Result<AscApiKey> {
        let AccountSetup { root, manifest, mut settings, explicit } =
            AccountSetup::new(root, reconfigure)?;
        let interactive = io::stdin().is_terminal() && io::stderr().is_terminal();
        if reconfigure && !interactive {
            return Err(PistonError::Generic(
                "--setup requires an interactive terminal. For CI, set ASC_ISSUER_ID and either \
                 ASC_KEY_ID or ASC_KEY_PATH in the environment."
                    .into(),
            ));
        }
        if !reconfigure {
            match AscApiKey::configured(&root, &settings, &mut |message| eprintln!("{message}")) {
                Ok(key) => match key.authorize() {
                    Ok(key) => return Ok(key),
                    Err(PistonError::ASCAuthenticationError(error)) if interactive => {
                        eprintln!("{error}")
                    }
                    Err(error) => return Err(error),
                },
                Err(error) if !interactive => {
                    return Err(PistonError::Generic(format!(
                        "{error}\nRun this deploy command in a terminal for guided account setup, or set \
                     ASC_ISSUER_ID and either ASC_KEY_ID or ASC_KEY_PATH for CI.\n{}",
                        Self::setup_help(),
                    )));
                }
                Err(error) if !settings.is_empty() => eprintln!("{error}"),
                Err(_) => {}
            }
        }
        eprintln!("App Store Connect setup for {}\n{}", root.display(), Self::setup_help());
        eprintln!("Enter q at any prompt to cancel. Private key contents are never requested.");
        HostCli::open_url("https://appstoreconnect.apple.com/access/integrations/api");
        loop {
            let issuer = Self::prompt(
                "Issuer ID (from Apple's Team Keys page)",
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
                let path = AscApiKey::key_path(&root, &input)?;
                let key_id = if let Some(key_id) = AscApiKey::id_from_path(&path) {
                    eprintln!("Using Key ID {key_id} from the filename.");
                    key_id.into()
                } else {
                    eprintln!(
                        "This filename does not identify the key. Copy its Key ID from Team Keys."
                    );
                    Self::prompt("Team API Key ID", "")?
                };
                settings.insert("asc_key_id".into(), key_id);
                settings.insert("asc_key_path".into(), path.to_string_lossy().into_owned());
            }
            let key_id = &settings["asc_key_id"];
            if !AscApiKey::valid_id(&key_id) {
                eprintln!("A Team API Key ID must contain 10 letters or digits.");
                continue;
            }
            if settings.get("asc_key_path").is_none_or(|path| path.is_empty()) {
                let paths = AscApiKey::find_key_paths(&root, key_id, &mut |message| {
                    eprintln!("{message}")
                })?;
                let path = match paths.as_slice() {
                    [] => {
                        eprintln!(
                            "No AuthKey_{key_id}.p8 file was found using system search or the standard \
                            key folders. In Apple's Team Keys list, click Download API Key beside \
                            Key ID {key_id}, then enter the downloaded file's path. \
                            You can keep this prompt open while downloading."
                        );
                        let input = Self::prompt(
                            "Path to the downloaded .p8 file",
                            &format!("~/Downloads/AuthKey_{key_id}.p8"),
                        )?;
                        AscApiKey::key_path(&root, &input)?
                    }
                    [path] => { eprintln!("Found API key: {}", path.display()); path.clone() }
                    _ => {
                        eprintln!(
                            "Several files match Key ID {key_id}. Choose the file for this app:"
                        );
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
                eprintln!("Checking App Store Connect API access...");
                match AscApiKey::configured(&root, &settings, &mut |message| eprintln!("{message}"))
                    .and_then(AscApiKey::authorize)
                {
                    Ok(key) => {
                        manifest.save(
                            &["app_store_connect"],
                            &[
                                ("issuer_id", key.issuer_id()),
                                ("key_id", key.key_id()),
                                ("key_path", &key.path().to_string_lossy()),
                            ],
                        )?;
                        eprintln!("Account setup saved in {}.", manifest.path().display());
                        if explicit {
                            eprintln!(
                                "Shell and .env values still override Cargo.toml. Update them \
                                if you replaced credentials supplied there."
                            );
                        }
                        return Ok(key);
                    }
                    Err(error) => eprintln!("{error}\nAccount setup has not been saved."),
                }
                let action =
                    Self::prompt("Enter to edit credentials, r to retry the check", "edit")?;
                if !action.eq_ignore_ascii_case("r") { break; }
            }
        }
    }
    fn prompt(label: &str, default: &str) -> Result<String> {
        loop {
            eprint!("{label}");
            if !default.is_empty() { eprint!(" [{default}]"); }
            eprint!(": ");
            io::stderr().flush().map_err(|error| PistonError::Generic(error.to_string()))?;
            let mut input = String::new();
            let count = io::stdin().read_line(&mut input).map_err(|error| {
                PistonError::Generic(format!("Cannot read setup input: {error}"))
            })?;
            if count == 0 || input.trim().eq_ignore_ascii_case("q") {
                return Err(PistonError::Generic("App Store Connect setup cancelled.".into()));
            }
            let input = input.trim();
            if !input.is_empty() { return Ok(input.into()); }
            if !default.is_empty() { return Ok(default.into()); }
            eprintln!("A value is required.");
        }
    }
    fn setup_help() -> &'static str {
        "Piston needs an App Store Connect Team API key for automatic signing and uploads.\n\
         Open https://appstoreconnect.apple.com/access/integrations/api . Under Team Keys, \
         generate a key with Admin access. Enter the Issuer ID first, then either \
         the Key ID or the downloaded .p8 file's path. Piston uses system search and checks \
         standard key folders and Downloads by Key ID, or infers the Key ID from an AuthKey_<KEY_ID>.p8 filename. \
         The Issuer ID is not in the private key; enter it once and Piston remembers it for this project.\n\
         Click Download API Key beside that key to get the AuthKey_<KEY_ID>.p8 file. \
         Check your browser's downloads for its location. Apple only permits one download: \
         if the link is missing, locate the file you already downloaded. If it is lost, \
         create a new Team Key and restart setup with the new Key ID and file.\n\
         If API access is disabled, the Account Holder must enable it. Store distribution \
         requires Apple Developer Program membership. Xcode's saved login alone is not API access.\n\
         Keep the .p8 file outside your repository. Piston saves only its location and IDs \
         in Cargo.toml under package.metadata.piston.app_store_connect. It does not copy the private key."
    }
}
