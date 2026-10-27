use air::{Contract, DefaultResolver, Id, Instance, Metadata, Name, Output, Request, Resolver, Secret};
use air_server::Client;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{self, Debug};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use crate::cli::Error as SharedError;
use crate::host::Host;
use crate::manifest::Manifest;
use crate::Result;

pub mod cli;
pub use cli::Cli;
use cli::Error;

#[derive(Clone, Hash, Serialize, Deserialize)]
pub struct Developer {
    pub owner: Name,
    pub entries: BTreeMap<String, Vec<u8>>,
}

impl Debug for Developer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Developer").field("owner", &self.owner)
            .field("accounts", &self.entries.keys().collect::<Vec<_>>()).finish()
    }
}

impl Contract for Developer {
    type Message = (String, Vec<u8>);
    type Init = String;
    type Result = bool;

    fn id() -> Id { Id::hash("cargo.piston.developer.v1") }

    fn init(_team: String, metadata: Metadata) -> Self {
        Self { owner: metadata.signer, entries: BTreeMap::new() }
    }

    fn apply(&mut self, (account, bytes): Self::Message, metadata: Metadata) -> bool {
        if metadata.signer != self.owner { return false; }
        self.entries.insert(account, bytes);
        true
    }
}

pub struct Credentials {
    path: PathBuf,
    identities: BTreeMap<String, Secret>,
    instances: BTreeMap<String, Instance<Developer>>,
    client: Option<Client>,
    resolver: DefaultResolver,
    _lock: File,
}

impl Credentials {
    pub fn new() -> Result<Self> {
        let home = match std::env::var_os("CARGO_HOME") {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from(std::env::var_os("HOME")
                .ok_or(SharedError::MissingEnvironment { name: "HOME" })?).join(".cargo"),
        };
        let directory = home.join("piston");
        fs::create_dir_all(&directory).unwrap();
        Self::secure(&directory, true)?;
        let lock_path = directory.join("identities.lock");
        if lock_path.symlink_metadata().is_ok() { Self::secure(&lock_path, false)?; }
        let lock = File::options().read(true).write(true).create(true).open(&lock_path).unwrap();
        Host::private_permissions(&lock_path, false).unwrap();
        lock.try_lock().map_err(|source| Error::DeveloperStorageLock {
            path: lock_path, source: Box::new(source),
        })?;
        let path = directory.join("identities.json");
        let identities: BTreeMap<String, Secret> = match fs::symlink_metadata(&path) {
            Ok(_) => {
                Self::secure(&path, false)?;
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap()
            }
            Err(error) if error.kind() == ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => Err(error).unwrap(),
        };
        for (team, secret) in &identities { Self::validate_identity(team, secret, None)?; }
        Ok(Self {
            path, identities, instances: BTreeMap::new(), client: None,
            resolver: DefaultResolver::default(), _lock: lock,
        })
    }

    pub fn path(&self) -> &Path { &self.path }

    pub fn identities(&self) -> Vec<(String, Name)> {
        self.identities.iter().map(|(team, secret)| (team.clone(), secret.name())).collect()
    }

    pub fn identity(&self, team: &str, expected: Option<&str>) -> Result<Option<Secret>> {
        let Some(secret) = self.identities.get(team) else { return Ok(None); };
        if expected.is_some_and(|expected| expected != secret.name().to_string()) {
            return Err(Error::IdentityOwnerMismatch.into());
        }
        Ok(Some(secret.clone()))
    }

    pub fn create_identity(&mut self, team: &str) -> Result<Name> {
        if let Some(secret) = self.identities.get(team) { return Ok(secret.name()); }
        let secret = Secret::new();
        Self::validate_identity(team, &secret, None)?;
        let name = secret.name();
        self.store_identity(team, secret);
        Ok(name)
    }

    pub fn backup_identity(&self, team: &str) -> Result<String> {
        let secret = self.identities.get(team)
            .ok_or_else(|| Error::IdentityMissing { team: team.into() })?;
        let bytes = serde_json::to_vec(&(team, secret)).unwrap();
        Ok(format!("piston-identity:{}", STANDARD.encode(bytes)))
    }

    pub fn restore_identity(&mut self, input: &str, team: Option<&str>, expected: Option<&str>) -> Result<(String, Name)> {
        let bytes = input.trim().strip_prefix("piston-identity:")
            .and_then(|data| STANDARD.decode(data).ok());
        let restored: Option<(String, Secret)> =
            bytes.and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let (restored_team, secret) = restored.ok_or(Error::IdentityImportInvalid)?;
        if let Some(team) = team.filter(|team| *team != restored_team) {
            return Err(Error::IdentityTeamMismatch {
                expected: team.into(), actual: restored_team,
            }.into());
        }
        Self::validate_identity(&restored_team, &secret, expected)?;
        if self.identities.get(&restored_team).is_some_and(|existing| *existing != secret) {
            return Err(Error::IdentityConflict { team: restored_team }.into());
        }
        let name = secret.name();
        self.store_identity(&restored_team, secret);
        Ok((restored_team, name))
    }

    pub fn remember(&self, manifest: &Manifest, team: &str) -> Result<()> {
        let identity = self.identities.get(team).unwrap().name().to_string();
        manifest.save(&["developer"], &[("team", team), ("identity", identity.as_str())])
    }

    pub async fn load(&mut self, team: &str) -> Result<()> {
        let secret = self.identity(team, None)?
            .ok_or_else(|| Error::IdentityMissing { team: team.into() })?;
        tokio::time::timeout(Duration::from_secs(90), async {
            if self.client.is_none() {
                let server = self.resolver.resolve(Name::orange_me(), None).await;
                self.client = Some(Client::new((*server).clone(), "wss").await
                    .map_err(Error::DeveloperConnection)?);
            }
            let location = Instance::<Developer>::generate_location(&secret, &team.to_string())
                .map_err(|error| Error::DeveloperContract(error.to_string()))?;
            let mut instance = match self.instances.remove(team) {
                Some(instance) => instance,
                None => Instance::receive(secret.clone(), location)
                    .map_err(|error| Error::DeveloperContract(error.to_string()))?,
            };
            self.synchronize(team, &mut instance).await?;
            if instance.confirmed().is_none() {
                instance = Instance::new(secret.clone(), team.to_string())
                    .map_err(|error| Error::DeveloperContract(error.to_string()))?;
                let request = instance.start();
                if !matches!(self.exchange(team, &mut instance, request).await?, Output::Init(_)) {
                    return Err(Error::DeveloperRejected { team: team.into() }.into());
                }
                instance = Instance::receive(secret.clone(), location)
                    .map_err(|error| Error::DeveloperContract(error.to_string()))?;
                self.synchronize(team, &mut instance).await?;
            }
            if instance.confirmed().is_none_or(|state| state.owner != secret.name()) {
                return Err(Error::DeveloperOwnerMismatch { team: team.into() }.into());
            }
            self.instances.insert(team.into(), instance);
            Ok(())
        }).await.map_err(|_| Error::DeveloperTimeout { team: team.into() })?
    }

    pub fn get(&self, team: &str, account: &str) -> Option<&[u8]> {
        self.instances.get(team).unwrap().confirmed().unwrap().entries.get(account)
            .map(Vec::as_slice)
    }

    pub async fn save(&mut self, team: &str, account: &str, bytes: Vec<u8>) -> Result<()> {
        let mut instance = self.instances.remove(team).unwrap();
        tokio::time::timeout(Duration::from_secs(90), async {
            self.synchronize(team, &mut instance).await?;
            if instance.confirmed().unwrap().entries.get(account) != Some(&bytes) {
                if !instance.send((account.into(), bytes.clone())) {
                    return Err(Error::DeveloperRejected { team: team.into() }.into());
                }
                self.synchronize(team, &mut instance).await?;
                if instance.confirmed().unwrap().entries.get(account) != Some(&bytes) {
                    return Err(Error::DeveloperUnconfirmed {
                        team: team.into(), account: account.into(),
                    }.into());
                }
            }
            self.instances.insert(team.into(), instance);
            Ok(())
        }).await.map_err(|_| Error::DeveloperTimeout { team: team.into() })?
    }

    fn store_identity(&mut self, team: &str, secret: Secret) {
        if self.identities.get(team) == Some(&secret) { return; }
        let mut identities = self.identities.clone();
        identities.insert(team.into(), secret);
        let bytes = serde_json::to_vec(&identities).unwrap();
        let mut file = tempfile::NamedTempFile::new_in(self.path.parent().unwrap()).unwrap();
        file.write_all(&bytes).unwrap();
        file.as_file().sync_all().unwrap();
        file.persist(&self.path).unwrap();
        File::open(self.path.parent().unwrap()).unwrap().sync_all().unwrap();
        self.identities = identities;
    }

    fn secure(path: &Path, directory: bool) -> Result<()> {
        let metadata = fs::symlink_metadata(path).unwrap();
        if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
            return Err(Error::DeveloperStoragePath { path: path.into() }.into());
        }
        Host::private_permissions(path, directory).unwrap();
        Ok(())
    }

    fn validate_identity(team: &str, secret: &Secret, expected: Option<&str>) -> Result<()> {
        if team.len() != 10
            || !team.bytes().all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        { return Err(Error::IdentityTeamInvalid { team: team.into() }.into()); }
        if !secret.path().is_empty()
            || secret.name().to_string() != secret.harden(None).public_key().to_string()
        { return Err(Error::IdentityRootMismatch.into()); }
        if expected.is_some_and(|expected| expected != secret.name().to_string()) {
            return Err(Error::IdentityOwnerMismatch.into());
        }
        Ok(())
    }

    async fn synchronize(&mut self, team: &str, instance: &mut Instance<Developer>) -> Result<()> {
        let mut request = instance.start();
        loop {
            match self.exchange(team, instance, request).await? {
                Output::Subscribed => return Ok(()),
                Output::Garbage => return Err(Error::DeveloperRejected { team: team.into() }.into()),
                _ => {}
            }
            request = instance.request().unwrap();
        }
    }

    async fn exchange(&mut self, team: &str, instance: &mut Instance<Developer>, request: Request) -> Result<Output<bool>> {
        let responses = self.client.as_ref().unwrap().send(request).await;
        let response = responses.recv().await
            .map_err(|_| Error::DeveloperDisconnected { team: team.into() })?;
        Ok(instance.response(&mut self.resolver, response).await)
    }
}
