use security_framework::os::macos::keychain::SecKeychain;
use security_framework::os::macos::keychain_item::SecKeychainItem;
use std::collections::HashMap;
use std::fs::{self, File};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use crate::{PistonError, Result};

pub struct Credentials {
    entries: HashMap<String, Vec<u8>>,
    item: SecKeychainItem,
    _lock: File,
}

impl Credentials {
    pub fn new() -> Result<Self> {
        let root = PathBuf::from(std::env::var_os("HOME").ok_or_else(|| {
            PistonError::Generic("Cannot locate Piston's credential lock without HOME.".into())
        })?)
        .join("Library/Caches/cargo-piston");
        fs::create_dir_all(&root).map_err(|error| {
            PistonError::Generic(format!(
                "Cannot create Piston's credential lock directory: {error}",
            ))
        })?;
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .open(root.join("credentials.lock"))
            .map_err(|error| {
                PistonError::Generic(format!("Cannot open Piston's credential lock: {error}",))
            })?;
        lock.try_lock().map_err(|error| {
            PistonError::Generic(format!(
                "Cannot lock Piston's credentials: {error}. Finish any other Piston notification \
             setup before retrying this command.",
            ))
        })?;
        let keychain = SecKeychain::default().map_err(Self::keychain_error)?;
        let (bytes, item) = match keychain.find_generic_password("cargo-piston", "credentials") {
            Ok(found) => found,
            Err(error) if error.code() == -25300 => {
                keychain
                    .add_generic_password("cargo-piston", "credentials", b"{}")
                    .map_err(Self::keychain_error)?;
                keychain
                    .find_generic_password("cargo-piston", "credentials")
                    .map_err(Self::keychain_error)?
            }
            Err(error) => return Err(Self::keychain_error(error)),
        };
        let entries = serde_json::from_slice(&bytes).map_err(|_| {
            PistonError::Generic(
            "Piston's Keychain item cargo-piston / credentials is invalid. Restore its backup; \
             do not delete it or replace an existing deployment identity.".into(),
        )
        })?;
        Ok(Self { entries, item, _lock: lock })
    }

    pub fn get(&self, account: &str) -> Option<&[u8]> {
        self.entries.get(account).map(Vec::as_slice)
    }

    pub fn save(&mut self, account: &str, secret: Vec<u8>) -> Result<()> {
        if self.get(account) == Some(secret.as_slice()) { return Ok(()); }
        let previous = self.entries.insert(account.into(), secret);
        if let Err(error) = self.item.set_password(&serde_json::to_vec(&self.entries).unwrap()) {
            match previous {
                Some(previous) => { self.entries.insert(account.into(), previous); }
                None => { self.entries.remove(account); }
            }
            return Err(Self::keychain_error(error));
        }
        Ok(())
    }

    fn keychain_error(error: security_framework::base::Error) -> PistonError {
        PistonError::Generic(format!(
            "Cannot access Piston's Keychain item cargo-piston / credentials: {error}. \
             Unlock your login keychain and allow cargo-piston access, then rerun.",
        ))
    }
}
