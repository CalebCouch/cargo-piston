use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::manifest::Manifest;
use crate::Result;

pub mod cli;
pub mod app_store;
use app_store::AppStoreConnect;
pub use app_store::{AscApiKey, AppStoreRelease, IOSArtwork, TestFlight};
mod push;
pub use push::{PushBuild, PushPlatform, PushSigning};
pub mod notifications;
pub use notifications::NotificationSetup;
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
    pub fn new(api_key: AscApiKey, keystore_path: String, manifest: &Path) -> Result<Self> {
        Ok(Self { api_key, keystore_path, manifest: Manifest::new(manifest)? })
    }

    pub fn create_or_find_security_cert(&self, team_id_override: Option<String>) -> Result<(String, String)> {
        if let Some((cert_id, signing_identity)) = self.saved_certificate()? {
            let check = Command::new("security")
                .args(["find-identity", "-v", "-p", "codesigning"])
                .output()
                .map_err(|e| cli::Error::SecurityFindIdentity(e.to_string()))?;
            if !check.status.success() {
                return Err(cli::Error::SecurityFindIdentity(
                    String::from_utf8_lossy(&check.stderr).trim().to_string(),
                ).into());
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
            .map_err(|error| cli::Error::KeyChainUnlock(error.to_string()))?;
        if !unlocked.status.success() {
            return Err(cli::Error::KeyChainUnlock(
                String::from_utf8_lossy(&unlocked.stderr).trim().to_string(),
            ).into());
        }

        let app_store = AppStoreConnect::new(&self.api_key)?;

        if let Some(cert_name) = app_store.distribution_certificate()? {
            let check = Command::new("security")
                .args(["find-identity", "-v", "-p", "codesigning"])
                .output()
                .map_err(|e| crate::cli::Error::CommandSpawn { program: "security".into(), source: e })?;

            if !check.status.success() {
                return Err(cli::Error::SecurityFindIdentity(
                    String::from_utf8_lossy(&check.stderr).trim().to_string(),
                ).into());
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
            tempfile::tempdir().map_err(|error| crate::cli::Error::WriteFile(error.to_string()))?;
        let key_path = temporary.path().join("key.pem");
        let csr_path = temporary.path().join("request.csr");

        let generated = Command::new("openssl")
            .args(["genrsa", "-out"])
            .arg(&key_path)
            .arg("2048")
            .output()
            .map_err(|error| cli::Error::OpenSSLKeyGen(error.to_string()))?;
        if !generated.status.success() {
            return Err(cli::Error::OpenSSLKeyGen(
                String::from_utf8_lossy(&generated.stderr).into_owned(),
            ).into());
        }

        let generated = Command::new("openssl")
            .args(["req", "-new", "-key"])
            .arg(&key_path)
            .arg("-out")
            .arg(&csr_path)
            .args(["-subj", "/CN=Distribution Certificate"])
            .output()
            .map_err(|error| cli::Error::OpenSSLCSR(error.to_string()))?;
        if !generated.status.success() {
            return Err(cli::Error::OpenSSLCSR(
                String::from_utf8_lossy(&generated.stderr).into_owned(),
            ).into());
        }

        let csr_content = fs::read_to_string(&csr_path)
            .map_err(|e| crate::cli::Error::ReadFile { path: csr_path.clone(), source: e })?;

        let (cert_name, cert_der) = app_store.create_distribution_certificate(&csr_content)?;

        let cer_path = temporary.path().join("certificate.cer");
        fs::write(&cer_path, cert_der)
            .map_err(|e| crate::cli::Error::CreateFile { path: cer_path.clone(), source: e })?;

        let import_key = Command::new("security")
            .arg("import")
            .arg(&key_path)
            .args(["-k", &keychain_path, "-P", ""])
            .output()
            .map_err(|e| crate::cli::Error::CommandSpawn { program: "security".into(), source: e })?;

        if !import_key.status.success() {
            return Err(cli::Error::KeyChainImport(
                String::from_utf8_lossy(&import_key.stderr).trim().to_string(),
            ).into());
        }

        let import_cert = Command::new("security")
            .arg("import")
            .arg(&cer_path)
            .args(["-k", &keychain_path])
            .output()
            .map_err(|e| crate::cli::Error::CommandSpawn { program: "security".into(), source: e })?;

        if !import_cert.status.success() {
            return Err(cli::Error::KeyChainImport(
                String::from_utf8_lossy(&import_cert.stderr).trim().to_string(),
            ).into());
        }

        let check = Command::new("security")
            .args(["find-identity", "-v", "-p", "codesigning"])
            .output()
            .map_err(|e| crate::cli::Error::CommandSpawn { program: "security".into(), source: e })?;
        if !check.status.success() {
            return Err(cli::Error::SecurityFindIdentity(
                String::from_utf8_lossy(&check.stderr).trim().to_string(),
            ).into());
        }
        let output = String::from_utf8_lossy(&check.stdout);
        let team_id = match team_id_override {
            Some(team_id) => team_id,
            None => Self::parse_team_id(&output, &cert_name)?,
        };

        self.save_certificate(&team_id, &cert_name)?;
        Ok((team_id, cert_name))
    }

    // Distribution capabilities are not yet added to the minimal entitlements file.
    pub fn is_device_provisioned(app_bundle_path: &Path, device_id: &str, idp_path: &str, provision_cache: &PathBuf) -> Result<bool> {
        let entries = fs::read_dir(provision_cache)
            .map_err(|e| crate::cli::Error::ReadDir { path: provision_cache.clone(), source: e })?;

        let mut profile_files = vec![];
        for entry in entries {
            let entry = entry.map_err(|e| crate::cli::Error::MapDir {
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
                .map_err(|e| cli::Error::QueryProvision {
                    path: profile_path.to_path_buf(),
                    source: e,
                })?;

            let xml = String::from_utf8_lossy(&output.stdout);
            let target = format!("<string>{}</string>", device_id);
            if xml.contains(&target) {
                let list_res = Command::new(idp_path).args(["list", "--udid", device_id]).output()
                    .map_err(|source| crate::cli::Error::CommandSpawn { program: idp_path.into(), source })?;
                if !list_res.status.success() {
                    return Err(crate::cli::Error::CommandFailed {
                        program: idp_path.into(), output: list_res,
                    }.into());
                }
                let installed = String::from_utf8_lossy(&list_res.stdout);
                let target_partial = profile_path.file_name().unwrap().to_str().unwrap();
                let target =
                    target_partial.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(target_partial);
                if installed.contains(target) {
                    let target_path = app_bundle_path.join("embedded.mobileprovision");
                    fs::copy(&profile_path, &target_path).map_err(|e| {
                        crate::cli::Error::CopyFile {
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

    pub fn sign_app_bundle(app_bundle_path: &Path, security_profile: &str, target: AppleSigningTarget) -> Result<()> {
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
            .map_err(|e| crate::cli::Error::CommandSpawn { program: "codesign".into(), source: e })?;

        if !status.status.success() {
            return Err(cli::Error::Codesign(
                String::from_utf8_lossy(&status.stderr).trim().to_string(),
            ).into());
        }

        Ok(())
    }

    pub fn ensure_entitlements(app_bundle_path: &Path) -> Result<()> {
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
                crate::cli::Error::CreateFile { path: entitlements_path.clone(), source: e }
            })?;

            return Ok(());
        }
        let cms = Command::new("security")
            .args(["cms", "-D", "-i", embedded.to_str().unwrap()])
            .output()
            .map_err(|e| crate::cli::Error::CommandSpawn { program: "security".into(), source: e })?;

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
            .map_err(|e| crate::cli::Error::CommandSpawn { program: "plutil".into(), source: e })?;

        if let Some(mut stdin) = plutil.stdin.take() {
            stdin.write_all(&cms.stdout).map_err(cli::Error::ApplePlistInput)?;
        }

        let result = plutil.wait_with_output().map_err(cli::Error::ApplePlistWait)?;
        if !result.status.success() {
            return Err(crate::cli::Error::CommandFailed { program: "plutil".into(), output: result }.into());
        }

        Ok(())
    }

    fn saved_certificate(&self) -> Result<Option<(String, String)>> {
        let mut settings = self.manifest.settings(&["apple", "signing"])?;
        if settings.is_empty() { return Ok(None); }
        let team = settings.remove("team").ok_or_else(|| {
            crate::cli::Error::MissingSetting { key: "package.metadata.piston.apple.signing.team".into() }
        })?;
        let identity = settings.remove("identity").ok_or_else(|| {
            crate::cli::Error::MissingSetting { key: "package.metadata.piston.apple.signing.identity".into() }
        })?;
        Ok(Some((team, identity)))
    }

    fn save_certificate(&self, team: &str, identity: &str) -> Result<()> {
        self.manifest.save(&["apple", "signing"], &[("team", team), ("identity", identity)])
    }

    fn parse_team_id(stdout: &str, cert_name: &str) -> Result<String> {
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
                return Err(cli::Error::SigningCertificateMissing { name: cert_name.into() }.into());
            }
            _ => {
                return Err(cli::Error::SigningCertificateAmbiguous {
                    name: cert_name.into(), teams: team_ids_for_cert.into_iter().collect(),
                }.into());
            }
        };
        Ok(team_id)
    }
}
