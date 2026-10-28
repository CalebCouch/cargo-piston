use cargo_subcommand::Subcommand;
use clap::Parser;
use std::env;
use std::fmt;
use std::path::PathBuf;
use crate::{android, dev, developer, devices, host, ios, linux, macos, windows};
use crate::host::Host;
use crate::project::Project;
use crate::Result;

#[derive(Debug)]
pub enum SetupPage { Url(String), XcodeAccounts(PathBuf), }

#[derive(Debug)]
pub enum Error {
    AppSelectionInvalid,
    Lipo { first_binary: PathBuf, second_binary: PathBuf, source: String },
    MacOSIcon { input_path: PathBuf, output_path: PathBuf, source: std::io::Error },
    AppPackageMissing,
    AppLibraryMissing,
    AppBinaryMissing,
    FileMetadata { path: PathBuf, source: std::io::Error },
    SaveImage { path: PathBuf, source: image::ImageError },
    ManifestTableRequired { path: PathBuf, section: String },
    ManifestReadOnly { path: PathBuf },
    ManifestChanged { path: PathBuf },
    ManifestPackageMissing { path: PathBuf },
    ManifestParse { path: PathBuf, source: toml_edit::TomlError },
    ManifestWrite { path: PathBuf, source: std::io::Error },
    CommandWait { program: std::ffi::OsString, source: std::io::Error },
    CommandLogged { log: PathBuf, diagnostics: String, error: Box<dyn std::error::Error> },
    SetPermissions { path: PathBuf, source: std::io::Error },
    CommandSpawn { program: std::ffi::OsString, source: std::io::Error },
    CommandFailed { program: std::ffi::OsString, output: std::process::Output },
    CommandStatus { program: std::ffi::OsString, status: std::process::ExitStatus },
    MissingEnvironment { name: &'static str },
    MissingSetting { key: String },
    InvalidBundleId { value: String },
    ArtifactNotProduced { package: String, target: Option<String> },
    ReadFile { path: PathBuf, source: std::io::Error },
    SetupRequired { error: Box<dyn std::error::Error>, page: SetupPage },
    Build(String),
    CargoParse(String),
    WriteFile(String),
    WriteImage(image::ImageError),
    UnsupportedTarget(String),
    ParseUTF8(String),
    IntoJSON(String),
    CopyFile { input_path: PathBuf, output_path: PathBuf, source: std::io::Error },
    ReadDir { path: PathBuf, source: std::io::Error },
    MapDir { path: PathBuf, source: std::io::Error },
    RemoveSubdir { path: PathBuf, source: std::io::Error },
    RemoveFile { path: PathBuf, source: std::io::Error },
    CreateFile { path: PathBuf, source: std::io::Error },
    RenameFile { path: PathBuf, source: std::io::Error },
    CreateDirAll { path: PathBuf, source: std::io::Error },
    OpenImage { path: PathBuf, source: image::ImageError },
    CurrentDirectory(std::io::Error),
    EnvironmentFile(std::io::Error),
    Subcommand(cargo_subcommand::Error),
    TargetUnsupported { target: Option<String> },
    DeviceNotFound,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lipo { first_binary, second_binary, source } => write!(
                f, "Error making universal binary with Lipo first binary: {:?}, \
                 second binary: {:?}, source: {}", first_binary, second_binary, source,
            ),
            Self::MacOSIcon { input_path, output_path, .. } => {
                write!(f, "Failed to format icon {:?} to {:?}", input_path, output_path)
            }
            Self::AppSelectionInvalid => write!(f,
                "Select one app package; Android uses its cdylib, desktop and iOS use its binary."),
            Self::AppPackageMissing => write!(f,
                "Select an application package with --package when running from a workspace."),
            Self::AppLibraryMissing => write!(f,
                "Android needs an app library with crate-type = [\"cdylib\"] and an android_main \
                entry point in Maverick."),
            Self::AppBinaryMissing => write!(f,
                "Choose an application executable with --bin (or set package.default-run)."),
            Self::FileMetadata { path, source } =>
                write!(f, "Cannot read metadata for {}: {source}", path.display()),
            Self::SaveImage { path, source } =>
                write!(f, "Cannot save image {}: {source}", path.display()),
            Self::ManifestTableRequired { path, section } => write!(f,
                "Expected a table for package.metadata.piston.{section} in {}.", path.display()),
            Self::ManifestReadOnly { path } => write!(f, "{} is read-only.", path.display()),
            Self::ManifestChanged { path } => write!(f,
                "{} changed during setup; rerun to save settings.", path.display()),
            Self::ManifestPackageMissing { path } => write!(f,
                "{} has no application package. Select a package with --package.", path.display()),
            Self::ManifestParse { path, source } =>
                write!(f, "Invalid {}: {source}", path.display()),
            Self::ManifestWrite { path, source } =>
                write!(f, "Cannot save Piston settings in {}: {source}", path.display()),
            Self::CommandWait { program, source } => {
                write!(f, "Cannot wait for {}: {source}", program.to_string_lossy())
            }
            Self::CommandLogged { log, diagnostics, error } => {
                write!(f, "{error}. Full log: {}\n{diagnostics}", log.display())
            }
            Self::SetPermissions { path, source } => {
                write!(f, "Cannot make {} executable: {source}", path.display())
            }
            Self::CommandSpawn { program, source } => {
                write!(f, "Could not start {}: {source}", program.to_string_lossy())
            }
            Self::CommandFailed { program, output } => write!(
                f, "{} failed ({}).\n{}{}", program.to_string_lossy(), output.status,
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr),
            ),
            Self::CommandStatus { program, status } => {
                write!(f, "{} failed: {status}", program.to_string_lossy())
            }
            Self::MissingEnvironment { name } => write!(f, "Set the {name} environment variable."),
            Self::MissingSetting { key } => write!(f, "Missing configuration setting: {key}."),
            Self::InvalidBundleId { value } => write!(f, "Invalid bundle identifier: {value}."),
            Self::ArtifactNotProduced { package, target } => {
                write!(f, "Cargo produced no application artifact for {package}")?;
                if let Some(target) = target { write!(f, " ({target})")?; }
                Ok(())
            }
            Self::ReadFile { path, source } => {
                write!(f, "Could not read {}: {source}", path.display())
            }
            Self::SetupRequired { error, .. } => fmt::Display::fmt(error, f),
            Self::Build(err) => write!(f, "Failed to build the binary: {}", err),
            Self::CargoParse(err) => {
                write!(f, "Could not read the Cargo project:\n{err}")?;
                if err.contains("Operation not permitted") || err.contains("Permission denied") {
                    write!(f, "\n\nAllow your terminal app to read the project folder. \
                        If it is in Documents, Desktop, or Downloads, check macOS System Settings \
                        > Privacy & Security > Files and Folders, or move the project to an \
                        accessible folder. Then rerun the same cargo piston command.")?;
                }
                Ok(())
            }
            Self::WriteFile(err) => write!(f, "Failed to write file: {}", err),
            Self::WriteImage(err) => write!(f, "Failed to write image: {}", err),
            Self::UnsupportedTarget(err) => {
                write!(f, "User Specified Target is Unsupported: {}", err)
            }
            Self::ParseUTF8(err) => write!(f, "Error Parsing UTF8: {}", err),
            Self::IntoJSON(err) => write!(f, "Error converting into JSON: {}", err),
            Self::CopyFile { input_path, output_path, .. } => {
                write!(f, "Failed to copy {:?} to {:?}", input_path, output_path)
            }
            Self::ReadDir { path, .. } => write!(f, "Failed to read directory {:?}", path),
            Self::MapDir { path, .. } => {
                write!(f, "Failed to map directory contents {:?}", path)
            }
            Self::RemoveSubdir { path, .. } => {
                write!(f, "Failed to remove subdirectory {:?}", path)
            }
            Self::RemoveFile { path, .. } => write!(f, "Failed to remove file {:?}", path),
            Self::CreateFile { path, .. } => write!(f, "Failed to Create file {:?}", path),
            Self::RenameFile { path, .. } => write!(f, "Failed to Rename file {:?}", path),
            Self::CreateDirAll { path, .. } => {
                write!(f, "Failed to create dir all {:?}", path)
            }
            Self::OpenImage { path, .. } => write!(f, "Failed to open image {:?}", path),
            Self::CurrentDirectory(source) => write!(f, "Error getting working directory: {source}"),
            Self::EnvironmentFile(source) => write!(f, "Error loading .env: {source}"),
            Self::Subcommand(source) => write!(f, "Error parsing subcommand: {source}"),
            Self::TargetUnsupported { target } => write!(f, "Unknown or unsupported target: {target:?}"),
            Self::DeviceNotFound => write!(f, "Device not found"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MacOSIcon { source, .. } => Some(source),
            Self::FileMetadata { source, .. } => Some(source),
            Self::ManifestWrite { source, .. } => Some(source),
            Self::SaveImage { source, .. } => Some(source),
            Self::ManifestParse { source, .. } => Some(source),
            Self::CommandWait { source, .. } => Some(source),
            Self::SetPermissions { source, .. } => Some(source),
            Self::CommandLogged { error, .. } => Some(error.as_ref()),
            Self::CommandSpawn { source, .. } => Some(source),
            Self::ReadFile { source, .. } => Some(source),
            Self::SetupRequired { error, .. } => Some(error.as_ref()),
            Self::CopyFile { source, .. } => Some(source),
            Self::ReadDir { source, .. } => Some(source),
            Self::MapDir { source, .. } => Some(source),
            Self::RemoveSubdir { source, .. } => Some(source),
            Self::RemoveFile { source, .. } => Some(source),
            Self::CreateFile { source, .. } => Some(source),
            Self::CreateDirAll { source, .. } => Some(source),
            Self::RenameFile { source, .. } => Some(source),
            Self::OpenImage { source, .. } => Some(source),
            Self::WriteImage(source) => Some(source),
            Self::CurrentDirectory(source) | Self::EnvironmentFile(source) => Some(source),
            Self::Subcommand(source) => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Parser)]
pub struct ApplicationArgs {
    #[command(flatten)]
    pub cargo: cargo_subcommand::Args,
    /// Apple signing team. Saved in the selected platform's Cargo metadata.
    #[arg(long)]
    pub team: Option<String>,
    /// App identifier. Development generates a default; store deployment requires one.
    #[arg(long)]
    pub bundle_id: Option<String>,
    /// Additional app settings, including privacy descriptions, from an Info.plist.
    #[arg(long)]
    pub info_plist: Option<PathBuf>,
    /// Entitlements to use when signing the development app.
    #[arg(long)]
    pub entitlements: Option<PathBuf>,
    /// Require the existing Cargo.lock.
    #[arg(long)]
    pub locked: bool,
    /// Build Cargo dependencies without accessing the network.
    #[arg(long)]
    pub offline: bool,
}

#[derive(Parser)]
#[command(name = "piston")]
pub struct Cli {
    #[clap(subcommand)]
    piston: PistonCmd,
}

impl Cli {
    pub fn run() {
        if let Err(error) = Self::start() {
            eprintln!("\nerror: {error}");
            let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error.as_ref());
            while let Some(current) = source {
                if let Some(Error::SetupRequired { page, .. }) = current.downcast_ref::<Error>() {
                    match page {
                        SetupPage::Url(url) => {
                            eprintln!("\n{}", host::Cli::link("Continue setup", url));
                        }
                        SetupPage::XcodeAccounts(developer) => {
                            eprintln!("\n{}", host::Cli::link(
                                "Xcode Settings > Accounts", "xcpref://Xcode.PreferencePane.Accounts",
                            ));
                            eprintln!("Selected Xcode: {}", developer.display());
                        }
                    }
                    break;
                }
                source = current.source();
            }
            std::process::exit(1);
        }
    }

    pub fn start() -> Result<()> {
        let cwd = env::current_dir().map_err(Error::CurrentDirectory)?;
        let Self { piston: PistonCmd::Piston { cmd } } = Self::parse();
        let Some(cmd) = cmd else { return developer::Cli::start(false); };
        match &cmd {
            PistonSubCmd::Setup => return developer::Cli::start(true),
            PistonSubCmd::Backup => return developer::Cli::backup(),
            PistonSubCmd::Restore { secret } => return developer::Cli::restore(secret),
            _ => (),
        }

        if let PistonSubCmd::Dev(args) = cmd { return dev::Cli::start(args); }
        if let PistonSubCmd::Deploy(args) = cmd { return ios::deployment::Cli::start(args); }
        if let PistonSubCmd::Version = cmd {
            println!("{}, {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        let env_vars = match Project::load_env_file() {
            Ok(values) => values,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(error) => return Err(Error::EnvironmentFile(error).into()),
        };

        match cmd {
            PistonSubCmd::Dev(_) | PistonSubCmd::Deploy(_) | PistonSubCmd::Version
            | PistonSubCmd::Setup | PistonSubCmd::Backup | PistonSubCmd::Restore { .. } => {
                unreachable!()
            }
            PistonSubCmd::Build(args) => {
                if args.dry_run {
                    println!("Dry run: would build with {:?}", args.common);
                    return Ok(());
                }
                let manifest = Project::manifest(&args.common.subcommand_args)?;
                developer::Cli::credentials(&manifest, None, false)?;
                Host::check_build()?;
                let cmd = Subcommand::new(args.common.subcommand_args).map_err(Error::Subcommand)?;
                let target = cmd.target();
                let platform = Platform::from_target(target.unwrap_or(Host::default_target()));
                let release = cmd.args().release;
                let external = args.common.external;
                let appimage = args.common.appimage;
                let release_override = release || external || appimage;
                let target_string = Project::build_target(target)?;
                let profile = if release_override { "release" } else { "debug" };
                println!("Building for {target_string} ({profile})");
                match platform {
                    Platform::Android => {
                        android::Cli::build(release_override, target_string, cwd, env_vars)?;
                    }
                    Platform::Ios => {
                        ios::Cli::build(release_override, target_string, cwd, env_vars, None)?;
                    }
                    Platform::Linux => {
                        linux::Cli::build(release_override, appimage, target_string, cwd, env_vars)?;
                    }
                    Platform::Windows => {
                        windows::Cli::build(release_override, target_string, cwd, env_vars)?;
                    }
                    Platform::Macos => {
                        macos::Cli::build(release_override, external, target_string, cwd, env_vars)?;
                    }
                    Platform::Unknown => {
                        return Err(Error::TargetUnsupported { target: target.map(str::to_owned) }.into());
                    }
                }
            }
            PistonSubCmd::Run(args) => {
                let manifest = Project::manifest(&args.common.subcommand_args)?;
                developer::Cli::credentials(&manifest, None, false)?;
                let cmd = Subcommand::new(args.common.subcommand_args).map_err(Error::Subcommand)?;
                let release = cmd.args().release;
                if let Some(device) = args.device {
                    let target = device.trim();
                    println!("Selecting device: {device}");
                    let devices = devices::Cli::list(env_vars.clone(), true)?;
                    let android = devices.android.iter().find(|device| device.id == target);
                    let ios = devices.ios.iter().find(|device| device.id == target);
                    if target == "ios" && !devices.ios.is_empty() {
                        let device = &devices.ios[0];
                        println!("Using {} ({})", device.model, device.id);
                        ios::Cli::run(release, cwd, env_vars, device)?;
                    } else if target == "android" && !devices.android.is_empty() {
                        let device = &devices.android[0];
                        println!("Using Android device {}", device.id);
                        android::Cli::run(release, cwd, env_vars, device)?;
                    } else if let Some(device) = android {
                        println!("Using Android device {}", device.id);
                        android::Cli::run(release, cwd, env_vars, device)?;
                    } else if let Some(device) = ios {
                        println!("Using {} ({})", device.model, device.id);
                        ios::Cli::run(release, cwd, env_vars, device)?;
                    } else {
                        return Err(Error::DeviceNotFound.into());
                    }
                } else {
                    println!("Running on this computer");
                    host::Cli::run_local(release, cwd, env_vars)?;
                }
            }
            PistonSubCmd::ListDevices => {
                println!("list all available connected devices and relevant information");
                devices::Cli::list(env_vars, false)?;
            }
        }
        Ok(())
    }
}


#[derive(Clone, Debug, Eq, PartialEq, Parser)]
#[group(skip)]
struct CommonArgs {
    #[clap(flatten)]
    subcommand_args: cargo_subcommand::Args,
    #[clap(long = "release-external")]
    external: bool,
    #[clap(long = "release-appimage")]
    appimage: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Parser)]
#[group(skip)]
struct BuildArgs {
    #[clap(flatten)]
    common: CommonArgs,
    #[clap(long)]
    dry_run: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Parser)]
#[group(skip)]
struct RunArgs {
    #[clap(flatten)]
    common: CommonArgs,
    #[clap(long)]
    device: Option<String>,
}

#[derive(clap::Subcommand)]
enum PistonSubCmd {
    /// Choose or create this project's developer team identity.
    Setup,
    /// Select a team identity and print its private backup data.
    Backup,
    /// Restore the quoted private data printed by cargo piston backup.
    Restore { secret: String },
    /// Build and run on the host, a mobile device/emulator, or a Linux VM.
    Dev(crate::dev::DevelopmentArgs),
    /// Upload an iOS build to TestFlight or the App Store.
    Deploy(crate::ios::DeploymentArgs),
    /// Build an app bundle for a selected target.
    #[clap(visible_alias = "b")]
    Build(BuildArgs),
    /// Build and launch using the legacy platform runner.
    #[clap(visible_alias = "r")]
    Run(RunArgs),
    /// Show available connected devices.
    #[clap(visible_alias = "l")]
    ListDevices,
    /// Show the installed Piston version.
    #[clap(visible_alias = "v")]
    Version,
}

#[derive(clap::Subcommand)]
enum PistonCmd {
    #[command(
        about = "Build, run, and deploy Rust apps.",
        after_help = "Common commands:\n  \
            cargo piston dev                    Run on this computer\n  \
            cargo piston dev ios                Use an iPhone or iOS Simulator\n  \
            cargo piston dev android            Use an Android device or emulator\n  \
            cargo piston deploy testflight      Upload and wait for TestFlight\n\n\
            Run cargo piston to choose this project's developer identity.\n\
            Setup links open when clicked. Enter q at a prompt to cancel.\n\
            Use cargo piston <command> --help for command options."
    )]
    Piston {
        #[clap(subcommand)]
        cmd: Option<PistonSubCmd>,
    },
}

#[derive(Debug)]
enum Platform { Android, Ios, Linux, Windows, Macos, Unknown, }

impl Platform {
    fn from_target(target: &str) -> Self {
        match target.to_lowercase().as_str() {
            "macos" | "aarch64-apple-darwin" | "x86_64-apple-darwin" => Self::Macos,
            "ios" | "aarch64-apple-ios" | "x86_64-apple-ios" => Self::Ios,
            "linux" | "aarch64-unknown-linux-gnu" | "x86_64-unknown-linux-gnu" => Self::Linux,
            "windows" | "x86_64-pc-windows-gnu" => Self::Windows,
            "android" | "aarch64-linux-android" | "x86_64-linux-android" => Self::Android,
            _ => Self::Unknown,
        }
    }
}

#[test]
fn test_platform_from_target() {
    assert!(matches!(Platform::from_target("aarch64-apple-darwin"), Platform::Macos));
    assert!(matches!(Platform::from_target("aarch64-apple-ios"), Platform::Ios));
    assert!(matches!(Platform::from_target("aarch64-unknown-linux-gnu"), Platform::Linux));
    assert!(matches!(Platform::from_target("x86_64-pc-windows-gnu"), Platform::Windows));
    assert!(matches!(Platform::from_target("aarch64-linux-android"), Platform::Android));
    assert!(matches!(Platform::from_target("some-unknown-target"), Platform::Unknown));
}
