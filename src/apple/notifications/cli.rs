use std::io::{self, IsTerminal, Write};
use crate::developer;
use crate::host::{self, Host};
use crate::manifest::Manifest;
use crate::{PistonError, Result};
use super::NotificationSetup;

pub struct Cli;

impl Cli {
    pub fn enabled(manifest: &Manifest) -> Result<bool> {
        if let Some(enabled) = NotificationSetup::enabled(manifest)? { return Ok(enabled); }
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(PistonError::Generic(
                "Push notification usage has not been set up.\n\n\
                 Rerun this command in a terminal to answer the setup question.\n\
                 Or set enabled = true or false in [package.metadata.piston.notifications].".into(),
            ));
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
            if let Some(application) = setup.configure(None, &mut store)? { return Ok(application); }
        }
        if !interactive {
            return Err(PistonError::Generic(format!(
                "APNs setup is incomplete for {} ({environment}).\n\n\
                 Rerun this command in a terminal to import an APNs key.\n\
                 To use setup from another computer, restore that team's identity first:\n\
                   cargo piston restore \"secret data\"\n\n{}",
                setup.signing.topic,
                host::Cli::link("Apple developer keys",
                    "https://developer.apple.com/account/resources/authkeys/list"),
            )));
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
                "\nRegistering {environment} notifications; waiting for AIR confirmation..."
            );
            let application = match setup.configure(Some((&path, &key_id)), &mut store) {
                Ok(application) => application.unwrap(),
                Err(error @ (PistonError::PushKeyRead { .. } | PistonError::PushKeyEncoding(_)
                    | PistonError::PushKeySigning(_) | PistonError::PushKeyIdMismatch { .. })) => {
                    eprintln!("{error}"); continue;
                }
                Err(error) => return Err(error),
            };
            println!(
                "\nNotifications configured.\n\
                 Registration confirmed in AIR and shared with the notification service.\n\
                 Public settings saved in Cargo.toml.\n\
                 Apple checks the key's permissions when a push is sent."
            );
            return Ok(application);
        }
    }

    fn prompt(label: &str, default: &str) -> Result<String> {
        eprint!("\n{label}");
        if !default.is_empty() { eprint!(" [{default}]"); }
        eprint!(" (q to cancel): ");
        io::stderr().flush().map_err(|error| PistonError::Generic(error.to_string()))?;
        let mut input = String::new();
        if io::stdin()
            .read_line(&mut input)
            .map_err(|error| PistonError::Generic(error.to_string()))?
            == 0
        { return Err(PistonError::Generic("Notification setup cancelled: input closed.".into())); }
        let input = input.trim();
        if input.eq_ignore_ascii_case("q") {
            return Err(PistonError::Generic("Notification setup cancelled.".into()));
        }
        Ok(if input.is_empty() { default.to_owned() } else { input.to_owned() })
    }
}
