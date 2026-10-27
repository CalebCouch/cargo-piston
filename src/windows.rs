use cargo_metadata::{DependencyKind, Metadata, MetadataCommand};
use image::{self, DynamicImage, ImageEncoder, imageops};
use std::collections::HashMap;
use std::fs::{File, copy, create_dir_all, remove_file, write};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::Result;
use crate::cli::Error;
use crate::project::Project;

pub mod cli;
pub use cli::Cli;

pub struct WindowsBuilder {
    release: bool,
    target: String,
    cwd: PathBuf,
    output_path: PathBuf,
    icon_path: String,
    assets: String,
    embed_resources_ok: bool,
    cargo_path: String,
    app_name: String,
}

impl WindowsBuilder {
    fn new(release: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<Self> {
        let cargo_path: String = env_vars.get("cargo_path").cloned().unwrap_or("cargo".to_string());
        let metadata: Metadata = MetadataCommand::new()
            .current_dir(&cwd)
            .exec()
            .map_err(|e| Error::CargoParseError(e.to_string()))?;

        let embed_resources_ok: bool = if let Some(root_pkg) = metadata.root_package() {
            root_pkg
                .dependencies
                .iter()
                .any(|dep| dep.name == "embed-resource" && dep.kind == DependencyKind::Build)
        } else {
            false
        };
        let icon_path = Project::get_icon_path(&metadata, &cwd);
        let assets = Project::get_assets_path(&metadata);
        let app_name = Project::get_app_name(&metadata)?;
        let output_path =
            cwd.join(if release { "target/release/windows" } else { "target/debug/windows" });
        Ok(WindowsBuilder {
            release,
            target,
            cwd,
            output_path,
            icon_path,
            assets,
            embed_resources_ok,
            cargo_path,
            app_name,
        })
    }

    fn pre_build(&self) -> Result<()> {
        let cwd = &self.cwd;
        let rel_output = self.output_path.strip_prefix(&self.cwd).unwrap();
        let path = self.output_path.as_path();
        Project::empty_directory(path, &["assets"])?;
        create_dir_all(path).map_err(|e| Error::CreateDirAllError {
            path: self.output_path.clone(),
            source: e,
        })?;
        let rc_path: PathBuf = self.output_path.join("app.rc");
        let rc_icon: &PathBuf = &rel_output.join("windows_icon.ico");
        let content = format!("IDI_ICON1 ICON \"{}\"", rc_icon.display());
        write(&rc_path, content.as_bytes())
            .map_err(|e| Error::WriteFileError(e.to_string()))?;

        let assets_src = Path::new(&self.assets);
        let assets_tgt = path.join("assets");
        Project::sync_assets(assets_src, &assets_tgt)?;
        if self.embed_resources_ok {
            let icon_output: PathBuf = cwd.join(rc_icon);
            let img_path = Path::new(&self.icon_path);
            let img = image::open(img_path).map_err(|e| Error::OpenImageError {
                path: img_path.to_path_buf(),
                source: e,
            })?;
            let resized = imageops::resize(&img, 64, 64, imageops::FilterType::Lanczos3);
            let resized_img = DynamicImage::ImageRgba8(resized);
            let file = File::create(&icon_output).map_err(|e| Error::CreateFileError {
                path: icon_output.to_path_buf(),
                source: e,
            })?;
            let mut writer = std::io::BufWriter::new(file);
            let encoder = image::codecs::ico::IcoEncoder::new(&mut writer);
            encoder
                .write_image(resized_img.as_bytes(), 64, 64, image::ExtendedColorType::Rgba8)
                .map_err(Error::WriteImageError)?;
            let build_path: PathBuf = cwd.join("build.rs");
            if build_path.exists() {
                remove_file(&build_path).map_err(|e| Error::RemoveFileError {
                    path: build_path.to_path_buf(),
                    source: e,
                })?;
            }
            let build_content = format!(
                r#"
                use std::io;

                fn main() {{
                    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" && std::path::Path::new("{}").exists() {{
                        embed_resource::compile("app.rc", embed_resource::NONE)
                        .manifest_optional();
                    }}
            }}
            
                "#,
                &icon_output.display()
            );
            let mut build_file = File::create(&build_path).map_err(|e| {
                Error::CreateFileError { path: build_path.to_path_buf(), source: e }
            })?;
            build_file
                .write_all(build_content.as_bytes())
                .map_err(|e| Error::WriteFileError(e.to_string()))?;
            build_file.flush().map_err(cli::Error::FileFlushError)?;
        }
        Ok(())
    }

    fn build(&self) -> Result<()> {
        let cargo_args = format!(
            "build --target {} {}",
            self.target,
            if self.release { "--release" } else { "" }
        );
        let cargo_cmd = format!("{} {}", self.cargo_path, cargo_args);
        let builder = Command::new("bash")
            .arg("-c")
            .arg(&cargo_cmd)
            .current_dir(&self.cwd)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| Error::CommandSpawn { program: "bash".into(), source: e })?;
        if !builder.status.success() {
            return Err(Error::CommandFailed { program: "bash".into(), output: builder }.into());
        }

        Ok(())
    }

    fn post_build(&self) -> Result<()> {
        let binary_path = self
            .cwd
            .join("target")
            .join(&self.target)
            .join(if self.release { "release" } else { "debug" })
            .join(format!("{}.exe", self.app_name));
        let bundle_path = self.output_path.join(format!("{}.exe", self.app_name));
        copy(&binary_path, &bundle_path).map_err(|e| Error::CopyFileError {
            input_path: binary_path.to_path_buf(),
            output_path: bundle_path.to_path_buf(),
            source: e,
        })?;
        Ok(())
    }
}
