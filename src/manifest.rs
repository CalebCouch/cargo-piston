use toml_edit::{DocumentMut, Item, Table, Value, value};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::{PistonError, Result};

#[derive(Debug)]
pub struct Manifest {
    path: PathBuf,
}

impl Manifest {
    pub fn new(path: &Path) -> Result<Self> {
        let path = fs::canonicalize(path).map_err(|source| PistonError::ReadFile {
            path: path.to_path_buf(), source,
        })?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path { &self.path }

    pub fn setting(&self, section: &[&str]) -> Result<Option<Item>> {
        let (_, document) = self.read()?;
        let mut item = document.as_item();
        for name in ["package", "metadata", "piston"].iter().chain(section) {
            let Some(table) = item.as_table_like() else { return Ok(None); };
            let Some(next) = table.get(name) else { return Ok(None); };
            item = next;
        }
        Ok(Some(item.clone()))
    }

    pub fn settings(&self, section: &[&str]) -> Result<HashMap<String, String>> {
        let Some(item) = self.setting(section)? else { return Ok(HashMap::new()); };
        let Some(table) = item.as_table_like() else { return Ok(HashMap::new()); };
        Ok(table.iter().filter_map(|(name, item)| {
            item.as_str().map(|value| (name.into(), value.into()))
        }).collect())
    }

    pub fn save<T: Clone + Into<Value>>(&self, section: &[&str], settings: &[(&str, T)]) -> Result<()> {
        let (original, mut document) = self.read()?;
        let mut item = document.as_item_mut();
        for (index, name) in ["package", "metadata", "piston"].iter().chain(section).enumerate() {
            let table = item.as_table_like_mut().ok_or_else(|| PistonError::ManifestTableRequired {
                path: self.path.clone(), section: section.join("."),
            })?;
            if !table.contains_key(name)
                || (index >= 2 && table.get(name).unwrap().as_table_like().is_none())
            {
                let mut child = Table::new();
                child.set_implicit(true);
                table.insert(name, Item::Table(child));
            }
            item = table.get_mut(name).unwrap();
        }
        let table = item.as_table_like_mut().ok_or_else(|| PistonError::ManifestTableRequired {
            path: self.path.clone(), section: section.join("."),
        })?;
        for (name, setting) in settings {
            let existing = table.get(name);
            let mut replacement = value(setting.clone());
            if existing.is_some_and(|existing| {
                existing.as_str().zip(replacement.as_str()).is_some_and(|(old, new)| old == new)
                    || existing
                        .as_bool()
                        .zip(replacement.as_bool())
                        .is_some_and(|(old, new)| old == new)
            }) {
                continue;
            }
            if let Some(existing) = existing.and_then(Item::as_value) {
                *replacement.as_value_mut().unwrap().decor_mut() = existing.decor().clone();
            }
            table.insert(name, replacement);
        }
        self.write(&original, &document.to_string())
    }

    pub fn remove(&self, section: &[&str], name: &str) -> Result<()> {
        let (original, mut document) = self.read()?;
        let mut item = document.as_item_mut();
        for name in ["package", "metadata", "piston"].iter().chain(section) {
            let Some(next) = item.as_table_like_mut().and_then(|table| table.get_mut(name))
            else { return Ok(()); };
            item = next;
        }
        let Some(table) = item.as_table_like_mut() else { return Ok(()); };
        if table.remove(name).is_none() { return Ok(()); }
        self.write(&original, &document.to_string())
    }

    fn write(&self, original: &str, updated: &str) -> Result<()> {
        if updated == original { return Ok(()); }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let temporary =
            self.path.with_file_name(format!(".Cargo.toml.piston-{}-{stamp}", std::process::id()));
        let result = (|| -> Result<()> {
            let permissions = fs::metadata(&self.path).map_err(|source| PistonError::FileMetadata {
                path: self.path.clone(), source,
            })?.permissions();
            if permissions.readonly() {
                return Err(PistonError::ManifestReadOnly { path: self.path.clone() });
            }
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)
                .map_err(|source| PistonError::ManifestWrite { path: self.path.clone(), source })?;
            fs::set_permissions(&temporary, permissions)
                .and_then(|_| file.write_all(updated.as_bytes()))
                .and_then(|_| file.sync_all())
                .map_err(|source| PistonError::ManifestWrite { path: self.path.clone(), source })?;
            let current = fs::read_to_string(&self.path)
                .map_err(|source| PistonError::ReadFile { path: self.path.clone(), source })?;
            if current != original {
                return Err(PistonError::ManifestChanged { path: self.path.clone() });
            }
            fs::rename(&temporary, &self.path)
                .map_err(|source| PistonError::ManifestWrite { path: self.path.clone(), source })
        })();
        if result.is_err() { let _ = fs::remove_file(&temporary); }
        result
    }

    fn read(&self) -> Result<(String, DocumentMut)> {
        let contents = fs::read_to_string(&self.path).map_err(|source| PistonError::ReadFile {
            path: self.path.clone(), source,
        })?;
        let document = contents.parse::<DocumentMut>().map_err(|source| PistonError::ManifestParse {
            path: self.path.clone(), source,
        })?;
        if document.get("package").and_then(Item::as_table_like).is_none() {
            return Err(PistonError::ManifestPackageMissing { path: self.path.clone() });
        }
        Ok((contents, document))
    }
}
