use image::imageops;
use serde_json::Value;
use cargo_metadata::{Message, Metadata, MetadataCommand, Package, TargetKind};
use std::collections::HashMap;
use std::env;
use std::fs::{self, File, copy, create_dir_all, read_dir, remove_dir_all, remove_file};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::Result;
use crate::cli::{ApplicationArgs, Error};
use crate::host;
use crate::manifest::Manifest;

pub struct Project {
    pub args: ApplicationArgs,
    pub package: Package,
    pub workspace: PathBuf,
    pub target_dir: PathBuf,
    pub work: PathBuf,
}

impl Project {
    pub fn manifest(args: &cargo_subcommand::Args) -> Result<Manifest> {
        if args.workspace || args.package.len() > 1 || !args.exclude.is_empty() {
            return Err(Error::AppSelectionInvalid.into());
        }
        if let Some(name) = args.package.first() {
            let mut metadata = MetadataCommand::new();
            metadata.no_deps().other_options(vec!["--offline".into()]);
            if let Some(path) = &args.manifest_path {
                let path = fs::canonicalize(path).map_err(|source| Error::ReadFile {
                    path: path.clone(), source,
                })?;
                metadata.manifest_path(&path).current_dir(path.parent().unwrap());
            }
            let metadata = metadata.exec()
                .map_err(|error| Error::CargoParseError(error.to_string()))?;
            let package = metadata.packages.iter()
                .find(|package| package.name.as_str() == name)
                .ok_or(Error::AppPackageMissing)?;
            return Manifest::new(package.manifest_path.as_std_path());
        }
        let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        command.args(["locate-project", "--message-format", "plain"]);
        if let Some(path) = &args.manifest_path { command.arg("--manifest-path").arg(path); }
        let path = Self::output(&mut command)?;
        Manifest::new(Path::new(String::from_utf8_lossy(&path).trim()))
    }

    pub fn new(mut args: ApplicationArgs, platform: &str, library: bool) -> Result<Self> {
        if args.cargo.workspace
            || args.cargo.package.len() > 1
            || args.cargo.bin.len() > 1
            || args.cargo.bins
            || args.cargo.examples
            || !args.cargo.example.is_empty()
            || !args.cargo.exclude.is_empty()
            || (library && !args.cargo.bin.is_empty())
            || (!library && args.cargo.lib)
        {
            return Err(Error::AppSelectionInvalid.into());
        }
        let mut metadata = MetadataCommand::new();
        metadata.no_deps();
        if let Some(path) = &args.cargo.manifest_path {
            let path = fs::canonicalize(path)
                .map_err(|error| Error::CargoParseError(error.to_string()))?;
            metadata.manifest_path(&path).current_dir(path.parent().unwrap());
            args.cargo.manifest_path = Some(path);
        }
        let mut options = Vec::new();
        if args.locked { options.push("--locked".into()); }
        if args.offline { options.push("--offline".into()); }
        let metadata = metadata
            .other_options(options)
            .exec()
            .map_err(|error| Error::CargoParseError(error.to_string()))?;
        let mut locate = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        locate.args(["locate-project", "--message-format", "plain"]);
        if let Some(path) = &args.cargo.manifest_path { locate.arg("--manifest-path").arg(path); }
        let manifest = Self::output(&mut locate)?;
        let manifest = fs::canonicalize(String::from_utf8_lossy(&manifest).trim())
            .map_err(|error| Error::CargoParseError(error.to_string()))?;
        let package = metadata
            .packages
            .iter()
            .find(|package| {
                if let Some(name) = args.cargo.package.first() {
                    package.name.as_str() == name
                } else {
                    package.manifest_path.as_std_path() == manifest
                }
            })
            .cloned()
            .ok_or(Error::AppPackageMissing)?;
        args.cargo.package = vec![package.name.to_string()];
        args.cargo.manifest_path = Some(package.manifest_path.clone().into_std_path_buf());
        if library {
            if !package.targets.iter().any(|target| target.kind.contains(&TargetKind::CDyLib)) {
                return Err(Error::AppLibraryMissing.into());
            }
            args.cargo.lib = true;
        } else {
            let binaries: Vec<_> = package
                .targets
                .iter()
                .filter(|target| target.kind.contains(&TargetKind::Bin))
                .collect();
            let selected = args.cargo.bin.first().or(package.default_run.as_ref());
            let binary = if let Some(name) = selected {
                binaries.iter().find(|target| &target.name == name).copied()
            } else if binaries.len() == 1 {
                Some(binaries[0])
            } else {
                None
            };
            let binary = binary.ok_or(Error::AppBinaryMissing)?;
            args.cargo.bin = vec![binary.name.clone()];
        }
        let target_dir =
            args.cargo.target_dir.clone().unwrap_or(metadata.target_directory.into_std_path_buf());
        let target_dir = env::current_dir()?.join(target_dir);
        args.cargo.target_dir = Some(target_dir.clone());
        let work = target_dir.join("piston").join(platform).join(package.name.as_str());
        fs::create_dir_all(&work)
            .map_err(|error| Error::WriteFileError(error.to_string()))?;
        Ok(Self {
            args,
            package,
            workspace: metadata.workspace_root.into_std_path_buf(),
            target_dir,
            work,
        })
    }

    pub fn root(&self) -> &Path { self.package.manifest_path.parent().unwrap().as_std_path() }

    pub fn dependencies(&self) -> Result<Metadata> {
        let mut command = MetadataCommand::new();
        command.manifest_path(&self.package.manifest_path).current_dir(self.root());
        if self.args.cargo.all_features { command.features(cargo_metadata::CargoOpt::AllFeatures); }
        if self.args.cargo.no_default_features {
            command.features(cargo_metadata::CargoOpt::NoDefaultFeatures);
        }
        if !self.args.cargo.features.is_empty() {
            command
                .features(cargo_metadata::CargoOpt::SomeFeatures(self.args.cargo.features.clone()));
        }
        let mut options = Vec::new();
        if self.args.locked { options.push("--locked".into()); }
        if self.args.offline { options.push("--offline".into()); }
        command
            .other_options(options)
            .exec()
            .map_err(|error| Error::CargoParseError(error.to_string()).into())
    }

    pub fn build_command(&self) -> Command {
        let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        command.arg("build").arg("--message-format=json-render-diagnostics");
        self.build_options(&mut command);
        command
    }

    pub fn build_options(&self, command: &mut Command) {
        self.args.cargo.apply(command);
        if self.args.locked { command.arg("--locked"); }
        if self.args.offline { command.arg("--offline"); }
        command.current_dir(self.root()).env_remove("RAMP_NOTIFICATION_APPLICATION");
    }

    pub fn compile<F: FnMut(&str)>(&self, mut command: Command, mut report: F) -> Result<PathBuf> {
        let program = command.get_program().to_os_string();
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|source| Error::CommandSpawn { program: program.clone(), source })?;
        let mut artifact = None;
        for message in Message::parse_stream(BufReader::new(child.stdout.take().unwrap())) {
            match message {
                Ok(Message::CompilerArtifact(value)) if value.package_id == self.package.id => {
                    if self.args.cargo.lib {
                        artifact = value
                            .filenames
                            .iter()
                            .find(|path| path.extension() == Some("so"))
                            .map(|path| path.clone().into_std_path_buf())
                            .or(artifact);
                    } else if value.target.name == self.args.cargo.bin[0] {
                        artifact =
                            value.executable.map(|path| path.into_std_path_buf()).or(artifact);
                    }
                }
                Ok(Message::CompilerMessage(value)) => {
                    if let Some(rendered) = value.message.rendered { report(&rendered); }
                }
                Ok(Message::TextLine(line)) => report(&format!("{line}\n")),
                _ => {}
            }
        }
        let status = child.wait().map_err(|source| Error::CommandWait {
            program: program.clone(), source,
        })?;
        if !status.success() {
            return Err(Error::CommandStatus { program, status }.into());
        }
        artifact.ok_or_else(|| Error::ArtifactNotProduced {
            package: self.package.name.to_string(), target: self.args.cargo.target.clone(),
        }.into())
    }

    pub fn launch(&self, executable: &Path, arguments: &[String]) -> Result<()> {
        Self::run(Command::new(executable).args(arguments).current_dir(self.root()))
    }

    pub fn rust_target(target: &str) -> Result<()> {
        let output = Self::output(Command::new("rustup").args(["target", "list", "--installed"]))?;
        if !String::from_utf8_lossy(&output).lines().any(|line| line == target) {
            Self::run(Command::new("rustup").args(["target", "add", target]))?;
        }
        Ok(())
    }

    pub fn build_target(target: Option<&str>) -> Result<String> {
        let host = Self::host_target()?;
        Ok(match target {
            None => host,
            Some("windows") => "x86_64-pc-windows-gnu".into(),
            Some(target @ ("ios" | "android" | "linux" | "macos")) => {
                let architecture = if host.contains("aarch64") {
                    "aarch64"
                } else if host.contains("x86_64") {
                    "x86_64"
                } else {
                    return Err(host::cli::Error::HostArchitectureUnsupported { target: host }.into());
                };
                let suffix = match target {
                    "ios" => "apple-ios",
                    "android" => "linux-android",
                    "linux" => "unknown-linux-gnu",
                    "macos" => "apple-darwin",
                    _ => unreachable!(),
                };
                format!("{architecture}-{suffix}")
            }
            Some(target) => target.to_owned(),
        })
    }

    pub fn host_target() -> Result<String> {
        let version = Self::output(Command::new("rustc").arg("-vV"))?;
        String::from_utf8_lossy(&version)
            .lines()
            .find_map(|line| line.strip_prefix("host: "))
            .map(str::to_owned)
            .ok_or_else(|| host::cli::Error::RustHostMissing.into())
    }

    pub fn run(command: &mut Command) -> Result<()> {
        let program = command.get_program().to_os_string();
        let status = command.status().map_err(|source| Error::CommandSpawn {
            program: program.clone(), source,
        })?;
        if status.success() { Ok(()) } else { Err(Error::CommandStatus { program, status }.into()) }
    }

    pub fn output(command: &mut Command) -> Result<Vec<u8>> {
        let program = command.get_program().to_os_string();
        let output = command.output().map_err(|source| Error::CommandSpawn {
            program: program.clone(), source,
        })?;
        if output.status.success() { return Ok(output.stdout); }
        Err(Error::CommandFailed { program, output }.into())
    }

    pub fn write(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| Error::WriteFileError(error.to_string()))?;
        }
        fs::write(path, contents).map_err(|error| Error::WriteFileError(error.to_string()).into())
    }
}

impl Project {
    pub fn empty_directory(tgt_path: &Path, preserve: &[&str]) -> Result<()> {
        if !tgt_path.exists() || !tgt_path.is_dir() { return Ok(()); }
        for entry in read_dir(tgt_path)
            .map_err(|e| Error::ReadDirError { path: tgt_path.to_path_buf(), source: e })?
        {
            let entry = entry.map_err(|e| Error::ReadDirError {
                path: tgt_path.to_path_buf(),
                source: e,
            })?;
            let entry_path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry_path.is_dir() && preserve.contains(&name.as_str()) { continue; }
            if entry_path.is_dir() {
                remove_dir_all(&entry_path)
                    .map_err(|e| Error::RemoveSubdirError { path: entry_path, source: e })?;
            } else {
                remove_file(&entry_path)
                    .map_err(|e| Error::RemoveFileError { path: entry_path, source: e })?;
            }
        }
        Ok(())
    }

    pub fn sync_assets(src: &Path, tgt: &Path) -> Result<()> {
        if !src.exists() {
            if tgt.exists() {
                remove_dir_all(tgt).map_err(|e| Error::RemoveSubdirError {
                    path: tgt.to_path_buf(),
                    source: e,
                })?;
            }
            return Ok(());
        }
        create_dir_all(tgt)
            .map_err(|e| Error::CreateDirAllError { path: tgt.to_path_buf(), source: e })?;
        Self::copy_updated_files(src, tgt)?;
        Self::remove_stale_files(src, tgt)?;
        Ok(())
    }

    pub fn load_env_file() -> io::Result<HashMap<String, String>> {
        let path = std::env::current_dir()?.join(".env");
        let reader = BufReader::new(File::open(path)?);
        let mut env_map = HashMap::new();
        for line in reader.lines() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            if let Some(eq_index) = line.find('=') {
                let key = line[..eq_index].trim().to_string();
                let value = line[eq_index + 1..].trim().to_string();
                if !key.is_empty() { env_map.insert(key, value); }
            }
        }
        Ok(env_map)
    }

    pub fn capitalize_first(s: &str) -> String {
        match s.get(0..1) { None => String::new(), Some(first) => first.to_uppercase() + &s[1..], }
    }

    pub fn resize_png(input_name: &str, target_name: &str, width: u32, height: u32) -> Result<()> {
        let img = image::open(input_name).map_err(|e| Error::OpenImageError {
            path: PathBuf::from(input_name),
            source: e,
        })?;
        if Path::new(target_name).exists() {
            fs::remove_file(target_name).map_err(|source| Error::RemoveFileError {
                path: PathBuf::from(target_name),
                source,
            })?;
        }
        let resized_img = imageops::resize(&img, width, height, imageops::FilterType::Lanczos3);
        resized_img.save(target_name).map_err(|source| Error::SaveImage {
            path: PathBuf::from(target_name), source,
        })?;
        Ok(())
    }

    pub fn get_lib_name(metadata: &Metadata) -> Result<String> {
        let root_package = metadata.root_package().ok_or(Error::AppPackageMissing)?;
        let target_type: TargetKind = "cdylib".into();
        Ok(root_package
            .targets
            .iter()
            .find(|target| target.kind.contains(&target_type))
            .map(|target| target.name.clone())
            .unwrap_or_else(|| root_package.name.replace('-', "_")))
    }

    pub fn find_icon(root: &Path) -> Option<PathBuf> {
        ["icon.png", "icon.jpg", "icon.jpeg"]
            .into_iter()
            .map(|name| root.join(name))
            .find(|path| path.is_file())
    }

    pub fn get_icon_path(metadata: &Metadata, cwd: &Path) -> String {
        let root = metadata
            .root_package()
            .map(|package| package.manifest_path.parent().unwrap().as_std_path())
            .unwrap_or(cwd);
        if let Some(path) = Self::find_icon(root) { return path.to_string_lossy().into_owned(); }
        let default = root.join("icon.png");
        if !default.exists() {
            let bundled_icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("icon.png");
            if bundled_icon.exists() { let _ = fs::copy(&bundled_icon, &default); }
        }
        default.to_string_lossy().into_owned()
    }

    pub fn get_assets_path(metadata: &Metadata) -> String {
        metadata
            .root_package()
            .and_then(|pkg| pkg.metadata.get("assets_path"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    pub fn get_app_name(metadata: &Metadata) -> Result<String> {
        metadata.root_package().map(|pkg| pkg.name.to_string())
            .ok_or_else(|| Error::AppPackageMissing.into())
    }

    pub fn get_app_version(metadata: &Metadata) -> Result<String> {
        metadata.root_package().map(|pkg| pkg.version.to_string())
            .ok_or_else(|| Error::AppPackageMissing.into())
    }

    pub fn get_bundle_id(metadata: &Metadata, app_name: &str) -> String {
        metadata
            .root_package()
            .and_then(|pkg| pkg.metadata.get("ios"))
            .and_then(|ios| ios.get("bundle_id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("com.piston.{}", app_name))
    }

    pub fn get_min_os(metadata: &Metadata) -> f32 {
        metadata
            .root_package()
            .and_then(|pkg| pkg.metadata.get("ios"))
            .and_then(|ios| ios.get("min_os_version"))
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(17.5)
    }

    fn copy_updated_files(src: &Path, dst: &Path) -> Result<()> {
        for entry in read_dir(src)
            .map_err(|e| Error::ReadDirError { path: src.to_path_buf(), source: e })?
        {
            let entry = entry
                .map_err(|e| Error::ReadDirError { path: src.to_path_buf(), source: e })?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if src_path.is_dir() {
                create_dir_all(&dst_path).map_err(|source| Error::CreateDirAllError {
                    path: dst_path.clone(), source,
                })?;
                Self::copy_updated_files(&src_path, &dst_path)?;
            } else {
                let needs_copy = match (src_path.metadata(), dst_path.metadata()) {
                    (Ok(src_meta), Ok(dst_meta)) => {
                        src_meta.modified().map_err(|source| Error::FileMetadata {
                            path: src_path.clone(), source,
                        })? > dst_meta.modified().map_err(|source| Error::FileMetadata {
                            path: dst_path.clone(), source,
                        })?
                    }
                    (Ok(_), Err(_)) => true,
                    (Err(e), _) => {
                        return Err(Error::FileMetadata { path: src_path, source: e }.into());
                    }
                };
                if needs_copy {
                    copy(&src_path, &dst_path).map_err(|source| Error::CopyFileError {
                        input_path: src_path.clone(), output_path: dst_path.clone(), source,
                    })?;
                }
            }
        }
        Ok(())
    }

    fn remove_stale_files(src: &Path, dst: &Path) -> Result<()> {
        for entry in read_dir(dst)
            .map_err(|e| Error::ReadDirError { path: dst.to_path_buf(), source: e })?
        {
            let entry = entry
                .map_err(|e| Error::ReadDirError { path: dst.to_path_buf(), source: e })?;
            let dst_path = entry.path();
            let corresponding_src = src.join(entry.file_name());
            if dst_path.is_dir() {
                if !corresponding_src.exists() {
                    remove_dir_all(&dst_path).map_err(|e| Error::RemoveSubdirError {
                        path: dst_path,
                        source: e,
                    })?;
                } else {
                    Self::remove_stale_files(&corresponding_src, &dst_path)?;
                }
            } else if !corresponding_src.exists() {
                remove_file(&dst_path)
                    .map_err(|e| Error::RemoveFileError { path: dst_path, source: e })?;
            }
        }
        Ok(())
    }
}
