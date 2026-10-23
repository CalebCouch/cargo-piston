use air::{Name, Secret};
use base64::{Engine, engine::general_purpose::STANDARD};
use crate::{PistonError, Result};

#[cfg(target_os = "macos")]
#[path = "host/macos.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "host/linux.rs"]
mod platform;
pub use platform::{Credentials, Host, LinuxToolchain};

pub mod cli;

impl Credentials {
    pub fn identities(&self) -> Result<Vec<(String, Name)>> {
        let mut identities = Vec::new();
        for account in self.accounts() {
            if let Some(team) = account.strip_prefix("team:") {
                identities.push((team.to_owned(), self.identity(team, None)?.unwrap().name()));
            }
        }
        identities.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(identities)
    }

    pub fn identity(&self, team: &str, expected: Option<&str>) -> Result<Option<Secret>> {
        let Some(bytes) = self.get(&format!("team:{team}")) else { return Ok(None); };
        let secret = serde_json::from_slice(bytes).map_err(|_| {
            PistonError::Generic(format!("The Keychain identity for team {team} is invalid."))
        })?;
        Self::validate_identity(team, &secret, expected)?;
        Ok(Some(secret))
    }

    pub fn create_identity(&mut self, team: &str) -> Result<Name> {
        if let Some(secret) = self.identity(team, None)? { return Ok(secret.name()); }
        let secret = Secret::new();
        Self::validate_identity(team, &secret, None)?;
        self.save(&format!("team:{team}"), serde_json::to_vec(&secret).unwrap())?;
        Ok(secret.name())
    }

    pub fn share_identity(&self, team: &str) -> Result<String> {
        let secret = self.identity(team, None)?.ok_or_else(|| {
            PistonError::Generic(format!("No identity is stored for team {team}."))
        })?;
        let bytes = serde_json::to_vec(&(team, secret)).unwrap();
        Ok(format!("piston-identity:{}", STANDARD.encode(bytes)))
    }

    pub fn restore_identity(&mut self, input: &str, team: Option<&str>, expected: Option<&str>) -> Result<(String, Name)> {
        let bytes = input.trim().strip_prefix("piston-identity:")
            .and_then(|data| STANDARD.decode(data).ok());
        let restored: Option<(String, Secret)> =
            bytes.and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let (restored_team, secret) = restored.ok_or_else(|| {
            PistonError::Generic(
                "Paste the complete piston-identity: line from cargo piston share.".into(),
            )
        })?;
        if team.is_some_and(|team| team != restored_team) {
            return Err(PistonError::Generic(
                "This identity belongs to a different Apple team. Ask for that team's identity."
                    .into(),
            ));
        }
        Self::validate_identity(&restored_team, &secret, expected)?;
        if self.identity(&restored_team, None)?.is_some_and(|existing| existing != secret) {
            return Err(PistonError::Generic(format!(
                "Team {restored_team} already has a different identity in Keychain. \
                 Piston will not overwrite it. Use the matching identity or the intended Mac."
            )));
        }
        self.save(&format!("team:{restored_team}"), serde_json::to_vec(&secret).unwrap())?;
        Ok((restored_team, secret.name()))
    }

    fn validate_identity(team: &str, secret: &Secret, expected: Option<&str>) -> Result<()> {
        if team.len() != 10
            || !team.bytes().all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            return Err(PistonError::Generic(
                "An Apple Team ID must contain 10 uppercase letters or digits.".into(),
            ));
        }
        if !secret.path().is_empty()
            || secret.name().to_string() != secret.harden(None).public_key().to_string()
        {
            return Err(PistonError::Generic(
                "Invalid team identity: the root key does not match.".into(),
            ));
        }
        if expected.is_some_and(|expected| expected != secret.name().to_string()) {
            return Err(PistonError::Generic(
                "This identity does not own the project's notification configuration. \
                 Ask a teammate to run cargo piston share on the Mac that configured it, \
                 then restore that identity."
                    .into(),
            ));
        }
        Ok(())
    }
}
