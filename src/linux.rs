use backhand::{FilesystemWriter, NodeHeader};
use cargo_metadata::{Metadata, MetadataCommand};
use std::collections::HashMap;
use std::fs::{self, File, copy, create_dir_all};
use std::io::{self, Cursor, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::project::Project;
use crate::error::PistonError;
use crate::host::LinuxToolchain;

pub mod cli;
#[cfg(target_os = "macos")]
mod macos;

pub struct LinuxBuilder {
    release: bool,
    appimage: bool,
    target: String,
    cwd: PathBuf,
    output_path: PathBuf,
    icon_path: String,
    assets: String,
    toolchain: LinuxToolchain,
    gpg_path: Option<String>,
    app_name: String,
    key_id: Option<String>,
    key_pass: Option<String>,
    runtime_path: Option<String>,
}

impl LinuxBuilder {
    fn new(release: bool, appimage: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<Self, PistonError> {
        let gpg_path: Option<String> = env_vars.get("gpg_path").cloned();
        let key_id: Option<String> = env_vars.get("linux_gpg_key_id").cloned();
        let key_pass: Option<String> = env_vars.get("linux_gpg_key_pass").cloned();
        let runtime_path: Option<String> = env_vars.get("linux_runtime_path").cloned();
        let metadata: Metadata = MetadataCommand::new()
            .current_dir(&cwd)
            .exec()
            .map_err(|e| PistonError::CargoParseError(e.to_string()))?;

        let icon_path = Project::get_icon_path(&metadata, &cwd);
        let assets = Project::get_assets_path(&metadata);
        let app_name = Project::get_app_name(&metadata)?;
        let toolchain = LinuxToolchain::new(&env_vars)?;
        let output_path = cwd.join(if appimage {
            "target/release-appimage/linux"
        } else if release {
            "target/release/linux"
        } else {
            "target/debug/linux"
        });

        Ok(LinuxBuilder {
            release,
            appimage,
            target,
            cwd,
            output_path,
            icon_path,
            assets,
            toolchain,
            gpg_path,
            app_name,
            key_id,
            key_pass,
            runtime_path,
        })
    }

    fn pre_build(&self) -> Result<(), PistonError> {
        let path = self.output_path.as_path();
        Project::empty_directory(path, &["assets"])?;
        create_dir_all(path).map_err(|e| PistonError::CreateDirAllError {
            path: self.output_path.clone(),
            source: e,
        })?;
        Ok(())
    }

    fn build(&self) -> Result<(), PistonError> {
        self.toolchain.build(&self.target, self.release, &self.cwd)
    }
}

#[derive(Debug)]
struct AppImage;
impl AppImage {
    /// Builds a complete, standalone AppImage and returns the path to the final file.
    fn build(
        app_name: String, runtime_path: Option<String>, target: String, binary_path: PathBuf,
        output_dir: PathBuf, icon_path: PathBuf, description: Option<String>,
        assets: Option<PathBuf>,
    ) -> Result<PathBuf, PistonError> {
        let appimage_name = format!("{}.AppImage", app_name);
        let final_appimage = output_dir.join(&appimage_name);

        let mut fs = FilesystemWriter::default();

        let appdir_prefix = format!("{}.AppDir", app_name);

        fs.push_dir_all(
            &format!("{appdir_prefix}/usr/bin"),
            NodeHeader { permissions: 0o755, ..NodeHeader::default() },
        )
        .map_err(|e| PistonError::Generic(format!("Failed to create dir usr/bin: {}", e)))?;

        fs.push_dir_all(
            &format!("{appdir_prefix}/usr/share/applications"),
            NodeHeader { permissions: 0o755, ..NodeHeader::default() },
        )
        .map_err(|e| {
            PistonError::Generic(format!("Failed to create dir usr/share/applications: {}", e))
        })?;

        fs.push_dir_all(
            &format!("{appdir_prefix}/usr/share/icons/hicolor/256x256/apps"),
            NodeHeader { permissions: 0o755, ..NodeHeader::default() },
        )
        .map_err(|e| {
            PistonError::Generic(format!("Failed to create dir usr/share/icons/...: {}", e))
        })?;

        {
            let binary_data = fs::read(&binary_path).map_err(|e| {
                PistonError::Generic(format!(
                    "Failed to read binary {}: {}",
                    &binary_path.display(),
                    e
                ))
            })?;

            fs.push_file(
                Cursor::new(binary_data),
                &format!("{appdir_prefix}/usr/bin/{}", &app_name),
                NodeHeader { permissions: 0o755, ..NodeHeader::default() },
            )
            .map_err(|e| {
                PistonError::Generic(format!("Failed to push binary into AppDir: {}", e))
            })?;
        }

        let apprun_content = format!("#!/bin/sh\nexec \"$APPDIR/usr/bin/{}\" \"$@\"\n", app_name);
        fs.push_file(
            Cursor::new(apprun_content.into_bytes()),
            &format!("{appdir_prefix}/AppRun"),
            NodeHeader { permissions: 0o755, ..NodeHeader::default() },
        )
        .map_err(|e| PistonError::Generic(format!("Failed to push apprun content: {}", e)))?;

        let desktop_content = format!(
            r#"[Desktop Entry]
Name={}
Exec=AppRun
Icon={}
Type=Application
Categories=Utility;
Comment={}
"#,
            app_name,
            app_name,
            description.as_deref().unwrap_or("Rust application")
        );
        fs.push_file(
            Cursor::new(desktop_content.into_bytes()),
            &format!("{appdir_prefix}/{}.desktop", app_name),
            NodeHeader { permissions: 0o644, ..NodeHeader::default() },
        )
        .map_err(|e| PistonError::Generic(format!("Failed to push {}.desktop: {}", app_name, e)))?;

        let icon_dest =
            format!("{appdir_prefix}/usr/share/icons/hicolor/256x256/apps/{}.png", app_name);

        {
            let file = fs::read(&icon_path).map_err(|e| {
                PistonError::Generic(format!("Failed to read icon {}: {}", icon_path.display(), e))
            })?;

            let mut header = NodeHeader::default();
            header.permissions = 0o644;

            fs.push_file(Cursor::new(file), &icon_dest, header).map_err(|e| {
                PistonError::Generic(format!("Failed to push icon into AppDir: {}", e))
            })?;
        }

        let dir_icon = format!("{appdir_prefix}/.DirIcon");
        let root_icon = format!("{appdir_prefix}/{}.png", app_name);

        fs.push_symlink(
            icon_dest.clone(),
            &dir_icon,
            NodeHeader { permissions: 0o755, ..NodeHeader::default() },
        )
        .map_err(|e| PistonError::Generic(format!("Failed to create .DirIcon symlink: {}", e)))?;
        fs.push_symlink(
            icon_dest,
            &root_icon,
            NodeHeader { permissions: 0o755, ..NodeHeader::default() },
        )
        .map_err(|e| PistonError::Generic(format!("Failed to create root icon symlink: {}", e)))?;

        if let Some(assets) = &assets {
            let base_dest = format!("{appdir_prefix}/assets");

            fs.push_dir_all(&base_dest, NodeHeader { permissions: 0o755, ..NodeHeader::default() })
                .map_err(|e| {
                    PistonError::Generic(format!("Failed to create assets directory: {}", e))
                })?;

            Self::add_assets(&mut fs, assets, &base_dest)
                .map_err(|e| PistonError::Generic(format!("Failed to bundle assets: {}", e)))?;
        }

        let squash_path = output_dir.join(format!("{}.squashfs", app_name));
        let mut squash_file = File::create(&squash_path)
            .map_err(|e| PistonError::Generic(format!("Failed to create squashfs file: {}", e)))?;
        fs.write(&mut squash_file)
            .map_err(|e| PistonError::Generic(format!("Failed to write squashfs: {}", e)))?;

        let binding = runtime_path.unwrap();
        let runtime_dir = Path::new(&binding);
        let runtime_filename = match target.as_str() {
            t if t.starts_with("x86_64") => "runtime-x86_64",
            t if t.starts_with("aarch64") => "runtime-aarch64",
            t if t.starts_with("armv7") || t.starts_with("arm-") || t.starts_with("armv6") => {
                "runtime-armhf"
            }
            t if t.starts_with("i686") || t.starts_with("i386") => "runtime-i686",
            t if t.starts_with("ppc64le") => "runtime-ppc64le",
            t if t.starts_with("riscv64") => "runtime-riscv64",
            _ => {
                return Err(PistonError::Generic(format!(
                    "Unsupported target architecture for AppImage: {}\n\
                    Supported: x86_64, aarch64, arm*, i686, ppc64le, riscv64",
                    target
                )));
            }
        };
        let runtime_path = runtime_dir.join(runtime_filename);
        if !runtime_path.exists() {
            return Err(PistonError::Generic(format!(
                "{} not found in {}",
                runtime_filename,
                runtime_dir.display()
            )));
        }

        let mut runtime = File::open(runtime_path)
            .map_err(|e| PistonError::Generic(format!("Failed to open runtime: {}", e)))?;

        let mut appimage_file = File::create(&final_appimage)
            .map_err(|e| PistonError::Generic(format!("Failed to create final AppImage: {}", e)))?;

        io::copy(&mut runtime, &mut appimage_file)
            .map_err(|e| PistonError::Generic(format!("Failed to copy runtime: {}", e)))?;

        let mut squash = File::open(&squash_path)
            .map_err(|e| PistonError::Generic(format!("Failed to open squashfs: {}", e)))?;

        io::copy(&mut squash, &mut appimage_file)
            .map_err(|e| PistonError::Generic(format!("Failed to append squashfs: {}", e)))?;

        fs::set_permissions(&final_appimage, fs::Permissions::from_mode(0o755))
            .map_err(|e| PistonError::Generic(format!("Failed to set permissions: {}", e)))?;

        let _ = fs::remove_file(squash_path);

        Ok(final_appimage)
    }

    fn add_assets(fs: &mut FilesystemWriter, source_dir: &Path, dest_base: &str) -> Result<(), String> {
        for entry in fs::read_dir(source_dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();

            let relative =
                path.strip_prefix(source_dir).map_err(|e| format!("Strip prefix failed: {}", e))?;
            let dest_path = PathBuf::from(dest_base).join(relative).to_string_lossy().into_owned();

            if path.is_dir() {
                fs.push_dir_all(
                    &dest_path,
                    NodeHeader { permissions: 0o755, ..NodeHeader::default() },
                )
                .map_err(|e| format!("push_dir_all failed for {}: {}", dest_path, e))?;
                Self::add_assets(fs, &path, &dest_path)?;
            } else {
                let data = fs::read(&path)
                    .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

                fs.push_file(
                    Cursor::new(data),
                    &dest_path,
                    NodeHeader { permissions: 0o644, ..NodeHeader::default() },
                )
                .map_err(|e| format!("Failed to push file {}: {}", dest_path, e))?;
            }
        }
        Ok(())
    }
}

struct GPGSigner;

impl GPGSigner {
    fn gpg_valid(key_id: Option<String>, gpg_bin: Option<String>) -> bool {
        if key_id.is_none() || gpg_bin.is_none() { return false; }
        let output =
            Command::new(gpg_bin.unwrap()).arg("--list-keys").arg(key_id.unwrap()).output();

        output.is_ok_and(|output| output.status.success())
    }

    fn gpg_sign(key_id: Option<String>, key_pass: Option<String>, gpg_path: Option<String>, bundle_path: &PathBuf) -> String {
        let mut sig_path = bundle_path.clone();
        sig_path.set_extension("asc");

        let mut cmd = Command::new(gpg_path.unwrap());

        cmd.arg("--batch");
        cmd.arg("--no-tty");
        cmd.arg("--yes");
        cmd.arg("--pinentry-mode").arg("loopback");
        cmd.arg("--passphrase-fd").arg("0");

        cmd.arg("--armor")
            .arg("--output")
            .arg(&sig_path)
            .arg("-u")
            .arg(&key_id.unwrap())
            .arg("--detach-sig")
            .arg("--verbose")
            .arg(&bundle_path.display().to_string())
            .stdin(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(..) => return "Error spawning child process, GPG signing failed".to_string(),
        };

        if let Some(pass) = key_pass {
            if let Some(mut stdin) = child.stdin.take() {
                match stdin.write(pass.as_bytes()) {
                    Ok(res) => res,
                    Err(..) => {
                        return "Error writing passphrase to bytes, GPG signing failed".to_string();
                    }
                };
            }
        }

        let output = match child.wait_with_output() {
            Ok(c) => c,
            Err(..) => return "Error with gpg child output spawn, GPG signing failed".to_string(),
        };
        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            return format!("GPG signing failed: {}", err_msg);
        }

        format!(
            "successfully signed {} with signature at {:?}",
            bundle_path.display(),
            sig_path.display()
        )
    }
}

impl LinuxBuilder {
    fn post_build(&self) -> Result<Vec<String>, PistonError> {
        let mut messages = Vec::new();
        messages.push(format!("post build for linux"));
        let binary_path = self
            .cwd
            .join("target")
            .join(&self.target)
            .join(if self.release { "release" } else { "debug" })
            .join(&self.app_name);
        let bundle_path = &self.output_path;
        messages.push(format!("binary path is: {}", &binary_path.display()));
        messages.push(format!("bundle path is: {}", &bundle_path.display()));
        let icon_path: PathBuf = PathBuf::from(&self.icon_path);
        let assets_str: &str = self.assets.as_ref();
        let assets_path: PathBuf = PathBuf::from(assets_str);
        let assets = assets_path.exists().then_some(assets_path);
        if self.appimage {
            if self.runtime_path.is_some() {
                let image = AppImage::build(
                    self.app_name.clone(),
                    self.runtime_path.clone(),
                    self.target.clone(),
                    binary_path.clone(),
                    bundle_path.clone(),
                    icon_path,
                    None,
                    assets,
                )?;
                messages.push(format!("appimage was created: {:?}", image));
                if GPGSigner::gpg_valid(self.key_id.clone(), self.gpg_path.clone()) {
                    messages.push(format!("key is valid"));
                    let sign = GPGSigner::gpg_sign(
                        self.key_id.clone(),
                        self.key_pass.clone(),
                        self.gpg_path.clone(),
                        &image,
                    );
                    messages.push(format!("{}", sign));
                }
                messages.push(format!("app image available at: {}", &image.display()));
            } else {
                return Err(PistonError::Generic("Missing path to runtimes in .env".to_string()));
            }
        } else {
            messages.push(format!("Building static binary"));
            let target_path = bundle_path.join(&self.app_name);
            messages.push(format!("copying binary to app bundle"));
            copy(&binary_path, &target_path).map_err(|e| PistonError::CopyFileError {
                input_path: target_path.to_path_buf(),
                output_path: bundle_path.to_path_buf(),
                source: e,
            })?;
            //TODO tarball the static binary and output it at the output path
            if GPGSigner::gpg_valid(self.key_id.clone(), self.gpg_path.clone()) {
                messages.push(format!("key is valid"));
                let sign = GPGSigner::gpg_sign(
                    self.key_id.clone(),
                    self.key_pass.clone(),
                    self.gpg_path.clone(),
                    &target_path,
                );
                messages.push(format!("{}", sign));
            }
            messages.push(format!("app bundle available at: {}", &bundle_path.display()));
        }
        Ok(messages)
    }
}
