use std::io::{self, IsTerminal, Write};
use crate::host::{Credentials, Host};
use crate::host::cli::HostCli;
use crate::manifest::Manifest;
use crate::{PistonError, Result};
use super::NotificationSetup;

pub struct NotificationCli;

impl NotificationCli {
    pub fn enabled(manifest: &Manifest) -> Result<bool> {
        if let Some(enabled) = NotificationSetup::enabled(manifest)? { return Ok(enabled); }
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(PistonError::Generic(
                "Notification usage has not been set up. Rerun this command in an interactive \
                 terminal to answer whether the app uses push notifications, or set enabled = true \
                 or enabled = false in [package.metadata.piston.notifications] in Cargo.toml."
                    .into(),
            ));
        }
        loop {
            let answer =
                Self::prompt("Does this app use push notifications? (y/n, q to cancel)", "")?;
            let enabled = match answer.to_ascii_lowercase().as_str() {
                "y" | "yes" => true,
                "n" | "no" => false,
                _ => { eprintln!("Enter y for yes or n for no."); continue; }
            };
            manifest.save(&["notifications"], &[("enabled", enabled)])?;
            return Ok(enabled);
        }
    }

    pub fn configure(setup: &NotificationSetup) -> Result<String> {
        let mut store = Credentials::new()?;
        HostCli::identity(&mut store, Some(&setup.signing.team), setup.expected_identity())?;
        let interactive = io::stdin().is_terminal() && io::stderr().is_terminal();
        let environment = if setup.signing.sandbox { "development" } else { "production" };
        let path = setup.settings.get("key_path").map(String::as_str).unwrap_or("");
        let id = setup.settings.get("key_id").map(String::as_str).unwrap_or("");
        if !path.is_empty() && !id.is_empty() {
            match setup.key_path(path) {
                Ok(path) => {
                    println!("Checking {environment} notifications for {}", setup.signing.topic);
                    return setup.configure(&path, id, false, &mut store);
                }
                Err(error) if interactive => eprintln!("{error}"),
                Err(error) => return Err(error),
            }
        }
        if !interactive {
            return Err(PistonError::Generic(format!(
                "APNs setup for {} ({environment}) needs an interactive terminal. Rerun this \
                 command in an interactive terminal. For unattended builds, configure key_path and \
                 key_id in [package.metadata.piston.notifications.{}.{environment}] and \
                 provision this Mac's Piston Keychain identity. APNs keys are created at \
                 https://developer.apple.com/account/resources/authkeys/list.",
                setup.signing.topic,
                setup.signing.platform.name(),
            )));
        }
        println!(
            "APNs setup for {} ({environment}), Apple team {}",
            setup.signing.topic, setup.signing.team
        );
        println!(
            "Use an Apple Push Notification service key for this team, topic, and environment. \
            App Store Connect API keys do not provide push access. In Apple's Keys page, create \
            or select an APNs key, configure its scope, then download the .p8 file. \
            Apple permits downloading it only once. Keep the file outside your repository."
        );
        println!(
            "Piston shares the APNs credentials with RAMP's notification service over AIR. \
            Its owner identity and private registration stay in macOS Keychain. Enter q to cancel."
        );
        HostCli::open_url("https://developer.apple.com/account/resources/authkeys/list");
        loop {
            let input = Self::prompt(
                "APNs Key ID or .p8 file path",
                if path.is_empty() { id } else { path },
            )?;
            let (path, key_id) = if NotificationSetup::valid_id(&input) {
                let paths = Host::find_files(&format!("AuthKey_{input}.p8"))?;
                let path = match paths.as_slice() {
                    [path] => {
                        println!("Found {}", path.display());
                        path.to_string_lossy().into_owned()
                    }
                    [] => Self::prompt("Path to the downloaded APNs .p8 file", "")?,
                    _ => {
                        println!("Choose the downloaded APNs key:");
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
                    None => Self::prompt("APNs Key ID from Apple's Keys page", id)?,
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
                "Registering {environment} APNs configuration; waiting for AIR confirmation..."
            );
            let application = setup.configure(&path, &key_id, true, &mut store)?;
            println!(
                "APNs configuration confirmed in AIR and shared with the notification service. \
                Public settings saved in Cargo.toml. Apple key permissions are verified on push delivery."
            );
            return Ok(application);
        }
    }

    fn prompt(label: &str, default: &str) -> Result<String> {
        if default.is_empty() { eprint!("{label}: "); } else { eprint!("{label} [{default}]: "); }
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
