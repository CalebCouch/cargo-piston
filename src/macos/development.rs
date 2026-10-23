use plist::{Dictionary, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use crate::apple::xcode::cli::SigningCli;
use crate::apple::NotificationCli;
use crate::dev::DevelopmentArgs;
use crate::host::cli::HostCli;
use crate::apple::{NotificationSetup, PushBuild, PushPlatform, PushSigning, XcodeSigning};
use crate::manifest::Manifest;
use crate::project::Project;
use crate::{PistonError, Result};

pub struct MacDevelopment {
    project: Project,
    developer: PathBuf,
    bundle_id: String,
    notifications: Option<PushBuild>,
    app_args: Vec<String>,
}

impl MacDevelopment {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        let project = Project::new(args.application, "macos", false)?;
        let developer = HostCli::prepare_xcode("macosx", 0)?;
        let mut session = MacDevelopment::new(project, args.app_args, developer)?;
        let manifest = Manifest::new(session.project.package.manifest_path.as_std_path())?;
        if NotificationCli::enabled(&manifest)? {
            let team = manifest.setting(&["macos", "team"])?;
            let signing = SigningCli::development(
                session
                    .project
                    .args
                    .team
                    .as_deref()
                    .or(team.as_ref().and_then(toml_edit::Item::as_str)),
                &session.developer,
            )?;
            let setup = session.notification_setup(signing.team())?;
            let application = NotificationCli::configure(&setup)?;
            session.notifications = Some(PushBuild { application, signing: setup.signing });
        }
        println!(
            "Building {} for macOS ({})",
            session.project.package.name,
            session.project.args.cargo.target.as_ref().unwrap()
        );
        let executable =
            session.project.compile(session.build_command(), |message| eprint!("{message}"))?;
        let bundle = session.bundle(&executable)?;
        println!(
            "Opening {}. App output follows; press Ctrl-C to stop.",
            session.project.package.name
        );
        session.launch(&bundle)
    }

    fn new(mut project: Project, app_args: Vec<String>, developer: PathBuf) -> Result<Self> {
        let target = project.args.cargo.target.clone().unwrap_or(Project::host_target()?);
        if !matches!(target.as_str(), "aarch64-apple-darwin" | "x86_64-apple-darwin") {
            return Err(PistonError::UnsupportedTargetError(format!(
                "{target} is not a macOS target."
            )));
        }
        project.args.cargo.target = Some(target.clone());
        Project::rust_target(&target)?;
        let settings = project.package.metadata.pointer("/piston/macos");
        let bundle_id = project
            .args
            .bundle_id
            .clone()
            .or_else(|| {
                settings
                    .and_then(|value| value.get("bundle_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .or_else(|| {
                project
                    .package
                    .metadata
                    .pointer("/piston/ios/bundle_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("dev.piston.{}", project.package.name.replace('_', "-")));
        if !bundle_id.contains('.')
            || bundle_id.split('.').any(str::is_empty)
            || !bundle_id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".-".contains(character))
        { return Err(PistonError::Generic(format!("Invalid bundle identifier: {bundle_id}"))); }
        Ok(Self { project, developer, bundle_id, notifications: None, app_args })
    }

    fn notification_setup(&self, team: &str) -> Result<NotificationSetup> {
        let manifest = Manifest::new(self.project.package.manifest_path.as_std_path())?;
        manifest.save(&["macos"], &[("team", team)])?;
        NotificationSetup::new(
            manifest.path(),
            PushSigning {
                platform: PushPlatform::Macos,
                team: team.into(),
                topic: self.bundle_id.clone(),
                sandbox: true,
            },
        )
    }

    fn build_command(&self) -> Command {
        let mut command = self.project.build_command();
        command.env("DEVELOPER_DIR", &self.developer);
        if let Some(push) = &self.notifications { push.compile(&mut command); }
        command
    }

    fn bundle(&self, executable: &Path) -> Result<PathBuf> {
        let project = &self.project;
        let name = project.package.name.as_str();
        let bundle = project.work.join(format!("{name}.app"));
        if bundle.exists() {
            fs::remove_dir_all(&bundle)
                .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        }
        let contents = bundle.join("Contents");
        let binary = contents.join("MacOS").join(name);
        fs::create_dir_all(binary.parent().unwrap())
            .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        fs::copy(executable, &binary)
            .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        let config = project.package.metadata.pointer("/piston/macos");
        let source = project.args.info_plist.clone().or_else(|| {
            config
                .and_then(|value| value.get("info_plist"))
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from)
        });
        let mut info = if let Some(source) = source {
            Value::from_file(project.root().join(source))
                .map_err(|error| PistonError::Generic(error.to_string()))?
        } else {
            Value::Dictionary(Dictionary::new())
        };
        let dict = info
            .as_dictionary_mut()
            .ok_or_else(|| PistonError::Generic("Info.plist must be a dictionary.".into()))?;
        for (key, value) in [
            ("CFBundleIdentifier", self.bundle_id.clone()),
            ("CFBundleExecutable", name.into()),
            ("CFBundleName", name.into()),
            ("CFBundleDisplayName", name.into()),
            ("CFBundlePackageType", "APPL".into()),
            ("CFBundleInfoDictionaryVersion", "6.0".into()),
            (
                "CFBundleVersion",
                format!(
                    "{}.{}.{}",
                    project.package.version.major,
                    project.package.version.minor,
                    project.package.version.patch
                ),
            ),
            (
                "CFBundleShortVersionString",
                format!(
                    "{}.{}.{}",
                    project.package.version.major,
                    project.package.version.minor,
                    project.package.version.patch
                ),
            ),
        ] { dict.insert(key.into(), Value::String(value)); }
        if !dict.contains_key("NSHighResolutionCapable") {
            dict.insert("NSHighResolutionCapable".into(), Value::Boolean(true));
        }
        if let Some(icon) = Project::find_icon(project.root()) {
            let resource = contents.join("Resources");
            let iconset = project.work.join("AppIcon.iconset");
            fs::create_dir_all(&resource)
                .and_then(|_| fs::create_dir_all(&iconset))
                .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
            let image = image::open(&icon)
                .map_err(|source| PistonError::OpenImageError { path: icon, source })?;
            for size in [16, 32, 128, 256, 512] {
                for scale in [1, 2] {
                    let suffix = if scale == 2 { "@2x" } else { "" };
                    image
                        .resize_exact(
                            size * scale,
                            size * scale,
                            image::imageops::FilterType::Lanczos3,
                        )
                        .save(iconset.join(format!("icon_{size}x{size}{suffix}.png")))
                        .map_err(PistonError::WriteImageError)?;
                }
            }
            Project::output(
                Command::new("iconutil")
                    .args(["-c", "icns"])
                    .arg(&iconset)
                    .arg("-o")
                    .arg(resource.join("AppIcon.icns")),
                "Creating macOS icon",
            )?;
            dict.insert("CFBundleIconFile".into(), Value::String("AppIcon.icns".into()));
        }
        if let Some(resources) =
            config.and_then(|value| value.get("resources")).and_then(serde_json::Value::as_array)
        {
            for resource in resources {
                let resource = resource.as_str().ok_or_else(|| {
                    PistonError::Generic("macos.resources must contain paths.".into())
                })?;
                let path = project.root().join(resource);
                let name = path
                    .file_name()
                    .ok_or_else(|| PistonError::Generic("Invalid resource path.".into()))?;
                Project::output(
                    Command::new("ditto").arg(&path).arg(contents.join("Resources").join(name)),
                    "Copying macOS resource",
                )?;
            }
        }
        info.to_file_xml(contents.join("Info.plist"))
            .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        let entitlements = project
            .args
            .entitlements
            .clone()
            .or_else(|| {
                config
                    .and_then(|value| value.get("entitlements"))
                    .and_then(serde_json::Value::as_str)
                    .map(PathBuf::from)
            })
            .map(|path| project.root().join(path));
        if let Some(push) = &self.notifications {
            let entitlements = push.signing.entitlements(
                entitlements.as_deref(),
                &project.work.join("notifications.entitlements"),
            )?;
            let signed = XcodeSigning::new(&push.signing.team, &self.developer).sign_macos(
                &bundle,
                &project.work,
                &entitlements,
                false,
                false,
            )?;
            push.signing.verify(&signed)?;
            Ok(signed)
        } else {
            let mut sign = Command::new("codesign");
            sign.args(["--force", "--sign", "-", "--timestamp=none"]);
            if let Some(entitlements) = entitlements {
                sign.arg("--entitlements").arg(entitlements);
            }
            Project::output(sign.arg(&bundle), "Signing the local macOS app")?;
            Ok(bundle)
        }
    }

    fn launch(&self, bundle: &Path) -> Result<()> {
        self.project.launch(
            &bundle.join("Contents/MacOS").join(self.project.package.name.as_str()),
            &self.app_args,
        )
    }
}
