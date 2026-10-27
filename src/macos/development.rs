use plist::{Dictionary, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use crate::{cli, Result};
use crate::apple::{NotificationSetup, PushBuild, PushPlatform, PushSigning, XcodeSigning};
use crate::manifest::Manifest;
use crate::project::Project;
use super::cli::Error;

pub struct MacDevelopment {
    project: Project,
    developer: PathBuf,
    bundle_id: String,
    notifications: Option<PushBuild>,
    app_args: Vec<String>,
}

impl MacDevelopment {
    pub(crate) fn new(mut project: Project, app_args: Vec<String>, developer: PathBuf) -> Result<Self> {
        let target = project.args.cargo.target.clone().unwrap_or(Project::host_target()?);
        if !matches!(target.as_str(), "aarch64-apple-darwin" | "x86_64-apple-darwin") {
            return Err(cli::Error::UnsupportedTargetError(target).into());
        }
        project.args.cargo.target = Some(target.clone());
        Project::rust_target(&target)?;
        let manifest = Manifest::new(project.package.manifest_path.as_std_path())?;
        let settings = manifest.settings(&["macos"])?;
        let bundle_id = project
            .args
            .bundle_id
            .clone()
            .or_else(|| settings.get("bundle_id").cloned())
            .or(manifest.setting(&["ios", "bundle_id"])?
                .as_ref().and_then(toml_edit::Item::as_str).map(str::to_owned))
            .unwrap_or_else(|| format!("dev.piston.{}", project.package.name.replace('_', "-")));
        if !bundle_id.contains('.')
            || bundle_id.split('.').any(str::is_empty)
            || !bundle_id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".-".contains(character))
        { return Err(cli::Error::InvalidBundleId { value: bundle_id }.into()); }
        Ok(Self { project, developer, bundle_id, notifications: None, app_args })
    }

    pub fn project(&self) -> &Project { &self.project }
    pub fn developer(&self) -> &Path { &self.developer }
    pub fn notifications(&mut self, push: PushBuild) { self.notifications = Some(push); }

    pub fn valid_setting(name: &str, value: &toml_edit::Item, root: &Path) -> bool {
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
            "info_plist" | "entitlements" => !value.is_empty()
                && Value::from_file(root.join(value))
                    .is_ok_and(|value| value.as_dictionary().is_some()),
            _ => unreachable!(),
        }
    }

    pub fn notification_setup(&self, team: &str) -> Result<NotificationSetup> {
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

    pub fn build_command(&self) -> Command {
        let mut command = self.project.build_command();
        command.env("DEVELOPER_DIR", &self.developer);
        if let Some(push) = &self.notifications { push.compile(&mut command); }
        command
    }

    pub fn bundle(&self, executable: &Path) -> Result<PathBuf> {
        let project = &self.project;
        let name = project.package.name.as_str();
        let bundle = project.work.join(format!("{name}.app"));
        if bundle.exists() {
            fs::remove_dir_all(&bundle)
                .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
        }
        let contents = bundle.join("Contents");
        let binary = contents.join("MacOS").join(name);
        fs::create_dir_all(binary.parent().unwrap())
            .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
        fs::copy(executable, &binary)
            .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
        let manifest = Manifest::new(project.package.manifest_path.as_std_path())?;
        let config = manifest.settings(&["macos"])?;
        let source = project.args.info_plist.clone()
            .or_else(|| config.get("info_plist").map(PathBuf::from));
        let source = source.map(|path| project.root().join(path));
        let mut info = if let Some(source) = &source {
            Value::from_file(source)?
        } else {
            Value::Dictionary(Dictionary::new())
        };
        let dict = info
            .as_dictionary_mut()
            .ok_or_else(|| crate::apple::cli::Error::PlistNotDictionary { path: source.unwrap() })?;
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
                .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
            let image = image::open(&icon)
                .map_err(|source| cli::Error::OpenImageError { path: icon, source })?;
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
                        .map_err(cli::Error::WriteImageError)?;
                }
            }
            Project::output(
                Command::new("iconutil")
                    .args(["-c", "icns"])
                    .arg(&iconset)
                    .arg("-o")
                    .arg(resource.join("AppIcon.icns")),
            )?;
            dict.insert("CFBundleIconFile".into(), Value::String("AppIcon.icns".into()));
        }
        if let Some(resources) = manifest.setting(&["macos", "resources"])?
            .as_ref().and_then(toml_edit::Item::as_array)
        {
            for resource in resources {
                let resource = resource.as_str().ok_or(Error::MacResourcesInvalid)?;
                let path = project.root().join(resource);
                let name = path
                    .file_name()
                    .ok_or_else(|| Error::MacResourceInvalid { path: path.clone() })?;
                Project::output(
                    Command::new("ditto").arg(&path).arg(contents.join("Resources").join(name)),
                )?;
            }
        }
        info.to_file_xml(contents.join("Info.plist"))
            .map_err(|error| cli::Error::WriteFileError(error.to_string()))?;
        let entitlements = project
            .args
            .entitlements
            .clone()
            .or_else(|| config.get("entitlements").map(PathBuf::from))
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
            Project::output(sign.arg(&bundle))?;
            Ok(bundle)
        }
    }

    pub fn launch(&self, bundle: &Path) -> Result<()> {
        self.project.launch(
            &bundle.join("Contents/MacOS").join(self.project.package.name.as_str()),
            &self.app_args,
        )
    }
}
