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
        let path = fs::canonicalize(path).map_err(|error| {
            PistonError::CargoParseError(format!("Cannot open {}: {error}", path.display(),))
        })?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path { &self.path }

    pub fn setting(&self, section: &[&str]) -> Result<Option<Item>> {
        let (_, document) = self.read()?;
        let mut item = document.as_item();
        for name in ["package", "metadata", "piston"].iter().chain(section) {
            let table = item.as_table_like().ok_or_else(|| {
                PistonError::CargoParseError(format!(
                    "Expected a table for package.metadata.piston.{} in {}.",
                    section.join("."),
                    self.path.display(),
                ))
            })?;
            let Some(next) = table.get(name) else { return Ok(None); };
            item = next;
        }
        Ok(Some(item.clone()))
    }

    pub fn settings(&self, section: &[&str]) -> Result<HashMap<String, String>> {
        let Some(item) = self.setting(section)? else { return Ok(HashMap::new()); };
        let table = item.as_table_like().ok_or_else(|| {
            PistonError::CargoParseError(format!(
                "Expected a table for package.metadata.piston.{} in {}.",
                section.join("."),
                self.path.display(),
            ))
        })?;
        table
            .iter()
            .map(|(name, item)| {
                let value = item.as_str().ok_or_else(|| {
                    PistonError::CargoParseError(format!(
                        "package.metadata.piston.{}.{name} must be a string in {}.",
                        section.join("."),
                        self.path.display(),
                    ))
                })?;
                Ok((name.into(), value.into()))
            })
            .collect()
    }

    pub fn save<T: Clone + Into<Value>>(&self, section: &[&str], settings: &[(&str, T)]) -> Result<()> {
        let (original, mut document) = self.read()?;
        let mut item = document.as_item_mut();
        for name in ["package", "metadata", "piston"].iter().chain(section) {
            let table = item.as_table_like_mut().ok_or_else(|| {
                PistonError::CargoParseError(format!(
                    "Expected a table for package.metadata.piston.{} in {}.",
                    section.join("."),
                    self.path.display(),
                ))
            })?;
            if !table.contains_key(name) {
                let mut child = Table::new();
                child.set_implicit(true);
                table.insert(name, Item::Table(child));
            }
            item = table.get_mut(name).unwrap();
        }
        let table = item.as_table_like_mut().ok_or_else(|| {
            PistonError::CargoParseError(format!(
                "Expected a table for package.metadata.piston.{} in {}.",
                section.join("."),
                self.path.display(),
            ))
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
        let updated = document.to_string();
        if updated == original { return Ok(()); }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let temporary =
            self.path.with_file_name(format!(".Cargo.toml.piston-{}-{stamp}", std::process::id()));
        let result = (|| -> std::io::Result<()> {
            let permissions = fs::metadata(&self.path)?.permissions();
            if permissions.readonly() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Cargo.toml is read-only",
                ));
            }
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
            fs::set_permissions(&temporary, permissions)?;
            file.write_all(updated.as_bytes())?;
            file.sync_all()?;
            if fs::read_to_string(&self.path)? != original {
                return Err(std::io::Error::other(
                    "Cargo.toml changed during setup; rerun to save settings",
                ));
            }
            fs::rename(&temporary, &self.path)
        })();
        if result.is_err() { let _ = fs::remove_file(&temporary); }
        result.map_err(|error| {
            PistonError::WriteFileError(format!(
                "Cannot save Piston settings in {}: {error}",
                self.path.display(),
            ))
        })
    }

    fn read(&self) -> Result<(String, DocumentMut)> {
        let contents = fs::read_to_string(&self.path).map_err(|error| {
            PistonError::CargoParseError(format!("Cannot read {}: {error}", self.path.display()))
        })?;
        let document = contents.parse::<DocumentMut>().map_err(|error| {
            PistonError::CargoParseError(format!("Invalid {}: {error}", self.path.display()))
        })?;
        if document.get("package").and_then(Item::as_table_like).is_none() {
            return Err(PistonError::CargoParseError(format!(
                "{} has no application package. Select a package with --package.",
                self.path.display(),
            )));
        }
        Ok((contents, document))
    }
}
