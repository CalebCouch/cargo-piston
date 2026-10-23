use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use crate::host::Host;
use crate::manifest::Manifest;
use crate::{PistonError, Result};
use super::{AppStoreConnect, AscApiKey};

pub mod cli;

pub struct AccountSetup {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub settings: HashMap<String, String>,
    pub explicit: bool,
}

impl AccountSetup {
    pub fn new(root: &Path, reconfigure: bool) -> Result<Self> {
        let root = fs::canonicalize(root)
            .map_err(|error| PistonError::Generic(format!("Cannot locate project: {error}")))?;
        let manifest = Manifest::new(&root.join("Cargo.toml"))?;
        let path = root.join(".env");
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(PistonError::Generic(format!(
                    "Cannot read {}: {error}",
                    path.display()
                )));
            }
        };
        let mut settings: HashMap<String, String> = contents
            .lines()
            .filter_map(|line| {
                let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
                if line.starts_with('#') { return None; }
                let (key, value) = line.split_once('=')?;
                let key = key.trim().to_ascii_lowercase();
                if !["asc_key_id", "asc_issuer_id", "asc_key_path"].contains(&key.as_str()) {
                    return None;
                }
                Some((key, value.trim().trim_matches(['\'', '"']).into()))
            })
            .collect();
        for name in ["ASC_KEY_ID", "ASC_ISSUER_ID", "ASC_KEY_PATH"] {
            if let Ok(value) = env::var(name) { settings.insert(name.to_ascii_lowercase(), value); }
        }
        let explicit = !settings.is_empty();
        if !explicit && !reconfigure {
            settings = manifest
                .settings(&["app_store_connect"])?
                .into_iter()
                .map(|(name, value)| (format!("asc_{name}"), value))
                .collect();
        }
        Ok(Self { root, manifest, settings, explicit })
    }
}

impl AscApiKey {
    fn configured(root: &Path, settings: &HashMap<String, String>, report: &mut dyn FnMut(String)) -> Result<Self> {
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
            return Err(PistonError::Generic(
                "Provide a 10-character Team API Key ID or an AuthKey_<KEY_ID>.p8 file path."
                    .into(),
            ));
        }
        let issuer = settings.get("asc_issuer_id").map(String::as_str).unwrap_or("");
        let parts: Vec<_> = issuer.split('-').collect();
        if parts.len() != 5
            || !parts.iter().zip([8, 4, 4, 4, 12]).all(|(part, length)| {
                part.len() == length && part.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(PistonError::Generic(
                "Enter the Issuer ID shown above Team Keys (ASC_ISSUER_ID), in UUID format.".into(),
            ));
        }
        let path = match path {
            Some(path) => path,
            None => {
                let paths = Self::find_key_paths(root, &key_id, report)?;
                match paths.as_slice() {
                    [path] => path.clone(),
                    [] => {
                        return Err(PistonError::Generic(format!(
                            "Cannot find AuthKey_{key_id}.p8 using system search or the standard key folders. \
                         Set ASC_KEY_PATH or use the interactive setup.",
                        )));
                    }
                    _ => {
                        return Err(PistonError::Generic(format!(
                            "Multiple files match Key ID {key_id}. Set ASC_KEY_PATH to choose one:\n{}",
                            paths
                                .iter()
                                .map(|path| path.display().to_string())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )));
                    }
                }
            }
        };
        let path = fs::canonicalize(&path).map_err(|error| {
            PistonError::Generic(format!(
                "Cannot read the API key at {}: {error}. Choose the downloaded .p8 file.",
                path.display(),
            ))
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
                    .ok_or_else(|| PistonError::Generic("HOME is unavailable.".into()))?,
            )
            .join(relative))
        } else {
            Ok(root.join(value))
        }
    }

    fn find_key_paths(root: &Path, key_id: &str, report: &mut dyn FnMut(String)) -> Result<Vec<PathBuf>> {
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
            Err(error) => report(format!("{error} Checking standard key folders directly.")),
        }
        let mut paths = Vec::new();
        for path in candidates {
            let path = match fs::canonicalize(&path) {
                Ok(path) => path,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    report(format!(
                        "Cannot search {}: permission denied. Supply the key file path if stored there.",
                        path.parent().unwrap().display()
                    ));
                    continue;
                }
                Err(error) => {
                    return Err(PistonError::Generic(format!(
                        "Cannot search {}: {error}",
                        path.display(),
                    )));
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
