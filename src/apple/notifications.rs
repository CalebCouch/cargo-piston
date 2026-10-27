use air::{DefaultResolver, Id, Inbox, Instance, Output, Resolver, Response, Secret};
use air_notifications::{ApnsCredentials, NotificationApplication, Notifications};
use air_server::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use crate::developer::Credentials;
use crate::manifest::Manifest;
use crate::{PistonError, Result};
use super::PushSigning;

pub mod cli;
pub use cli::Cli;

pub struct NotificationSetup {
    manifest: Manifest,
    pub signing: PushSigning,
    runtime: tokio::runtime::Runtime,
}

impl NotificationSetup {
    pub fn enabled(manifest: &Manifest) -> Result<Option<bool>> {
        Ok(manifest.setting(&["notifications", "enabled"])?
            .and_then(|value| value.as_bool()))
    }

    pub fn new(manifest: &Path, signing: PushSigning) -> Result<Self> {
        let manifest = Manifest::new(manifest)?;
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        Ok(Self { manifest, signing, runtime })
    }

    pub fn key_path(&self, input: &str) -> Result<PathBuf> {
        let input = input.trim().trim_matches(['\'', '"']);
        let path = if let Some(path) = input.strip_prefix("~/") {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or(PistonError::MissingEnvironment { name: "HOME" })?
                .join(path)
        } else {
            self.manifest.path().parent().unwrap().join(input)
        };
        let path = fs::canonicalize(&path).map_err(|error| {
            PistonError::PushKeyRead { path: path.clone(), source: error }
        })?;
        let root = self.manifest.path().parent().unwrap();
        let repository = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(root)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()));
        if path.starts_with(root) || repository.is_some_and(|root| path.starts_with(root)) {
            return Err(PistonError::PushKeyInRepository { path });
        }
        Ok(path)
    }

    pub fn key_id(path: &Path) -> Option<&str> {
        path.file_stem()?.to_str()?.strip_prefix("AuthKey_").filter(|id| Self::valid_id(id))
    }

    pub fn valid_id(id: &str) -> bool {
        id.len() == 10 && id.bytes().all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    }

    pub fn configure(&self, key: Option<(&Path, &str)>, store: &mut Credentials) -> Result<Option<String>> {
        let team = &self.signing.team;
        let account = format!(
            "apns:{}:{}:{}", self.signing.platform.name(), self.signing.topic, self.signing.sandbox,
        );
        self.runtime.block_on(store.load(team))?;
        let credentials = match key {
            Some((path, key_id)) => {
                if !Self::valid_id(key_id) || !Self::valid_id(team) {
                    return Err(PistonError::PushIdentifiersInvalid);
                }
                if Self::key_id(path).is_some_and(|id| id != key_id) {
                    return Err(PistonError::PushKeyIdMismatch {
                        key_id: key_id.into(), path: path.to_path_buf(),
                    });
                }
                let key = fs::read_to_string(path).map_err(|source| {
                    PistonError::PushKeyRead { path: path.to_path_buf(), source }
                })?;
                let encoding = jsonwebtoken::EncodingKey::from_ec_pem(key.as_bytes())
                    .map_err(PistonError::PushKeyEncoding)?;
                jsonwebtoken::encode(
                    &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256),
                    &serde_json::json!({"iss": team, "iat": 0}), &encoding,
                ).map_err(PistonError::PushKeySigning)?;
                let credentials = ApnsCredentials { key, key_id: key_id.into(), team_id: team.clone() };
                let bytes = serde_json::to_vec(&credentials).unwrap();
                self.runtime.block_on(store.save(team, &account, bytes))?;
                credentials
            }
            None => match store.get(team, &account) {
                Some(bytes) => serde_json::from_slice(bytes)
                    .map_err(PistonError::PushCredentialsInvalid)?,
                None => return Ok(None),
            },
        };
        if credentials.team_id != *team { return Err(PistonError::PushIdentifiersInvalid); }
        let secret = store.identity(team, None)?
            .ok_or_else(|| PistonError::PushIdentityMissing { team: team.clone() })?;
        let owner = secret.name().to_string();
        let init = (self.signing.topic.clone(), self.signing.sandbox);
        let application = self.runtime.block_on(async {
            let publish = async {
                let mut registration = Registration::new(team, &secret, &init, store).await?;
                let application = registration.instance.id().to_string();
                registration.publish(credentials, key.is_some(), secret, init, store).await?;
                Ok(application)
            };
            tokio::time::timeout(Duration::from_secs(90), publish).await
                .map_err(|_| PistonError::PushSetupTimeout)?
        })?;
        self.manifest.save::<&str>(
            &[
                "notifications", self.signing.platform.name(),
                if self.signing.sandbox { "development" } else { "production" },
            ],
            &[
                ("team", team), ("topic", &self.signing.topic),
                ("identity", &owner), ("application", &application),
            ],
        )?;
        Ok(Some(application))
    }
}

#[derive(Serialize, Deserialize)]
struct Registration {
    team: String,
    instance: Instance<NotificationApplication>,
    inbox: Inbox,
    shared: Option<bool>,
}

impl Registration {
    async fn new(team: &str, secret: &Secret, init: &(String, bool), store: &mut Credentials) -> Result<Self> {
        let location = Instance::<NotificationApplication>::generate_location(secret, init)
            .map_err(|error| PistonError::PushLocationError(format!("{error:?}")))?;
        let account = format!("application:{}", Id::hash(&location));
        if let Some(bytes) = store.get(team, &account) {
            let registration: Self = serde_json::from_slice(bytes)
                .map_err(PistonError::PushRegistrationInvalid)?;
            if registration.team != team || registration.instance.location() != &location {
                return Err(PistonError::PushLocationMismatch);
            }
            return Ok(registration);
        }
        let instance = Instance::receive(secret.clone(), location).map_err(|error| {
            PistonError::PushContractError(format!("{error:?}"))
        })?;
        let inbox = Inbox::new(secret.clone()).map_err(|error| {
            PistonError::PushInboxError(format!("{error:?}"))
        })?;
        let registration = Self { team: team.into(), instance, inbox, shared: None };
        registration.save(store).await?;
        Ok(registration)
    }

    async fn publish(
        &mut self, credentials: ApnsCredentials, refresh: bool,
        secret: Secret, init: (String, bool), store: &mut Credentials,
    ) -> Result<()> {
        if !refresh && self.shared == Some(true) && self.matches(&credentials) { return Ok(()); }
        let mut resolver = DefaultResolver::default();
        let identity = resolver.resolve(self.instance.location().server, None).await;
        let client = Client::new((*identity).clone(), "wss")
            .await
            .map_err(|error| PistonError::PushConnectionError(error.to_string()))?;
        self.synchronize(&client, &mut resolver, &secret, store).await?;
        if self.instance.confirmed().is_none() {
            self.instance = Instance::new(secret.clone(), init).map_err(|error| {
                PistonError::PushInitializationError(format!("{error:?}"))
            })?;
            self.synchronize(&client, &mut resolver, &secret, store).await?;
        }
        if !self.matches(&credentials) {
            if !self.instance.send(credentials.clone()) {
                return Err(PistonError::PushUpdateDenied);
            }
            self.synchronize(&client, &mut resolver, &secret, store).await?;
            if !self.matches(&credentials) {
                return Err(PistonError::PushCredentialsUnconfirmed);
            }
        }
        if self.shared != Some(true) {
            if self.shared.is_none() {
                self.inbox.send(&mut resolver, Notifications::server(), *self.instance.location()).await;
                self.shared = Some(false);
            }
            let mut requests = self.inbox.start();
            self.save(store).await?;
            while !requests.is_empty() {
                for (kind, request) in requests {
                    let responses = client.send(request).await;
                    let response = responses.recv().await.map_err(|_| {
                        PistonError::PushSharingDisconnected
                    })?;
                    if matches!(response, Response::InvalidSignature) {
                        return Err(PistonError::PushInboxSignatureRejected);
                    }
                    self.inbox.response(&mut resolver, kind, response).await;
                }
                requests = self.inbox.requests();
                self.save(store).await?;
            }
            self.shared = Some(true);
            self.save(store).await?;
        }
        Ok(())
    }

    async fn synchronize(&mut self, client: &Client, resolver: &mut DefaultResolver, secret: &Secret, store: &mut Credentials) -> Result<()> {
        let mut initializing = self.instance.confirmed().is_none() && self.instance.pending().is_some();
        let mut request = Some(self.instance.start());
        while let Some(outgoing) = request {
            self.save(store).await?;
            let responses = client.send(outgoing).await;
            let response = responses.recv().await.map_err(|_| {
                PistonError::PushConfirmationDisconnected
            })?;
            match self.instance.response(resolver, response).await {
                Output::Init(_) if initializing => {
                    self.instance = Instance::receive(secret.clone(), *self.instance.location())
                        .map_err(|error| PistonError::PushContractError(format!("{error:?}")))?;
                    initializing = false;
                    request = Some(self.instance.start());
                    continue;
                }
                Output::Created(false, _) | Output::Read(false, _) | Output::Garbage => {
                    return Err(PistonError::PushUpdateRejected);
                }
                _ => {}
            }
            request = self.instance.request();
        }
        self.save(store).await
    }

    fn matches(&self, credentials: &ApnsCredentials) -> bool {
        self.instance.confirmed().and_then(|state| state.credentials.as_ref()).is_some_and(
            |saved| {
                saved.key_id == credentials.key_id
                    && saved.team_id == credentials.team_id
                    && Sha256::digest(saved.key.as_bytes())
                        == Sha256::digest(credentials.key.as_bytes())
            },
        )
    }

    async fn save(&self, store: &mut Credentials) -> Result<()> {
        let account = format!("application:{}", self.instance.id());
        store.save(&self.team, &account, serde_json::to_vec(self).unwrap()).await
    }
}
