use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::PistonError;
use crate::manifest::Manifest;

pub mod app_store;
use app_store::AppStoreConnect;
pub use app_store::{AscApiKey, AppStoreRelease, IOSArtwork, TestFlight};
mod push;
pub use push::{PushBuild, PushPlatform, PushSigning};
mod notifications;
pub use notifications::{NotificationSetup, cli::NotificationCli};
pub mod xcode;
pub use xcode::XcodeSigning;

pub enum AppleSigningTarget { Ios, MacAppStore, MacExternal, }

#[derive(Debug)]
pub struct AppleSigning {
    api_key: AscApiKey,
    keystore_path: String,
    manifest: Manifest,
}

impl AppleSigning {
    pub fn new(api_key: AscApiKey, keystore_path: String, manifest: &Path) -> Result<Self, PistonError> {
        Ok(Self { api_key, keystore_path, manifest: Manifest::new(manifest)? })
    }

    pub fn create_or_find_security_cert(&self, team_id_override: Option<String>) -> Result<(String, String), PistonError> {
        if let Some((cert_id, signing_identity)) = self.saved_certificate()? {
            let check = Command::new("security")
                .args(["find-identity", "-v", "-p", "codesigning"])
                .output()
                .map_err(|e| PistonError::SecurityFindIdentityError(e.to_string()))?;
            if !check.status.success() {
                return Err(PistonError::SecurityFindIdentityError(
                    String::from_utf8_lossy(&check.stderr).trim().to_string(),
                ));
            }
            let output = String::from_utf8_lossy(&check.stdout);
            if output.contains(&signing_identity)
                && team_id_override.as_ref().is_none_or(|team| team == &cert_id)
            { return Ok((cert_id, signing_identity)); }
        }
        let keychain_path = format!("{}/login.keychain-db", self.keystore_path);
        let unlocked = Command::new("security")
            .args(["unlock-keychain", &keychain_path])
            .output()
            .map_err(|error| PistonError::KeyChainUnlockError(error.to_string()))?;
        if !unlocked.status.success() {
            return Err(PistonError::KeyChainUnlockError(
                String::from_utf8_lossy(&unlocked.stderr).trim().to_string(),
            ));
        }

        let app_store = AppStoreConnect::new(&self.api_key)?;

        if let Some(cert_name) = app_store.distribution_certificate()? {
            let check = Command::new("security")
                .args(["find-identity", "-v", "-p", "codesigning"])
                .output()
                .map_err(|e| {
                    PistonError::KeyChainImportError(format!("Failed to check keychain: {}", e))
                })?;

            if !check.status.success() {
                return Err(PistonError::SecurityFindIdentityError(
                    String::from_utf8_lossy(&check.stderr).trim().to_string(),
                ));
            }
            let output = String::from_utf8_lossy(&check.stdout);

            if output.contains(&cert_name) {
                let team_id = match team_id_override {
                    Some(team_id) => team_id,
                    None => Self::parse_team_id(&output, &cert_name)?,
                };
                self.save_certificate(&team_id, &cert_name)?;
                return Ok((team_id, cert_name));
            }
        }

        let temporary =
            tempfile::tempdir().map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        let key_path = temporary.path().join("key.pem");
        let csr_path = temporary.path().join("request.csr");

        let generated = Command::new("openssl")
            .args(["genrsa", "-out"])
            .arg(&key_path)
            .arg("2048")
            .output()
            .map_err(|error| PistonError::OpenSSLKeyGenError(error.to_string()))?;
        if !generated.status.success() {
            return Err(PistonError::OpenSSLKeyGenError(
                String::from_utf8_lossy(&generated.stderr).into_owned(),
            ));
        }

        let generated = Command::new("openssl")
            .args(["req", "-new", "-key"])
            .arg(&key_path)
            .arg("-out")
            .arg(&csr_path)
            .args(["-subj", "/CN=Distribution Certificate"])
            .output()
            .map_err(|error| PistonError::OpenSSLCSRError(error.to_string()))?;
        if !generated.status.success() {
            return Err(PistonError::OpenSSLCSRError(
                String::from_utf8_lossy(&generated.stderr).into_owned(),
            ));
        }

        let csr_content = fs::read_to_string(&csr_path)
            .map_err(|e| PistonError::ReadCSRError(format!("Failed to read CSR: {}", e)))?;

        let (cert_name, cert_der) = app_store.create_distribution_certificate(&csr_content)?;

        let cer_path = temporary.path().join("certificate.cer");
        fs::write(&cer_path, cert_der)
            .map_err(|e| PistonError::WriteFileError(format!("Write failed: {}", e)))?;

        let import_key = Command::new("security")
            .arg("import")
            .arg(&key_path)
            .args(["-k", &keychain_path, "-P", ""])
            .output()
            .map_err(|e| PistonError::KeyChainImportError(format!("Key import failed: {}", e)))?;

        if !import_key.status.success() {
            return Err(PistonError::KeyChainImportError(
                String::from_utf8_lossy(&import_key.stderr).trim().to_string(),
            ));
        }

        let import_cert = Command::new("security")
            .arg("import")
            .arg(&cer_path)
            .args(["-k", &keychain_path])
            .output()
            .map_err(|e| PistonError::KeyChainImportError(format!("Cert import failed: {}", e)))?;

        if !import_cert.status.success() {
            return Err(PistonError::KeyChainImportError(
                String::from_utf8_lossy(&import_cert.stderr).trim().to_string(),
            ));
        }

        let check = Command::new("security")
            .args(["find-identity", "-v", "-p", "codesigning"])
            .output()
            .map_err(|e| {
                PistonError::KeyChainImportError(format!("Failed to check keychain: {}", e))
            })?;
        if !check.status.success() {
            return Err(PistonError::SecurityFindIdentityError(
                String::from_utf8_lossy(&check.stderr).trim().to_string(),
            ));
        }
        let output = String::from_utf8_lossy(&check.stdout);
        let team_id = match team_id_override {
            Some(team_id) => team_id,
            None => Self::parse_team_id(&output, &cert_name)?,
        };

        self.save_certificate(&team_id, &cert_name)?;
        Ok((team_id, cert_name))
    }

    pub fn provision_ios_device(
        &self, device_id: &str, bundle_id: &str, app_name: &str, certificate_id: &str,
        app_bundle_path: &PathBuf, ideviceprovision_path: &str, provision_cache: &PathBuf,
    ) -> Result<(), PistonError> {
        let app_store = AppStoreConnect::new(&self.api_key)?;
        let (profile_name, profile_data) =
            app_store.ios_ad_hoc_profile(device_id, bundle_id, app_name, certificate_id)?;

        let profile_path = format!("{}.mobileprovision", profile_name);
        fs::write(&profile_path, profile_data)
            .map_err(|e| PistonError::WriteFileError(format!("Failed to write profile: {}", e)))?;

        let _ = fs::create_dir_all(provision_cache);
        let cached_profile = provision_cache.join(format!("{}.mobileprovision", profile_name));
        let _ = fs::copy(&profile_path, &cached_profile);

        let embedded_path = format!("{}/embedded.mobileprovision", app_bundle_path.display());
        fs::copy(&profile_path, &embedded_path)
            .map_err(|e| PistonError::WriteFileError(format!("Failed to embed profile: {}", e)))?;

        let install = Command::new(ideviceprovision_path)
            .args(["install", &embedded_path, "--udid", device_id])
            .output()
            .map_err(|e| {
                PistonError::DeviceProvisionError(format!("ideviceprovision failed: {}", e))
            })?;

        if !install.status.success() {
            return Err(PistonError::DeviceProvisionError(
                String::from_utf8_lossy(&install.stderr).trim().to_string(),
            ));
        }

        Self::ensure_entitlements(app_bundle_path)?;

        let _ = Command::new("xattr")
            .args(["-cr", app_bundle_path.to_str().unwrap()])
            .status()
            .map_err(|e| PistonError::Generic(format!("xattr error: {}", e)))?;

        let _ = fs::remove_file(profile_path);

        Ok(())
    }

    // Distribution capabilities are not yet added to the minimal entitlements file.
    pub fn is_device_provisioned(app_bundle_path: &PathBuf, device_id: &str, idp_path: &str, provision_cache: &PathBuf) -> Result<bool, PistonError> {
        let entries = fs::read_dir(provision_cache)
            .map_err(|e| PistonError::ReadDirError { path: provision_cache.clone(), source: e })?;

        let mut profile_files = vec![];
        for entry in entries {
            let entry = entry.map_err(|e| PistonError::MapDirError {
                path: provision_cache.to_path_buf(),
                source: e,
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".mobileprovision") { profile_files.push(entry.path()); }
        }

        if profile_files.is_empty() { return Ok(false); }

        for profile_path in profile_files {
            let output = Command::new("security")
                .args(["cms", "-D", "-i", profile_path.to_str().unwrap()])
                .output()
                .map_err(|e| PistonError::QueryProvisionError {
                    path: profile_path.to_path_buf(),
                    source: e,
                })?;

            let xml = String::from_utf8_lossy(&output.stdout);
            let target = format!("<string>{}</string>", device_id);
            if xml.contains(&target) {
                let list = Command::new(idp_path).args(["list", "--udid", device_id]).output();
                let list_res = list.unwrap();
                if !list_res.status.success() {
                    return Err(PistonError::Generic(
                        "Failed to list provisioning profiles with IDP".into(),
                    ));
                }
                let installed = String::from_utf8_lossy(&list_res.stdout);
                let target_partial = profile_path.file_name().unwrap().to_str().unwrap();
                let target =
                    target_partial.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(target_partial);
                if installed.contains(target) {
                    let target_path = app_bundle_path.join("embedded.mobileprovision");
                    fs::copy(&profile_path, &target_path).map_err(|e| {
                        PistonError::CopyFileError {
                            input_path: profile_path,
                            output_path: target_path,
                            source: e,
                        }
                    })?;

                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    pub fn sign_app_bundle(app_bundle_path: &PathBuf, security_profile: &str, target: AppleSigningTarget) -> Result<(), PistonError> {
        let bundle_path = app_bundle_path.display().to_string();

        let code_signature_dir = app_bundle_path.join("_CodeSignature");
        if code_signature_dir.exists() { let _ = fs::remove_dir_all(&code_signature_dir); }

        Self::ensure_entitlements(app_bundle_path)?;
        let app_bundle_parent = app_bundle_path.parent().unwrap();

        let entitlements_path = format!("{}/entitlements.plist", app_bundle_parent.display());

        // Mac App Store signing must omit the hardened runtime option.
        let args = match target {
            AppleSigningTarget::MacAppStore => {
                vec![
                    "--force",
                    "--deep",
                    "--sign",
                    security_profile,
                    "--entitlements",
                    &entitlements_path,
                    "--timestamp",
                    "--generate-entitlement-der",
                    &bundle_path,
                ]
            }
            AppleSigningTarget::Ios | AppleSigningTarget::MacExternal => {
                vec![
                    "--force",
                    "--deep",
                    "--options=runtime",
                    "--sign",
                    security_profile,
                    "--entitlements",
                    &entitlements_path,
                    "--timestamp=none",
                    "--generate-entitlement-der",
                    "--preserve-metadata=identifier,entitlements,flags,runtime",
                    &bundle_path,
                ]
            }
        };

        let status = Command::new("codesign")
            .args(&args)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| {
                PistonError::CodesignError(format!("Failed to sign outer bundle: {}", e))
            })?;

        if !status.status.success() {
            return Err(PistonError::CodesignError(
                String::from_utf8_lossy(&status.stderr).trim().to_string(),
            ));
        }

        Ok(())
    }

    fn ensure_entitlements(app_bundle_path: &PathBuf) -> Result<(), PistonError> {
        let embedded = app_bundle_path.join("embedded.mobileprovision");
        let app_bundle_parent = app_bundle_path.parent().unwrap();
        let entitlements_path = app_bundle_parent.join("entitlements.plist");
        if !embedded.exists() {
            let content = r#"<?xml version="1.0" encoding="UTF-8"?>
            <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
            <plist version="1.0">
            <dict>
            </dict>
            </plist>"#;

            fs::write(&entitlements_path, content.trim()).map_err(|e| {
                PistonError::WriteFileError(format!("Failed to write entitlements.plist: {}", e))
            })?;

            return Ok(());
        }
        let cms = Command::new("security")
            .args(["cms", "-D", "-i", embedded.to_str().unwrap()])
            .output()
            .map_err(|e| PistonError::Generic(format!("security cms failed: {}", e)))?;

        let mut plutil = Command::new("plutil")
            .args([
                "-extract",
                "Entitlements",
                "xml1",
                "-o",
                entitlements_path.to_str().unwrap(),
                "-",
            ])
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| PistonError::Generic(format!("plutil spawn failed: {}", e)))?;

        if let Some(mut stdin) = plutil.stdin.take() {
            stdin.write_all(&cms.stdout).map_err(|e| {
                PistonError::WritePlUtilError(format!("Failed to write plutil file: {}", e))
            })?;
        }

        let result = plutil
            .wait_with_output()
            .map_err(|e| PistonError::Generic(format!("plutil failed: {}", e)))?;
        if !result.status.success() {
            return Err(PistonError::Generic("Failed to extract entitlements.plist".to_string()));
        }

        Ok(())
    }

    fn saved_certificate(&self) -> Result<Option<(String, String)>, PistonError> {
        let mut settings = self.manifest.settings(&["apple", "signing"])?;
        if settings.is_empty() { return Ok(None); }
        let team = settings.remove("team").ok_or_else(|| {
            PistonError::Generic("Missing package.metadata.piston.apple.signing.team.".into())
        })?;
        let identity = settings.remove("identity").ok_or_else(|| {
            PistonError::Generic("Missing package.metadata.piston.apple.signing.identity.".into())
        })?;
        Ok(Some((team, identity)))
    }

    fn save_certificate(&self, team: &str, identity: &str) -> Result<(), PistonError> {
        self.manifest.save(&["apple", "signing"], &[("team", team), ("identity", identity)])
    }

    fn parse_team_id(stdout: &str, cert_name: &str) -> Result<String, PistonError> {
        let team_ids_for_cert: HashSet<String> = stdout
            .lines()
            .filter_map(|line| {
                let (_, quoted) = line.split_once('"')?;
                let (quoted, _) = quoted.rsplit_once('"')?;
                let (name, rest) = quoted.rsplit_once(" (")?;
                if name.trim() != cert_name { return None; }
                let (team_id, _) = rest.split_once(')')?;
                Some(team_id.to_string())
            })
            .collect();

        let team_id = match team_ids_for_cert.len() {
            1 => team_ids_for_cert.into_iter().next().unwrap(),
            0 => {
                return Err(PistonError::Generic(format!(
                    "No certificate matching the name '{}' was found in the keychain.",
                    cert_name
                )));
            }
            _ => {
                return Err(PistonError::Generic(format!(
                    "Multiple certificates with the exact same name '{}' were found but with different Team IDs: {:?}. \
                    Team ID cannot be uniquely determined. Please designate a team ID in your .env with 'team_id=<team_id>' ",
                    cert_name, team_ids_for_cert
                )));
            }
        };
        Ok(team_id)
    }
}
