use air::{DefaultResolver, Id, Inbox, Instance, Output, Resolver, Response, Secret};
use air_notifications::{ApnsCredentials, NotificationApplication, Notifications};
use air_server::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use crate::host::Credentials;
use crate::manifest::Manifest;
use crate::{PistonError, Result};
use super::PushSigning;

pub mod cli;

pub struct NotificationSetup {
    pub manifest: Manifest,
    pub signing: PushSigning,
    pub settings: HashMap<String, String>,
}

impl NotificationSetup {
    pub fn enabled(manifest: &Manifest) -> Result<Option<bool>> {
        if let Some(value) = manifest.setting(&["notifications", "enabled"])? {
            return value.as_bool().map(Some).ok_or_else(|| {
                PistonError::CargoParseError(
                    "package.metadata.piston.notifications.enabled must be true or false.".into(),
                )
            });
        }
        for platform in ["ios", "macos"] {
            for environment in ["development", "production"] {
                if !manifest.settings(&["notifications", platform, environment])?.is_empty() {
                    return Ok(Some(true));
                }
            }
        }
        Ok(None)
    }

    pub fn new(manifest: &Path, signing: PushSigning) -> Result<Self> {
        let manifest = Manifest::new(manifest)?;
        let settings = manifest.settings(&[
            "notifications",
            signing.platform.name(),
            if signing.sandbox { "development" } else { "production" },
        ])?;
        Ok(Self { manifest, signing, settings })
    }

    pub fn key_path(&self, input: &str) -> Result<PathBuf> {
        let input = input.trim().trim_matches(['\'', '"']);
        let path = if let Some(path) = input.strip_prefix("~/") {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or_else(|| {
                    PistonError::Generic("Cannot expand ~ without a home directory.".into())
                })?
                .join(path)
        } else {
            self.manifest.path().parent().unwrap().join(input)
        };
        let path = fs::canonicalize(&path).map_err(|error| {
            PistonError::Generic(format!(
                "Cannot read the APNs key at {}: {error}. Download it from \
             https://developer.apple.com/account/resources/authkeys/list.",
                path.display(),
            ))
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
            return Err(PistonError::Generic(
                "Keep the APNs .p8 key outside the project and repository, for example in \
                 ~/.keys. Cargo.toml stores its path, never its contents."
                    .into(),
            ));
        }
        Ok(path)
    }

    pub fn key_id(path: &Path) -> Option<&str> {
        path.file_stem()?.to_str()?.strip_prefix("AuthKey_").filter(|id| Self::valid_id(id))
    }

    pub fn valid_id(id: &str) -> bool {
        id.len() == 10 && id.bytes().all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    }

    pub fn configure(&self, path: &Path, key_id: &str, refresh: bool) -> Result<String> {
        if !Self::valid_id(key_id) || !Self::valid_id(&self.signing.team) {
            return Err(PistonError::Generic(
                "APNs Key ID and Team ID must be 10 uppercase letters or digits.".into(),
            ));
        }
        if Self::key_id(path).is_some_and(|id| id != key_id) {
            return Err(PistonError::Generic(
                "The APNs Key ID does not match the .p8 filename.".into(),
            ));
        }
        let key = fs::read_to_string(path).map_err(|error| {
            PistonError::Generic(format!("Cannot read APNs key {}: {error}", path.display(),))
        })?;
        let encoding = jsonwebtoken::EncodingKey::from_ec_pem(key.as_bytes()).map_err(|_| {
            PistonError::Generic("The APNs .p8 file must contain an EC private key.".into())
        })?;
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256),
            &serde_json::json!({"iss": self.signing.team, "iat": 0}),
            &encoding,
        )
        .map_err(|_| {
            PistonError::Generic("The APNs .p8 file is not a valid P-256 signing key.".into())
        })?;
        let credentials =
            ApnsCredentials { key, key_id: key_id.into(), team_id: self.signing.team.clone() };
        let account = format!("team:{}", self.signing.team);
        let same_app = self.settings.get("team") == Some(&self.signing.team)
            && self.settings.get("topic") == Some(&self.signing.topic);
        let expected = self
            .settings
            .get("identity")
            .filter(|_| self.settings.get("team") == Some(&self.signing.team));
        let mut store = Credentials::new()?;
        let secret: Secret = match store.get(&account) {
            Some(bytes) => serde_json::from_slice(bytes).map_err(|_| {
                PistonError::Generic(
                    "Piston's deployment identity in Keychain is invalid. Restore its backup."
                        .into(),
                )
            })?,
            None if expected.is_some() => {
                return Err(PistonError::Generic(format!(
                    "The deployment identity for team {} is missing from Keychain. Restore the \
                 cargo-piston / credentials item containing {account} from the Mac that configured \
                 this app. \
                 A new identity cannot update the existing notification configuration.",
                    self.signing.team,
                )));
            }
            None => {
                let secret = Secret::new();
                store.save(&account, serde_json::to_vec(&secret).unwrap())?;
                secret
            }
        };
        let owner = secret.name().to_string();
        if expected.is_some_and(|expected| expected != &owner) {
            return Err(PistonError::Generic(
                "The Keychain identity does not own this app's notification configuration.".into(),
            ));
        }
        let init = (self.signing.topic.clone(), self.signing.sandbox);
        let mut registration = Registration::new(&secret, &init, &mut store)?;
        let application = registration.instance.id().to_string();
        if same_app && self.settings.get("application").is_some_and(|id| id != &application) {
            return Err(PistonError::Generic(
                "The saved notification application does not match its Keychain identity.".into(),
            ));
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| PistonError::Generic(error.to_string()))?;
        runtime.block_on(async {
            let publish = registration.publish(credentials, refresh, secret, init, &mut store);
            tokio::time::timeout(Duration::from_secs(90), publish).await
                .map_err(|_| PistonError::Generic(
                    "Notification setup timed out before confirmation. Check your connection and rerun; \
                     Piston will resume the same registration.".into(),
                ))?
        })?;
        self.manifest.save::<&str>(
            &[
                "notifications",
                self.signing.platform.name(),
                if self.signing.sandbox { "development" } else { "production" },
            ],
            &[
                ("team", &self.signing.team),
                ("topic", &self.signing.topic),
                ("identity", &owner),
                ("keychain", "cargo-piston/credentials"),
                ("application", &application),
                ("key_id", key_id),
                ("key_path", &path.to_string_lossy()),
            ],
        )?;
        Ok(application)
    }
}

#[derive(Serialize, Deserialize)]
struct Registration {
    instance: Instance<NotificationApplication>,
    inbox: Inbox,
    shared: bool,
}

impl Registration {
    fn new(secret: &Secret, init: &(String, bool), store: &mut Credentials) -> Result<Self> {
        let location = Instance::<NotificationApplication>::generate_location(secret, init)
            .map_err(|error| {
                PistonError::Generic(format!("Cannot create notification location: {error:?}"))
            })?;
        let account = format!("application:{}", Id::hash(&location));
        if let Some(bytes) = store.get(&account) {
            let registration: Self = serde_json::from_slice(bytes).map_err(|_| {
                PistonError::Generic(
                    "Invalid notification registration in Keychain. Restore its backup.".into(),
                )
            })?;
            if registration.instance.location() != &location {
                return Err(PistonError::Generic(
                    "The Keychain notification location does not match this app.".into(),
                ));
            }
            return Ok(registration);
        }
        let instance = Instance::receive(secret.clone(), location).map_err(|error| {
            PistonError::Generic(format!("Cannot create notification contract: {error:?}"))
        })?;
        let inbox = Inbox::new(secret.clone()).map_err(|error| {
            PistonError::Generic(format!("Cannot create deployment inbox: {error:?}"))
        })?;
        let registration = Self { instance, inbox, shared: false };
        registration.save(store)?;
        Ok(registration)
    }

    async fn publish(
        &mut self, credentials: ApnsCredentials, refresh: bool,
        secret: Secret, init: (String, bool), store: &mut Credentials,
    ) -> Result<()> {
        if !refresh && self.shared && self.matches(&credentials) { return Ok(()); }
        let mut resolver = DefaultResolver::default();
        let identity = resolver.resolve(self.instance.location().server, None).await;
        let client = Client::new((*identity).clone(), "wss")
            .await
            .map_err(|error| PistonError::Generic(format!("Cannot connect to AIR: {error}")))?;
        self.synchronize(&client, &mut resolver, store).await?;
        if self.instance.confirmed().is_none() {
            self.instance = Instance::new(secret, init).map_err(|error| {
                PistonError::Generic(format!("Cannot initialize notification contract: {error:?}"))
            })?;
            self.synchronize(&client, &mut resolver, store).await?;
        }
        if !self.matches(&credentials) {
            if !self.instance.send(credentials.clone()) {
                return Err(PistonError::Generic(
                    "The deployment identity cannot update this notification contract.".into(),
                ));
            }
            self.synchronize(&client, &mut resolver, store).await?;
            if !self.matches(&credentials) {
                return Err(PistonError::Generic(
                    "AIR did not confirm acceptance of the APNs credentials.".into(),
                ));
            }
        }
        if !self.shared {
            // Reusing the encrypted inbox channel safely resumes writes after interruption.
            self.inbox
                .send(&mut resolver, Notifications::server(), *self.instance.location())
                .await;
            let mut requests = self.inbox.start();
            self.save(store)?;
            while !requests.is_empty() {
                for (kind, request) in requests {
                    let responses = client.send(request).await;
                    let response = responses.recv().await.map_err(|_| {
                        PistonError::Generic(
                            "AIR disconnected before the notification configuration was shared."
                                .into(),
                        )
                    })?;
                    if matches!(response, Response::InvalidSignature) {
                        return Err(PistonError::Generic(
                            "AIR rejected the notification inbox signature.".into(),
                        ));
                    }
                    self.inbox.response(&mut resolver, kind, response).await;
                }
                requests = self.inbox.requests();
                self.save(store)?;
            }
            self.shared = true;
            self.save(store)?;
        }
        Ok(())
    }

    async fn synchronize(&mut self, client: &Client, resolver: &mut DefaultResolver, store: &mut Credentials) -> Result<()> {
        let mut request = Some(self.instance.start());
        while let Some(outgoing) = request {
            self.save(store)?;
            let responses = client.send(outgoing).await;
            let response = responses.recv().await.map_err(|_| {
                PistonError::Generic(
                    "AIR disconnected before confirming notification setup.".into(),
                )
            })?;
            match self.instance.response(resolver, response).await {
                Output::Created(false, _) | Output::Read(false, _) | Output::Garbage => {
                    return Err(PistonError::Generic(
                        "AIR rejected the notification configuration update.".into(),
                    ));
                }
                _ => {}
            }
            request = self.instance.request();
        }
        self.save(store)
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

    fn save(&self, store: &mut Credentials) -> Result<()> {
        store
            .save(&format!("application:{}", self.instance.id()), serde_json::to_vec(self).unwrap())
    }
}
