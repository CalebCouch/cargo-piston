use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use crate::developer::Credentials;
use crate::host::Host;
use crate::manifest::Manifest;
use crate::{PistonError, Result};
use super::{AppStoreConnect, AscApiKey};

pub mod cli;
pub use cli::Cli;

pub struct AccountSetup {
    root: PathBuf,
    manifest: Manifest,
    settings: HashMap<String, String>,
    team: String,
    store: Credentials,
    runtime: tokio::runtime::Runtime,
}

impl AccountSetup {
    pub(crate) fn new(root: &Path, team: String, store: Credentials) -> Result<Self> {
        let root = fs::canonicalize(root)
            .map_err(|source| PistonError::ReadFile { path: root.into(), source })?;
        let manifest = Manifest::new(&root.join("Cargo.toml"))?;
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()
            .map_err(|error| PistonError::Generic(error.to_string()))?;
        let settings = ["ASC_KEY_ID", "ASC_ISSUER_ID", "ASC_KEY_PATH"].into_iter()
            .filter_map(|name| env::var(name).ok().map(|value| (name.to_ascii_lowercase(), value)))
            .collect();
        Ok(Self { root, manifest, settings, team, store, runtime })
    }

    fn load(&mut self, replace: bool) -> Result<Option<AscApiKey>> {
        self.runtime.block_on(self.store.load(&self.team))?;
        if replace { return Ok(None); }
        self.store.get(&self.team, "app-store-connect")
            .map(|bytes| serde_json::from_slice(bytes).map_err(PistonError::StoreCredentialsInvalid))
            .transpose()
    }

    fn save(&mut self, key: &AscApiKey) -> Result<()> {
        let bytes = serde_json::to_vec(key).unwrap();
        self.runtime.block_on(self.store.save(&self.team, "app-store-connect", bytes))?;
        self.remember()
    }

    fn remember(&self) -> Result<()> {
        self.manifest.save(&["ios"], &[("team", &self.team)])
    }
}

impl AscApiKey {
    fn configured(root: &Path, settings: &HashMap<String, String>, report: &mut dyn FnMut(PistonError)) -> Result<Self> {
        let path = settings
            .get("asc_key_path")
            .filter(|path| !path.is_empty())
            .map(|path| Self::key_path(root, path))
            .transpose()?;
        let key_id = settings
            .get("asc_key_id")
            .filter(|id| !id.is_empty())
            .map(String::as_str)
            .or_else(|| path.as_deref().and_then(Self::id_from_path))
            .unwrap_or("")
            .to_owned();
        if !Self::valid_id(&key_id) {
            return Err(PistonError::StoreKeyIdInvalid);
        }
        let issuer = settings.get("asc_issuer_id").map(String::as_str).unwrap_or("");
        let parts: Vec<_> = issuer.split('-').collect();
        if parts.len() != 5
            || !parts.iter().zip([8, 4, 4, 4, 12]).all(|(part, length)| {
                part.len() == length && part.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(PistonError::StoreIssuerInvalid);
        }
        let path = match path {
            Some(path) => path,
            None => {
                let paths = Self::find_key_paths(root, &key_id, report)?;
                match paths.as_slice() {
                    [path] => path.clone(),
                    [] => {
                        return Err(PistonError::StoreKeyMissing { key_id });
                    }
                    _ => {
                        return Err(PistonError::StoreKeyAmbiguous { key_id, paths });
                    }
                }
            }
        };
        let path = fs::canonicalize(&path).map_err(|error| {
            PistonError::StoreKeyUnreadable { path: path.clone(), source: error }
        })?;
        let mut settings = settings.clone();
        settings.insert("asc_key_id".into(), key_id.into());
        settings.insert("asc_key_path".into(), path.to_string_lossy().into_owned());
        let key = Self::from_hm(&settings)?;
        key.token()?;
        Ok(key)
    }

    fn valid_id(value: &str) -> bool {
        value.len() == 10 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
    }

    fn id_from_path(path: &Path) -> Option<&str> {
        let id = path.file_name()?.to_str()?.strip_prefix("AuthKey_")?.strip_suffix(".p8")?;
        Self::valid_id(id).then_some(id)
    }

    fn key_path(root: &Path, value: &str) -> Result<PathBuf> {
        let value = value.trim().trim_matches(['\'', '"']);
        if let Some(relative) = value.strip_prefix("~/") {
            Ok(PathBuf::from(
                env::var_os("HOME")
                    .ok_or_else(|| PistonError::MissingEnvironment { name: "HOME" })?,
            )
            .join(relative))
        } else {
            Ok(root.join(value))
        }
    }

    fn find_key_paths(root: &Path, key_id: &str, report: &mut dyn FnMut(PistonError)) -> Result<Vec<PathBuf>> {
        let mut folders = vec![root.join("private_keys")];
        if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
            folders.extend(
                [".appstoreconnect/private_keys", ".private_keys", "private_keys", "Downloads"]
                    .into_iter()
                    .map(|folder| home.join(folder)),
            );
        }
        let name = format!("AuthKey_{key_id}.p8");
        let mut candidates: Vec<_> = folders.into_iter().map(|folder| folder.join(&name)).collect();
        match Host::find_files(&name) {
            Ok(paths) => candidates.extend(paths),
            Err(error) => report(PistonError::StoreKeySearchFallback(Box::new(error))),
        }
        let mut paths = Vec::new();
        for path in candidates {
            let path = match fs::canonicalize(&path) {
                Ok(path) => path,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    report(PistonError::StoreKeySearchDenied { path: path.parent().unwrap().into() });
                    continue;
                }
                Err(error) => {
                    return Err(PistonError::ReadFile { path, source: error });
                }
            };
            if path.is_file() && !paths.contains(&path) { paths.push(path); }
        }
        Ok(paths)
    }

    fn authorize(self) -> Result<Self> {
        AppStoreConnect::new(&self)?.request("GET", "/v1/apps", &[("limit", "1")], None)?;
        Ok(self)
    }
}
