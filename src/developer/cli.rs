use clap::Parser;
use std::io::{self, Write};
use crate::host;
use crate::manifest::Manifest;
use crate::project::Project;
use crate::{PistonError, Result};
use super::Credentials;

pub struct Cli;

impl Cli {
    pub fn credentials(manifest: &Manifest, requested_team: Option<&str>, select: bool) -> Result<(Credentials, String)> {
        let mut store = Credentials::new()?;
        let saved = manifest.settings(&["developer"])?;
        if !select {
            if let (Some(team), Some(identity)) = (saved.get("team"), saved.get("identity")) {
                if requested_team.is_none_or(|requested| requested == team) {
                    match store.identity(team, Some(identity)) {
                        Ok(Some(_)) => return Ok((store, team.clone())),
                        Ok(None) | Err(PistonError::IdentityOwnerMismatch) => (),
                        Err(error) => return Err(error),
                    }
                }
            }
        }
        let identities: Vec<_> = store.identities().into_iter()
            .filter(|(team, _)| requested_team.is_none_or(|requested| requested == team)).collect();
        eprintln!(
            "\nDeveloper identity\n\nProject: {}\n\n\
             Choose the team identity to use for this project.\n\
             Joining a team with an existing backup? Cancel and run:\n\n  \
             cargo piston restore \"secret data\"\n",
            manifest.path().display(),
        );
        for (index, (team, _)) in identities.iter().enumerate() {
            eprintln!("  {}. Apple team {team}", index + 1);
        }
        eprintln!("  0. Create a team identity\n");
        let team = loop {
            let choice = Self::prompt("Choose a number (q to cancel)")?;
            if choice == "0" {
                if requested_team.is_none() {
                    let url = "https://developer.apple.com/account#MembershipDetailsCard";
                    eprintln!(
                        "\nApple developer team\n\n\
                         Use the Team ID for the Apple account that will sign your app.\n\n\
                         {}\n\n  \
                         1. Open the link and sign in to your Apple Account.\n  \
                         2. Select your team and click Membership details.\n  \
                         3. Copy the 10-character Team ID and paste it below.\n",
                        host::Cli::link("Apple Developer account", url),
                    );
                }
                loop {
                    let team = match requested_team {
                        Some(team) => team.to_owned(),
                        None => Self::prompt("Apple Team ID (10 characters)")?,
                    };
                    match store.create_identity(&team) {
                        Ok(_) => {
                            eprintln!("\nIdentity ready for Apple team {team}.");
                            eprintln!("Keep a private backup: cargo piston backup");
                        }
                        Err(error @ PistonError::IdentityTeamInvalid { .. }) if requested_team.is_none() => {
                            eprintln!("{error}"); continue;
                        }
                        Err(error) => return Err(error),
                    }
                    store.remember(manifest, &team)?;
                    return Ok((store, team));
                }
            }
            if let Some((team, _)) = choice.parse::<usize>().ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| identities.get(index))
            { break team.clone(); }
            eprintln!("Choose a listed identity number or 0 to create one.");
        };
        store.remember(manifest, &team)?;
        eprintln!("\nApple team {team} selected. Choice saved in Cargo.toml.");
        Ok((store, team))
    }

    pub fn start(select: bool) -> Result<()> {
        let manifest = Project::manifest(&cargo_subcommand::Args::parse_from(["piston"]))?;
        let (_, team) = Self::credentials(&manifest, None, select)?;
        eprintln!("Using developer identity for team {team}.");
        Ok(())
    }

    pub fn backup() -> Result<()> {
        let store = Credentials::new()?;
        let identities = store.identities();
        if identities.is_empty() {
            return Err(PistonError::Generic(
                "No developer identities are saved on this computer.\n\n\
                 Create one from your project: cargo piston\n\
                 Restore a backup: cargo piston restore \"secret data\"".into(),
            ));
        }
        eprintln!("\nBack up a developer identity\n\nChoose an Apple team:\n");
        for (index, (team, _)) in identities.iter().enumerate() {
            eprintln!("  {}. Apple team {team}", index + 1);
        }
        let team = loop {
            let choice = Self::prompt("Choose a number (q to cancel)")?;
            if let Some((team, _)) = choice.parse::<usize>().ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| identities.get(index))
            { break team; }
            eprintln!("Choose a listed identity number.");
        };
        eprintln!(
            "\nPrivate backup for Apple team {team}\n\n\
             Copy the complete piston-identity: line below and keep it private.\n\
             Anyone with it can access this identity's apps and stored credentials.\n\n\
             Restore it on another computer with:\n  cargo piston restore \"secret data\"\n"
        );
        println!("{}", store.backup_identity(team)?);
        Ok(())
    }

    pub fn restore(secret: &str) -> Result<()> {
        let mut store = Credentials::new()?;
        let (team, _) = store.restore_identity(secret, None, None)?;
        eprintln!(
            "\nIdentity restored for Apple team {team}.\n\n\
             Saved privately to: {}\n\n\
             Next, run cargo piston in your project to select this identity.",
            store.path().display(),
        );
        Ok(())
    }

    fn prompt(label: &str) -> Result<String> {
        if !host::Cli::interactive() {
            return Err(PistonError::Generic(
                "Identity setup needs an interactive terminal.\n\n\
                 Run cargo piston in your project to select or create an identity.\n\
                 To use an existing backup, restore it first:\n\n  \
                 cargo piston restore \"secret data\"".into(),
            ));
        }
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
