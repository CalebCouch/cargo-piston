use cargo_metadata::{Message, MetadataCommand, Package, TargetKind};
use serde_json::{Value, json};
use toml_edit::Item;
use std::env;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::{cli, Result};
use crate::project::Project;
use crate::apple::{NotificationSetup, PushBuild, PushPlatform, PushSigning};
use crate::manifest::Manifest;
use crate::cli::ApplicationArgs;
use super::cli::Error;

pub struct IOSApplication {
    args: ApplicationArgs,
    package: Package,
    config: IOSConfig,
    work: PathBuf,
    notifications: Option<PushBuild>,
}

impl IOSApplication {
    pub fn new(mut args: ApplicationArgs) -> Result<Self> {
        if args.cargo.workspace
            || args.cargo.package.len() > 1
            || args.cargo.bin.len() > 1
            || args.cargo.lib
            || args.cargo.bins
            || args.cargo.examples
            || !args.cargo.example.is_empty()
            || !args.cargo.exclude.is_empty()
        {
            return Err(cli::Error::AppSelectionInvalid.into());
        }
        let mut command = MetadataCommand::new();
        command.no_deps();
        if let Some(path) = &args.cargo.manifest_path {
            let path = fs::canonicalize(path)
                .map_err(|error| cli::Error::CargoParseError(error.to_string()))?;
            command.manifest_path(&path).current_dir(path.parent().unwrap());
            args.cargo.manifest_path = Some(path);
        }
        let mut options = Vec::new();
        if args.locked { options.push("--locked".into()); }
        if args.offline { options.push("--offline".into()); }
        command.other_options(options);
        let metadata = command.exec()
            .map_err(|error| cli::Error::CargoParseError(error.to_string()))?;
        let mut locate = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        locate.args(["locate-project", "--message-format", "plain"]);
        if let Some(path) = &args.cargo.manifest_path { locate.arg("--manifest-path").arg(path); }
        let manifest =
            locate.output().map_err(|error| cli::Error::CargoParseError(error.to_string()))?;
        let manifest = Self::check(manifest, "cargo")?;
        let manifest = PathBuf::from(String::from_utf8_lossy(&manifest).trim());
        let manifest = fs::canonicalize(manifest)
            .map_err(|error| cli::Error::CargoParseError(error.to_string()))?;
        let package = if let Some(name) = args.cargo.package.first() {
            metadata.packages.iter().find(|package| package.name.as_str() == name)
        } else {
            metadata.packages.iter().find(|package| package.manifest_path.as_std_path() == manifest)
        }
        .cloned()
        .ok_or(cli::Error::AppPackageMissing)?;
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
        let binary = binary.ok_or(cli::Error::AppBinaryMissing)?;
        args.cargo.package = vec![package.name.to_string()];
        args.cargo.bin = vec![binary.name.clone()];
        let config = IOSConfig::new(package.metadata.pointer("/piston/ios"));
        let version: Vec<_> = config.minimum_os.split('.').collect();
        if version.is_empty()
            || version.len() > 3
            || version
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
            || version[0].parse::<u32>().unwrap_or(0) < 17
        {
            return Err(Error::IosMinimumVersion { value: config.minimum_os }.into());
        }
        let target_dir = args
            .cargo
            .target_dir
            .clone()
            .unwrap_or_else(|| metadata.target_directory.clone().into_std_path_buf());
        let target_dir = if target_dir.is_absolute() {
            target_dir
        } else {
            env::current_dir()?.join(target_dir)
        };
        let work = target_dir.join("piston").join("ios").join(package.name.as_str());
        fs::create_dir_all(&work)
            .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
        Ok(Self { args, package, config, work, notifications: None })
    }

    pub fn package(&self) -> &Package { &self.package }
    pub fn work(&self) -> &Path { &self.work }
    pub fn team(&self) -> Option<&str> { self.args.team.as_deref().or(self.config.team.as_deref()) }
    pub fn build_number(&self) -> Option<&str> { self.config.build_number.as_deref() }
    pub fn target(&self) -> &str {
        self.args.cargo.target.as_deref().unwrap_or("aarch64-apple-ios")
    }

    pub fn valid_setting(name: &str, value: &Item, root: &Path) -> bool {
        if name == "resources" {
            return value.as_array().is_some_and(|paths| paths.iter().all(|value| {
                value.as_str().is_some_and(|path| {
                    !path.is_empty() && root.join(path).exists()
                        && root.join(path).file_name().is_some()
                })
            }));
        }
        let Some(value) = value.as_str() else { return false; };
        match name {
            "team" => value.len() == 10 && value.bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()),
            "bundle_id" => value.contains('.') && !value.split('.').any(str::is_empty)
                && value.chars().all(|character| {
                    character.is_ascii_alphanumeric() || ".-".contains(character)
                }),
            "minimum_os" | "build_number" => {
                let parts: Vec<_> = value.split('.').collect();
                parts.len() <= 3 && parts.iter().all(|part| {
                    !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
                        && part.parse::<u64>().is_ok()
                }) && (name == "build_number" || parts[0].parse::<u32>().is_ok_and(|major| major >= 17))
            }
            "info_plist" | "entitlements" => !value.is_empty()
                && plist::Value::from_file(root.join(value))
                    .is_ok_and(|value| value.as_dictionary().is_some()),
            _ => unreachable!(),
        }
    }

    pub fn remember_team(&mut self, team: &str) -> Result<()> {
        Manifest::new(self.package.manifest_path.as_std_path())?.save(&["ios"], &[("team", team)])?;
        self.config.team = Some(team.into());
        Ok(())
    }

    pub fn notification_setup(&self, team: &str, sandbox: bool) -> Result<NotificationSetup> {
        let signing = PushSigning {
            platform: PushPlatform::Ios,
            team: team.into(),
            topic: self.bundle_id(team)?,
            sandbox,
        };
        NotificationSetup::new(self.package.manifest_path.as_std_path(), signing)
    }

    pub fn use_notifications(&mut self, setup: NotificationSetup, application: String) -> Result<()> {
        let entitlements = setup.signing.entitlements(
            self.entitlements().as_deref(),
            &self.work.join("notifications.entitlements"),
        )?;
        self.args.entitlements = Some(entitlements);
        self.notifications = Some(PushBuild { application, signing: setup.signing });
        Ok(())
    }

    pub fn verify_notifications(&self, bundle: &Path) -> Result<()> {
        if let Some(notifications) = &self.notifications { notifications.signing.verify(bundle)?; }
        Ok(())
    }

    pub fn verify_store_notifications(&self, ipa: &Path) -> Result<()> {
        if let Some(notifications) = &self.notifications {
            notifications.signing.verify_ipa(ipa, &self.work)?;
        }
        Ok(())
    }

    pub fn remember_build(&mut self, number: String) -> Result<()> {
        Manifest::new(self.package.manifest_path.as_std_path())?
            .save(&["ios"], &[("build_number", &number)])?;
        self.config.build_number = Some(number);
        Ok(())
    }

    pub fn entitlements(&self) -> Option<PathBuf> {
        self.args
            .entitlements
            .as_ref()
            .or(self.config.entitlements.as_ref())
            .map(|path| self.project_path(path))
    }

    pub fn store_bundle_id(&self) -> Result<&str> {
        let identifier = self.args.bundle_id.as_deref().or(self.config.bundle_id.as_deref())
            .ok_or(Error::IosStoreIdentifierMissing)?;
        if !identifier.contains('.')
            || identifier.split('.').any(str::is_empty)
            || !identifier.chars().all(|character| {
                character.is_ascii_alphanumeric() || character == '.' || character == '-'
            })
        {
            return Err(cli::Error::InvalidBundleId { value: identifier.into() }.into());
        }
        Ok(identifier)
    }

    pub fn store_release(&mut self) -> Result<()> {
        self.store_bundle_id()?;
        if self.target() != "aarch64-apple-ios"
            || self
                .args
                .cargo
                .profile
                .as_ref()
                .is_some_and(|profile| profile.to_string() != "release")
        {
            return Err(Error::IosStoreTargetInvalid.into());
        }
        self.args.cargo.release = true;
        self.args.cargo.profile = None;
        self.args.cargo.target = Some("aarch64-apple-ios".into());
        Ok(())
    }

    pub fn simulator_target(&mut self) -> Result<()> {
        let default = if crate::project::Project::host_target()?.starts_with("aarch64") {
            "aarch64-apple-ios-sim"
        } else {
            "x86_64-apple-ios"
        };
        let target = self.args.cargo.target.as_deref().unwrap_or(default);
        if !matches!(target, "aarch64-apple-ios-sim" | "x86_64-apple-ios") {
            return Err(Error::IosSimulatorTarget { target: target.into() }.into());
        }
        self.args.cargo.target = Some(target.into());
        Ok(())
    }

    pub fn sdk(&self) -> &str {
        if matches!(self.target(), "aarch64-apple-ios-sim" | "x86_64-apple-ios") {
            "iphonesimulator"
        } else {
            "iphoneos"
        }
    }

    pub fn build(&mut self, developer: &Path, team: &str, report: &mut dyn FnMut(&str)) -> Result<(PathBuf, String)> {
        let executable = self.compile(&self.target().to_string(), developer, report)?;
        self.bundle(&executable, team, developer, None)
    }

    pub fn build_store(&mut self, developer: &Path, team: &str, report: &mut dyn FnMut(&str)) -> Result<PathBuf> {
        let source = Project::find_icon(self.package.manifest_path.parent().unwrap().as_std_path())
            .ok_or(Error::IosStoreIconMissing)?;
        let icon = image::open(&source)
            .map_err(|error| cli::Error::OpenImageError { path: source.clone(), source: error })?;
        if icon.width() != icon.height()
            || icon.width() < 1024
            || icon.to_rgba16().pixels().any(|pixel| pixel.0[3] != u16::MAX)
        {
            return Err(Error::IosStoreIconInvalid { path: source }.into());
        }
        let normalized = self.work.join("store-icon.png");
        icon.to_rgb8().save(&normalized).map_err(cli::Error::WriteImageError)?;
        let executable = self.compile("aarch64-apple-ios", developer, report)?;
        Ok(self.bundle(&executable, team, developer, Some(&normalized))?.0)
    }

    pub fn command(developer: &Path, tool: &str) -> Command {
        let mut command = Command::new("xcrun");
        command.env("DEVELOPER_DIR", developer).arg(tool);
        command
    }

    pub fn check(output: std::process::Output, program: &str) -> Result<Vec<u8>> {
        if output.status.success() { return Ok(output.stdout); }
        Err(cli::Error::CommandFailed { program: program.into(), output }.into())
    }
}

struct IOSConfig {
    team: Option<String>,
    build_number: Option<String>,
    bundle_id: Option<String>,
    minimum_os: String,
    info_plist: Option<PathBuf>,
    entitlements: Option<PathBuf>,
    resources: Vec<PathBuf>,
}

impl IOSConfig {
    fn new(metadata: Option<&Value>) -> Self {
        let text = |name| {
            metadata.and_then(|value| value.get(name)).and_then(Value::as_str).map(str::to_owned)
        };
        Self {
            team: text("team"),
            build_number: text("build_number"),
            bundle_id: text("bundle_id"),
            minimum_os: text("minimum_os").unwrap_or_else(|| "17.0".into()),
            info_plist: text("info_plist").map(PathBuf::from),
            entitlements: text("entitlements").map(PathBuf::from),
            resources: metadata.and_then(|value| value.get("resources"))
                .and_then(Value::as_array).into_iter().flatten()
                .filter_map(Value::as_str).map(PathBuf::from).collect(),
        }
    }
}

impl IOSApplication {
    pub fn prepare_target(&self, target: &str) -> Result<()> {
        if !matches!(target, "aarch64-apple-ios" | "aarch64-apple-ios-sim" | "x86_64-apple-ios") {
            return Err(Error::IosTargetUnsupported { target: target.into() }.into());
        }
        let installed = Command::new("rustup")
            .args(["target", "list", "--installed"])
            .output()
            .map_err(|error| cli::Error::BuildError(error.to_string()))?;
        let installed = Self::check(installed, "rustup")?;
        if !String::from_utf8_lossy(&installed).lines().any(|line| line == target) {
            let output = Command::new("rustup")
                .args(["target", "add", target])
                .output()
                .map_err(|error| cli::Error::BuildError(error.to_string()))?;
            Self::check(output, "rustup")?;
        }
        Ok(())
    }

    fn compile(&mut self, target: &str, developer: &Path, report: &mut dyn FnMut(&str)) -> Result<PathBuf> {
        self.args.cargo.target = Some(target.into());
        let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        command.arg("rustc").arg("--message-format=json-render-diagnostics");
        self.args.cargo.apply(&mut command);
        if self.args.locked { command.arg("--locked"); }
        if self.args.offline { command.arg("--offline"); }
        command.args([
            "--", "-C", "link-arg=-Wl,-rpath,/usr/lib/swift",
            "-C", "link-arg=-Wl,-rpath,@executable_path/Frameworks",
        ]);
        command
            .current_dir(self.package.manifest_path.parent().unwrap())
            .env("DEVELOPER_DIR", developer)
            .env("IPHONEOS_DEPLOYMENT_TARGET", &self.config.minimum_os)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        command.env_remove("RAMP_NOTIFICATION_APPLICATION");
        if let Some(notifications) = &self.notifications { notifications.compile(&mut command); }
        let mut child =
            command.spawn().map_err(|error| cli::Error::BuildError(error.to_string()))?;
        let mut executable = None;
        for message in Message::parse_stream(BufReader::new(child.stdout.take().unwrap())) {
            match message {
                Ok(Message::CompilerArtifact(artifact))
                    if artifact.package_id == self.package.id
                        && artifact.target.name == self.args.cargo.bin[0] =>
                {
                    if let Some(path) = artifact.executable {
                        executable = Some(path.into_std_path_buf());
                    }
                }
                Ok(Message::CompilerMessage(message)) => {
                    if let Some(rendered) = message.message.rendered { report(&rendered); }
                }
                Ok(Message::TextLine(line)) => report(&format!("{line}\n")),
                _ => {}
            }
        }
        let status = child.wait().map_err(|error| cli::Error::BuildError(error.to_string()))?;
        if !status.success() {
            return Err(cli::Error::CommandStatus { program: "cargo".into(), status }.into());
        }
        executable.ok_or_else(|| cli::Error::ArtifactNotProduced {
            package: self.package.name.to_string(), target: Some(target.into()),
        }.into())
    }

    fn bundle_id(&self, team: &str) -> Result<String> {
        let slug: String = self
            .package
            .name
            .chars()
            .map(|character| if character.is_ascii_alphanumeric() { character } else { '-' })
            .collect();
        let identifier =
            self.args.bundle_id.clone().or(self.config.bundle_id.clone()).unwrap_or_else(|| {
                format!("dev.piston.{}.{}", team.to_lowercase(), slug.to_lowercase())
            });
        if identifier.split('.').any(|part| part.is_empty())
            || !identifier.contains('.')
            || !identifier.chars().all(|character| {
                character.is_ascii_alphanumeric() || character == '.' || character == '-'
            })
        { return Err(cli::Error::InvalidBundleId { value: identifier }.into()); }
        Ok(identifier)
    }

    fn bundle(&self, executable: &Path, team: &str, developer: &Path, icon: Option<&Path>) -> Result<(PathBuf, String)> {
        let identifier = self.bundle_id(team)?;
        let slug: String = self
            .package
            .name
            .chars()
            .map(|character| if character.is_ascii_alphanumeric() { character } else { '-' })
            .collect();
        let bundle = self.work.join("staging").join(format!("{slug}.app"));
        if bundle.exists() {
            fs::remove_dir_all(&bundle)
                .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
        }
        fs::create_dir_all(&bundle)
            .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
        let source = self.args.info_plist.as_ref().or(self.config.info_plist.as_ref());
        let mut plist = if let Some(path) = source {
            let output = Command::new("plutil")
                .args(["-convert", "json", "-o", "-"])
                .arg(self.project_path(path))
                .output()
                .map_err(|error| crate::apple::cli::Error::PlutilConvertError(error.to_string()))?;
            let bytes = Self::check(output, "plutil")?;
            serde_json::from_slice::<Value>(&bytes)
                .map_err(|error| cli::Error::IntoJSONError(error.to_string()))?
        } else {
            json!({})
        };
        let settings = plist
            .as_object_mut()
            .ok_or_else(|| crate::apple::cli::Error::PlistNotDictionary {
                path: self.project_path(source.unwrap()),
            })?;
        let version = format!(
            "{}.{}.{}",
            self.package.version.major, self.package.version.minor, self.package.version.patch
        );
        for (key, value) in [
            ("CFBundleIdentifier", json!(identifier)),
            ("CFBundleExecutable", json!(slug)),
            ("CFBundleName", json!(self.package.name)),
            ("CFBundlePackageType", json!("APPL")),
            ("CFBundleInfoDictionaryVersion", json!("6.0")),
            ("CFBundleShortVersionString", json!(version)),
            ("CFBundleVersion", json!(self.config.build_number.as_deref().unwrap_or(&version))),
            ("MinimumOSVersion", json!(self.config.minimum_os)),
            (
                "CFBundleSupportedPlatforms",
                json!([if self.sdk() == "iphonesimulator" {
                    "iPhoneSimulator"
                } else {
                    "iPhoneOS"
                }]),
            ),
            ("UIDeviceFamily", json!([1, 2])),
        ] { settings.insert(key.into(), value); }
        if settings
            .get("CFBundleDevelopmentRegion")
            .and_then(Value::as_str)
            .is_none_or(|region| region.contains("$("))
        { settings.insert("CFBundleDevelopmentRegion".into(), json!("en")); }
        settings.entry("CFBundleDisplayName").or_insert(json!(self.package.name));
        if !["UILaunchStoryboardName", "UILaunchStoryboards", "UILaunchScreens"]
            .iter()
            .any(|key| settings.contains_key(*key))
        { settings.entry("UILaunchScreen").or_insert(json!({})); }
        settings.entry("UISupportedInterfaceOrientations").or_insert(json!([
            "UIInterfaceOrientationPortrait",
            "UIInterfaceOrientationPortraitUpsideDown",
            "UIInterfaceOrientationLandscapeLeft",
            "UIInterfaceOrientationLandscapeRight"
        ]));
        fs::copy(executable, bundle.join(&slug))
            .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
        for resource in &self.config.resources {
            let source = self.project_path(resource);
            let name = source.file_name().ok_or_else(|| Error::IosResourceInvalid {
                path: source.clone(),
            })?;
            if name == "Info.plist" || name == slug.as_str() {
                return Err(Error::IosResourceConflict { path: source }.into());
            }
            let storyboard = source.extension().is_some_and(|extension| extension == "storyboard");
            let output = if storyboard {
                Self::command(developer, "ibtool")
                    .arg("--compile")
                    .arg(bundle.join(name).with_extension("storyboardc"))
                    .arg(&source)
                    .args([
                        "--minimum-deployment-target",
                        &self.config.minimum_os,
                        "--target-device",
                        "iphone",
                        "--target-device",
                        "ipad",
                    ])
                    .output()
            } else {
                Command::new("ditto").arg(&source).arg(bundle.join(name)).output()
            }
            .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
            Self::check(output, if storyboard { "ibtool" } else { "ditto" })?;
        }
        let icon = icon.map(Path::to_path_buf).or_else(|| {
            Project::find_icon(self.package.manifest_path.parent().unwrap().as_std_path())
        });
        let assets = self.bundle_assets(icon.as_deref(), &bundle, developer)?;
        for key in ["CFBundleIcons", "CFBundleIcons~ipad"] {
            if let Some(value) = assets.get(key) { plist[key] = value.clone(); }
        }
        let plist_path = bundle.join("Info.plist");
        fs::write(&plist_path, serde_json::to_vec_pretty(&plist).unwrap())
            .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
        let output = Command::new("plutil")
            .args(["-convert", "xml1"])
            .arg(&plist_path)
            .output()
            .map_err(|error| crate::apple::cli::Error::PlutilConvertError(error.to_string()))?;
        Self::check(output, "plutil")?;
        Ok((bundle, identifier))
    }

    fn bundle_assets(&self, icon: Option<&Path>, bundle: &Path, developer: &Path) -> Result<Value> {
        let catalog = self.work.join("Assets.xcassets");
        if catalog.exists() {
            fs::remove_dir_all(&catalog)
                .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
        }
        fs::create_dir_all(&catalog)
            .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
        let mut populated = false;
        for resource in &self.config.resources {
            let source = self.project_path(resource);
            let extension = source.extension().and_then(|value| value.to_str()).unwrap_or("");
            if !matches!(extension.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg") {
                continue;
            }
            let name = source.file_stem().unwrap().to_string_lossy();
            let set = catalog.join(format!("{name}.imageset"));
            fs::create_dir(&set).map_err(|source| Error::IosImageAssetCreate {
                path: set.clone(), source,
            })?;
            let filename = source.file_name().unwrap();
            fs::copy(&source, set.join(filename))
                .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
            let contents = json!({
                "images": [{"idiom": "universal", "filename": filename.to_string_lossy()}],
                "info": {"version": 1, "author": "xcode"},
            });
            fs::write(set.join("Contents.json"), serde_json::to_vec_pretty(&contents).unwrap())
                .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
            populated = true;
        }
        if let Some(source) = icon {
            let image = image::open(source).map_err(|error| cli::Error::OpenImageError {
                path: source.to_path_buf(),
                source: error,
            })?;
            let app_icon = catalog.join("AppIcon.appiconset");
            fs::create_dir_all(&app_icon)
                .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
            let mut images = Vec::new();
            for (idiom, size, scale, pixels) in [
                ("iphone", "20", 2, 40),
                ("iphone", "20", 3, 60),
                ("iphone", "29", 2, 58),
                ("iphone", "29", 3, 87),
                ("iphone", "40", 2, 80),
                ("iphone", "40", 3, 120),
                ("iphone", "60", 2, 120),
                ("iphone", "60", 3, 180),
                ("ipad", "20", 1, 20),
                ("ipad", "20", 2, 40),
                ("ipad", "29", 1, 29),
                ("ipad", "29", 2, 58),
                ("ipad", "40", 1, 40),
                ("ipad", "40", 2, 80),
                ("ipad", "76", 1, 76),
                ("ipad", "76", 2, 152),
                ("ipad", "83.5", 2, 167),
                ("ios-marketing", "1024", 1, 1024),
            ] {
                let filename = format!("{idiom}-{size}@{scale}x.png");
                image
                    .resize_exact(pixels, pixels, image::imageops::FilterType::Lanczos3)
                    .save(app_icon.join(&filename))
                    .map_err(cli::Error::WriteImageError)?;
                images.push(json!({
                    "idiom": idiom, "size": format!("{size}x{size}"),
                    "scale": format!("{scale}x"), "filename": filename,
                }));
            }
            let contents = json!({ "images": images, "info": { "version": 1, "author": "xcode" } });
            fs::write(
                app_icon.join("Contents.json"),
                serde_json::to_vec_pretty(&contents).unwrap(),
            )
            .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
            populated = true;
        }
        if !populated { return Ok(json!({})); }
        let partial = self.work.join("assets.plist");
        let mut command = Self::command(developer, "actool");
        command
            .args([
                "--output-format",
                "human-readable-text",
                "--warnings",
                "--errors",
                "--platform",
                self.sdk(),
                "--minimum-deployment-target",
                &self.config.minimum_os,
                "--target-device",
                "iphone",
                "--target-device",
                "ipad",
                "--output-partial-info-plist",
            ])
            .arg(&partial)
            .arg("--compile")
            .arg(bundle)
            .arg(&catalog);
        if icon.is_some() { command.args(["--app-icon", "AppIcon"]); }
        let output = command
            .output()
            .map_err(|error| cli::Error::CommandSpawn { program: "actool".into(), source: error })?;
        Self::check(output, "actool")?;
        let output = Command::new("plutil")
            .args(["-convert", "json", "-o", "-"])
            .arg(&partial)
            .output()
            .map_err(|error| crate::apple::cli::Error::PlutilConvertError(error.to_string()))?;
        let bytes = Self::check(output, "plutil")?;
        serde_json::from_slice(&bytes)
            .map_err(|error| cli::Error::IntoJSONError(error.to_string()).into())
    }

    fn project_path(&self, path: &Path) -> PathBuf {
        self.package.manifest_path.parent().unwrap().as_std_path().join(path)
    }
}
