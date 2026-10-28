use backhand::{FilesystemWriter, NodeHeader};
use cargo_metadata::{Metadata, MetadataCommand};
use std::collections::HashMap;
use std::fs::{self, File, copy, create_dir_all};
use std::io::{self, Cursor, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::Result;
use crate::cli::Error;
use crate::project::Project;
use crate::host::LinuxToolchain;

pub mod cli;
pub use cli::Cli;
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
    fn new(release: bool, appimage: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<Self> {
        let gpg_path: Option<String> = env_vars.get("gpg_path").cloned();
        let key_id: Option<String> = env_vars.get("linux_gpg_key_id").cloned();
        let key_pass: Option<String> = env_vars.get("linux_gpg_key_pass").cloned();
        let runtime_path: Option<String> = env_vars.get("linux_runtime_path").cloned();
        let metadata: Metadata = MetadataCommand::new()
            .current_dir(&cwd)
            .exec()
            .map_err(|e| Error::CargoParse(e.to_string()))?;

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

    fn pre_build(&self) -> Result<()> {
        let path = self.output_path.as_path();
        Project::empty_directory(path, &["assets"])?;
        create_dir_all(path).map_err(|e| Error::CreateDirAll {
            path: self.output_path.clone(),
            source: e,
        })?;
        Ok(())
    }

    fn build(&self) -> Result<()> {
        self.toolchain.build(&self.target, self.release, &self.cwd)
    }

    fn build_appimage(&self, binary_path: &Path) -> Result<PathBuf> {
        let appimage_name = format!("{}.AppImage", self.app_name);
        let final_appimage = self.output_path.join(&appimage_name);

        let mut fs = FilesystemWriter::default();

        let appdir_prefix = format!("{}.AppDir", self.app_name);

        fs.push_dir_all(
            format!("{appdir_prefix}/usr/bin"),
            NodeHeader { permissions: 0o755, ..NodeHeader::default() },
        )
        .map_err(|e| cli::Error::Image(e.to_string()))?;

        fs.push_dir_all(
            format!("{appdir_prefix}/usr/share/applications"),
            NodeHeader { permissions: 0o755, ..NodeHeader::default() },
        )
        .map_err(|e| cli::Error::Image(e.to_string()))?;

        fs.push_dir_all(
            format!("{appdir_prefix}/usr/share/icons/hicolor/256x256/apps"),
            NodeHeader { permissions: 0o755, ..NodeHeader::default() },
        )
        .map_err(|e| cli::Error::Image(e.to_string()))?;

        {
            let binary_data = fs::read(binary_path).map_err(|e| {
                Error::ReadFile { path: binary_path.to_path_buf(), source: e }
            })?;

            fs.push_file(
                Cursor::new(binary_data),
                format!("{appdir_prefix}/usr/bin/{}", self.app_name),
                NodeHeader { permissions: 0o755, ..NodeHeader::default() },
            )
            .map_err(|e| cli::Error::Image(e.to_string()))?;
        }

        let apprun_content = format!("#!/bin/sh\nexec \"$APPDIR/usr/bin/{}\" \"$@\"\n", self.app_name);
        fs.push_file(
            Cursor::new(apprun_content.into_bytes()),
            format!("{appdir_prefix}/AppRun"),
            NodeHeader { permissions: 0o755, ..NodeHeader::default() },
        )
        .map_err(|e| cli::Error::Image(e.to_string()))?;

        let desktop_content = format!(
            r#"[Desktop Entry]
Name={}
Exec=AppRun
Icon={}
Type=Application
Categories=Utility;
Comment=Rust application
"#,
            self.app_name, self.app_name,
        );
        fs.push_file(
            Cursor::new(desktop_content.into_bytes()),
            format!("{appdir_prefix}/{}.desktop", self.app_name),
            NodeHeader { permissions: 0o644, ..NodeHeader::default() },
        )
        .map_err(|e| cli::Error::Image(e.to_string()))?;

        let icon_dest =
            format!("{appdir_prefix}/usr/share/icons/hicolor/256x256/apps/{}.png", self.app_name);

        {
            let file = fs::read(&self.icon_path).map_err(|e| {
                Error::ReadFile { path: self.icon_path.clone().into(), source: e }
            })?;

            let header = NodeHeader {permissions: 0o644, ..NodeHeader::default()};

            fs.push_file(Cursor::new(file), &icon_dest, header).map_err(|e| {
                cli::Error::Image(e.to_string())
            })?;
        }

        let dir_icon = format!("{appdir_prefix}/.DirIcon");
        let root_icon = format!("{appdir_prefix}/{}.png", self.app_name);

        fs.push_symlink(
            icon_dest.clone(),
            &dir_icon,
            NodeHeader { permissions: 0o755, ..NodeHeader::default() },
        )
        .map_err(|e| cli::Error::Image(e.to_string()))?;
        fs.push_symlink(
            icon_dest,
            &root_icon,
            NodeHeader { permissions: 0o755, ..NodeHeader::default() },
        )
        .map_err(|e| cli::Error::Image(e.to_string()))?;

        let assets = Path::new(&self.assets);
        if assets.exists() {
            let base_dest = format!("{appdir_prefix}/assets");

            fs.push_dir_all(&base_dest, NodeHeader { permissions: 0o755, ..NodeHeader::default() })
                .map_err(|e| cli::Error::Image(e.to_string()))?;

            Self::add_assets(&mut fs, assets, &base_dest)?;
        }

        let squash_path = self.output_path.join(format!("{}.squashfs", self.app_name));
        let mut squash_file = File::create(&squash_path)
            .map_err(|source| Error::CreateFile { path: squash_path.clone(), source })?;
        fs.write(&mut squash_file)
            .map_err(|e| cli::Error::Image(e.to_string()))?;

        let runtime_dir = Path::new(self.runtime_path.as_ref().unwrap());
        let runtime_filename = match self.target.as_str() {
            t if t.starts_with("x86_64") => "runtime-x86_64",
            t if t.starts_with("aarch64") => "runtime-aarch64",
            t if t.starts_with("armv7") || t.starts_with("arm-") || t.starts_with("armv6") => {
                "runtime-armhf"
            }
            t if t.starts_with("i686") || t.starts_with("i386") => "runtime-i686",
            t if t.starts_with("ppc64le") => "runtime-ppc64le",
            t if t.starts_with("riscv64") => "runtime-riscv64",
            _ => {
                return Err(cli::Error::AppImageTargetUnsupported {
                    target: self.target.clone(),
                }.into());
            }
        };
        let runtime_path = runtime_dir.join(runtime_filename);
        if !runtime_path.exists() {
            return Err(cli::Error::RuntimeMissing { path: runtime_path }.into());
        }

        let mut runtime = File::open(&runtime_path)
            .map_err(|source| Error::ReadFile { path: runtime_path.clone(), source })?;

        let mut appimage_file = File::create(&final_appimage)
            .map_err(|source| Error::CreateFile { path: final_appimage.clone(), source })?;

        io::copy(&mut runtime, &mut appimage_file).map_err(|source| Error::CopyFile {
            input_path: runtime_path, output_path: final_appimage.clone(), source,
        })?;

        let mut squash = File::open(&squash_path)
            .map_err(|source| Error::ReadFile { path: squash_path.clone(), source })?;

        io::copy(&mut squash, &mut appimage_file).map_err(|source| Error::CopyFile {
            input_path: squash_path.clone(), output_path: final_appimage.clone(), source,
        })?;

        fs::set_permissions(&final_appimage, fs::Permissions::from_mode(0o755))
            .map_err(|source| Error::SetPermissions { path: final_appimage.clone(), source })?;

        let _ = fs::remove_file(squash_path);

        Ok(final_appimage)
    }

    fn add_assets(fs: &mut FilesystemWriter, source_dir: &Path, dest_base: &str) -> Result<()> {
        for entry in fs::read_dir(source_dir).map_err(|source| Error::ReadDir {
            path: source_dir.into(), source,
        })? {
            let entry = entry.map_err(|source| Error::MapDir {
                path: source_dir.into(), source,
            })?;
            let path = entry.path();

            let relative = path.strip_prefix(source_dir).unwrap();
            let dest_path = PathBuf::from(dest_base).join(relative).to_string_lossy().into_owned();

            if path.is_dir() {
                fs.push_dir_all(
                    &dest_path,
                    NodeHeader { permissions: 0o755, ..NodeHeader::default() },
                )
                .map_err(|e| cli::Error::Image(e.to_string()))?;
                Self::add_assets(fs, &path, &dest_path)?;
            } else {
                let data = fs::read(&path)
                    .map_err(|source| Error::ReadFile { path: path.clone(), source })?;

                fs.push_file(
                    Cursor::new(data),
                    &dest_path,
                    NodeHeader { permissions: 0o644, ..NodeHeader::default() },
                )
                .map_err(|e| cli::Error::Image(e.to_string()))?;
            }
        }
        Ok(())
    }

    fn post_build(&self) -> Result<PathBuf> {
        let binary_path = self.cwd.join("target").join(&self.target)
            .join(if self.release { "release" } else { "debug" }).join(&self.app_name);
        if self.appimage {
            if self.runtime_path.is_none() {
                return Err(Error::MissingSetting { key: "linux_runtime_path".into() }.into());
            }
            self.build_appimage(&binary_path)
        } else {
            let target_path = self.output_path.join(&self.app_name);
            copy(&binary_path, &target_path).map_err(|source| Error::CopyFile {
                input_path: binary_path, output_path: target_path.clone(), source,
            })?;
            Ok(target_path)
        }
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

    fn gpg_sign(key_id: Option<String>, key_pass: Option<String>, gpg_path: Option<String>, bundle_path: &Path) -> Result<PathBuf> {
        let sig_path = bundle_path.with_extension("asc");
        let mut command = Command::new(gpg_path.unwrap());
        command.args(["--batch", "--no-tty", "--yes", "--pinentry-mode", "loopback"])
            .args(["--passphrase-fd", "0", "--armor", "--output"]).arg(&sig_path)
            .arg("-u").arg(key_id.unwrap()).args(["--detach-sig", "--verbose"])
            .arg(bundle_path).stdin(Stdio::piped());
        let mut child = command.spawn().map_err(|source| Error::CommandSpawn {
            program: command.get_program().into(), source,
        })?;
        if let Some(pass) = key_pass {
            child.stdin.take().unwrap().write_all(pass.as_bytes())
                .map_err(cli::Error::SigningInput)?;
        }
        let output = child.wait_with_output().map_err(|source| Error::CommandWait {
            program: command.get_program().into(), source,
        })?;
        if !output.status.success() {
            return Err(Error::CommandFailed {
                program: command.get_program().into(), output,
            }.into());
        }
        Ok(sig_path)
    }
}
