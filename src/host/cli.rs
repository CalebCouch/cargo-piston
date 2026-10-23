use std::io::{self, IsTerminal, Write};
use std::path::Path;
use crate::{PistonError, Result};
use super::{Credentials, Host};

pub struct HostCli;

impl HostCli {
    pub fn interactive() -> bool { io::stdin().is_terminal() && io::stderr().is_terminal() }

    pub fn setup_identity(force: bool) -> Result<()> {
        if !force && !Credentials::needs_setup()? { return Ok(()); }
        Self::require_identity_terminal()?;
        let mut store = Credentials::new()?;
        let identities = store.identities()?;
        for (team, name) in identities { eprintln!("Team {team}: {name}"); }
        Self::identity(&mut store, None, None)
    }

    pub fn identity(store: &mut Credentials, team: Option<&str>, expected: Option<&str>) -> Result<()> {
        if let Some(team) = team {
            if store.identity(team, expected)?.is_some() { return Ok(()); }
            eprintln!("No developer identity is stored for Apple team {team}.");
        }
        if expected.is_some() {
            eprintln!(
                "This project has an owner identity. Ask a teammate to run cargo piston share."
            );
            return Self::restore(store, team, expected);
        }
        eprintln!(
            "Set up a developer identity. Each Apple team shares one identity across its apps.\n\
             1. Create an identity for a team you are setting up for the first time\n\
             2. Restore an identity shared by a teammate or saved from another Mac"
        );
        loop {
            match Self::identity_prompt("Identity [1-2, q to cancel]")?.as_str() {
                "1" => {
                    loop {
                        let selected = match team {
                            Some(team) => team.to_owned(),
                            None => Self::identity_prompt("Apple Team ID (10 characters)")?,
                        };
                        match store.create_identity(&selected) {
                            Ok(name) => {
                                eprintln!("Identity for team {selected} saved in Keychain: {name}");
                                return Ok(());
                            }
                            Err(error) if team.is_none() => eprintln!("{error}"),
                            Err(error) => return Err(error),
                        }
                    }
                }
                "2" => return Self::restore(store, team, expected),
                _ => eprintln!("Choose 1 to create or 2 to restore."),
            }
        }
    }

    pub fn share_identity() -> Result<()> {
        Self::require_identity_terminal()?;
        let mut store = Credentials::new()?;
        if store.identities()?.is_empty() { Self::identity(&mut store, None, None)?; }
        let identities = store.identities()?;
        eprintln!("Choose the developer identity to share:");
        for (index, (team, name)) in identities.iter().enumerate() {
            eprintln!("  {}. Team {team}: {name}", index + 1);
        }
        let team = loop {
            let choice = Self::identity_prompt("Identity number (q to cancel)")?;
            if let Some((team, _)) = choice.parse::<usize>().ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| identities.get(index))
            { break team; }
            eprintln!("Choose a listed identity number.");
        };
        eprintln!(
            "The following line contains the private identity for team {team}. \
             It grants full authority over every app using this identity.\n\
             Share it privately. Run cargo piston restore on the other Mac and paste the line."
        );
        println!("{}", store.share_identity(team)?);
        Ok(())
    }

    pub fn restore_identity() -> Result<()> {
        Self::require_identity_terminal()?;
        Self::restore(&mut Credentials::new()?, None, None)
    }

    pub fn open_url(url: &str) {
        if Self::interactive() {
            if let Err(error) = Host::open_url(url) {
                eprintln!("{error}. Open this address: {url}");
            }
        }
    }

    pub fn open_accounts(developer: &Path) {
        if Self::interactive() {
            eprintln!("Apple Account sign-in needs attention. Opening Xcode's Accounts settings.");
            if let Err(error) = Host::open_accounts(developer) { eprintln!("{error}"); }
        }
    }

    fn restore(store: &mut Credentials, team: Option<&str>, expected: Option<&str>) -> Result<()> {
        loop {
            let input = Self::identity_prompt("Paste the piston-identity: line (q to cancel)")?;
            match store.restore_identity(&input, team, expected) {
                Ok((team, name)) => {
                    eprintln!("Identity for team {team} restored to Keychain: {name}");
                    return Ok(());
                }
                Err(error) => eprintln!("{error}"),
            }
        }
    }

    fn require_identity_terminal() -> Result<()> {
        if !Self::interactive() {
            return Err(PistonError::Generic(
                "Developer identity setup needs an interactive terminal. Run cargo piston setup \
                 to create an identity or cargo piston restore to paste one shared by a teammate, \
                 then rerun the original command.".into(),
            ));
        }
        Ok(())
    }

    fn identity_prompt(label: &str) -> Result<String> {
        Self::require_identity_terminal()?;
        eprint!("{label}: ");
        io::stderr().flush().map_err(|error| PistonError::Generic(error.to_string()))?;
        let mut input = String::new();
        let count = io::stdin().read_line(&mut input)
            .map_err(|error| PistonError::Generic(error.to_string()))?;
        if count == 0 || input.trim().eq_ignore_ascii_case("q") {
            return Err(PistonError::Generic("Identity setup cancelled.".into()));
        }
        Ok(input.trim().to_owned())
    }
}
