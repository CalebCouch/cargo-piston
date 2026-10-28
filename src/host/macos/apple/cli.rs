use std::fmt;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use crate::host;
use crate::Result;
use super::{Release, Xcode};

#[derive(Debug)]
pub enum Error {
    SystemVersionInvalid { value: String },
    ArchitectureUnsupported { architecture: String },
    SelectedTooOld { version: [u32; 3], minimum: u32 },
    SdkUnavailable { developer: PathBuf, sdk: String, minimum: u32, output: std::process::Output },
    Incomplete { developer: PathBuf },
    Incompatible { developer: PathBuf },
    Discovery { path: PathBuf, error: Box<dyn std::error::Error> },
    CatalogUnavailable { source: Box<ureq::Error> },
    SystemTooOld { sdk: String, minimum: [u32; 3] },
    ReleaseUnavailable { architecture: String, sdk: String },
    DestinationExists { path: PathBuf },
    ArchiveMismatch { version: String, build: String, staging: PathBuf },
    InstallMove { destination: PathBuf, staging: PathBuf, source: std::io::Error },
    LicenseIncomplete,
    AdministratorApprovalIncomplete,
    Install(String),
    Build(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SystemVersionInvalid { value } => {
                write!(f, "Cannot read the macOS version: {value:?}.")
            }
            Self::ArchitectureUnsupported { architecture } => {
                write!(f, "Cannot select an Xcode installer for architecture {architecture:?}.")
            }
            Self::SelectedTooOld { version, minimum } => write!(f,
                "DEVELOPER_DIR explicitly selects Xcode {}.{}.{}; this build requires Xcode \
                 {minimum} or later. Unset DEVELOPER_DIR for automatic selection.",
                version[0], version[1], version[2]),
            Self::SdkUnavailable { developer, sdk, minimum, output } => write!(f,
                "Xcode at {} does not provide the required {sdk} SDK (minimum major version {minimum}). \
                 Install the full Xcode application, or unset DEVELOPER_DIR if it selects an incomplete \
                 installation.\n{}", developer.display(), String::from_utf8_lossy(&output.stderr)),
            Self::Incomplete { developer } => {
                write!(f, "{} is not a full Xcode installation.", developer.display())
            }
            Self::Incompatible { developer } => {
                write!(f, "{} is not an Xcode compatible with this macOS.", developer.display())
            }
            Self::Discovery { path, error } => {
                write!(f, "Skipping Xcode at {}: {error}", path.display())
            }
            Self::CatalogUnavailable { source } => write!(f,
                "Cannot load the Xcode Releases catalog: {source}. Existing installations can be \
                 selected with DEVELOPER_DIR; retry setup when the catalog is available."),
            Self::SystemTooOld { sdk, minimum } => write!(f,
                "No compatible release provides the required {sdk} SDK on this Mac. Update macOS \
                 to at least {}.{}.{} and rerun this command.", minimum[0], minimum[1], minimum[2]),
            Self::ReleaseUnavailable { architecture, sdk } => write!(f,
                "The release catalog has no supported Xcode installer for {architecture} and the \
                 required {sdk} SDK.\n\n{}",
                host::Cli::link("Xcode requirements", "https://developer.apple.com/xcode/system-requirements/")),
            Self::DestinationExists { path } => write!(f,
                "{} already exists but was not usable. Move it aside or repair it, then rerun. \
                 Piston will not replace an existing installation.", path.display()),
            Self::ArchiveMismatch { version, build, staging } => write!(f,
                "The archive contains a different Xcode than {version} ({build}). Extracted files \
                 remain at {}.", staging.display()),
            Self::InstallMove { destination, staging, source } => write!(f,
                "Cannot install Xcode at {}: {source}. Extracted files remain at {}.",
                destination.display(), staging.display()),
            Self::LicenseIncomplete => write!(f, "Xcode license acceptance was not completed."),
            Self::AdministratorApprovalIncomplete => {
                write!(f, "Administrator approval was not completed.")
            }
            Self::Install(err) => write!(f, "Xcode Installation not Found: {}", err),
            Self::Build(err) => write!(f, "Xcode could not complete the operation: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InstallMove { source, .. } => Some(source),
            Self::CatalogUnavailable { source } => Some(source.as_ref()),
            Self::Discovery { error, .. } => Some(error.as_ref()),
            _ => None,
        }
    }
}

pub struct Cli;

impl Cli {
    pub fn prepare(sdk: &str, minimum: u32) -> Result<PathBuf> {
        let xcode = Xcode::new()?;
        let developer =
            if let Some(path) = xcode.selected(minimum, &mut |message| eprintln!("{message}"))? {
                path
            } else {
                eprintln!("Finding the newest compatible Xcode release");
                let release = xcode.latest(sdk, minimum)?;
                let archive = Self::download(&xcode, &release)?;
                eprintln!("Extracting and installing Xcode {} from {}", release.name, archive.display());
                let developer = xcode.install(&release, &archive)?;
                eprintln!("Installed Xcode {} at {}", release.name, developer.display());
                developer
            };
        eprintln!("Using Xcode at {}", developer.display());
        if !xcode.license_accepted(&developer)? {
            if !host::Cli::interactive() {
                return Err(Error::Build(format!(
                    "Xcode's license needs your review.\n\n\
                     1. Run this in an interactive terminal:\n\
                          sudo '{}' -license\n\n\
                     2. Review the license, accept it if you agree, then rerun Piston.",
                    developer.join("usr/bin/xcodebuild").display(),
                )).into());
            }
            eprintln!(
                "\nXcode's license needs your review.\n\n\
                 Review the license below and accept it if you agree.\n\
                 macOS may ask for your administrator password.\n\n\
                 Piston will continue after acceptance.\n"
            );
            xcode.review_license(&developer)?;
        }
        if !xcode.components_ready(&developer)? {
            eprintln!(
                "Installing Xcode components. Log: {}",
                xcode.home.join("Library/Logs/Piston/Xcode/components.log").display()
            );
            if let Err(error) = xcode.install_components(&developer, false) {
                if !host::Cli::interactive() || !xcode.components_need_admin() { return Err(error); }
                eprintln!("Xcode component installation needs administrator approval.");
                xcode.install_components(&developer, true)?;
            }
        }
        xcode.verify_sdk(&developer, sdk, minimum)?;
        Ok(developer)
    }

    fn download(xcode: &Xcode, release: &Release) -> Result<PathBuf> {
        let filename = release.url.rsplit('/').next().unwrap();
        let mut instructions_shown = false;
        loop {
            if let Some(path) = xcode.archive(release, &mut |message| {
                if !instructions_shown { eprintln!("{message}"); }
            })? {
                return Ok(path);
            }
            if !host::Cli::interactive() {
                return Err(Error::Install(format!(
                    "Xcode {} is required.\n\n\
                     1. Download the archive:\n   {}\n\n\
                     2. Set PISTON_XCODE_ARCHIVE=/absolute/path/{filename}\n\
                        and rerun this command.\n\n\
                     Piston will extract the archive and install Xcode.",
                    release.name, host::Cli::link("Download Xcode", &release.url),
                )).into());
            }
            if !instructions_shown {
                eprintln!(
                    "\nXcode {} for {} is required.\n\n\
                     1. Download the archive:\n   {}\n\n\
                     2. Complete Apple sign-in if asked, then save the archive in Downloads.\n",
                    release.name, xcode.architecture,
                    host::Cli::link("Download Xcode", &release.url),
                );
                eprintln!(
                    "Waiting for {filename} in Downloads or Spotlight.\n\
                     Piston will extract it, install Xcode, and continue.\n\n\
                     Saved elsewhere? Move it to Downloads or rerun with PISTON_XCODE_ARCHIVE.\n\
                     Press Ctrl-C to cancel.\n"
                );
                instructions_shown = true;
            }
            thread::sleep(Duration::from_secs(3));
        }
    }
}
