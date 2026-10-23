use std::io::{self, IsTerminal, Write};
use std::path::Path;
use crate::PistonError;
use super::XcodeSigning;

pub struct SigningCli;

impl SigningCli {
    pub fn development(team: Option<&str>, developer: &Path) -> Result<XcodeSigning, PistonError> {
        Self::select(team, developer, SigningPurpose::Development)
    }

    pub fn store(team: Option<&str>, developer: &Path) -> Result<XcodeSigning, PistonError> {
        Self::select(team, developer, SigningPurpose::Store)
    }
    fn select(team: Option<&str>, developer: &Path, purpose: SigningPurpose) -> Result<XcodeSigning, PistonError> {
        if let Some(team) = team.filter(|team| !team.trim().is_empty()) {
            return Ok(XcodeSigning::new(team, developer));
        }

        let mut teams = XcodeSigning::teams()?;
        if teams.is_empty() { return Err(Self::missing_team(developer, purpose)); }
        let selected = if teams.len() == 1 {
            0
        } else {
            let choices = teams
                .iter()
                .enumerate()
                .map(|(index, (id, name))| format!("  {}. {name} ({id})", index + 1))
                .collect::<Vec<_>>()
                .join("\n");
            if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
                return Err(PistonError::CodesignError(format!(
                    "More than one Xcode signing team is available:\n{choices}\n\n\
                     Run in an interactive terminal to choose from the menu, \
                     or pass --team <TEAM_ID>."
                )));
            }
            eprintln!("Choose a signing team for this app:\n{choices}");
            loop {
                eprint!("Team [1-{}, q to cancel]: ", teams.len());
                io::stderr().flush().map_err(|error| {
                    PistonError::CodesignError(format!("Cannot display the team menu: {error}"))
                })?;
                let mut input = String::new();
                let read = io::stdin().read_line(&mut input).map_err(|error| {
                    PistonError::CodesignError(format!("Cannot read the team selection: {error}"))
                })?;
                if read == 0 || input.trim().eq_ignore_ascii_case("q") {
                    return Err(PistonError::CodesignError("Team selection cancelled.".into()));
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
    fn missing_team(developer: &Path, purpose: SigningPurpose) -> PistonError {
        if let SigningPurpose::Store = purpose {
            return PistonError::CodesignError(
                "No signing team is selected. Pass --team <TEAM_ID> using the 10-character Team ID \
                 from Membership details at https://developer.apple.com/account. Select the paid \
                 Apple Developer Program team associated with your App Store Connect API key.".into()
            );
        }
        PistonError::CodesignError(
            "No Xcode signing team is configured.\n\n\
             1. Open Xcode > Settings > Accounts, add or select your Apple Account, \
             and complete sign-in and any verification. Confirm that a team is listed.\n\
             2. Rerun your dev command. If Xcode lists several teams, select one with \
             --team <TEAM_ID>. Cargo will reuse its existing build cache."
                .into(),
        )
        .setup(crate::error::SetupPage::XcodeAccounts(developer.to_path_buf()))
    }
}

enum SigningPurpose { Development, Store, }
