use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use crate::host;
use crate::{PistonError, Result};
use super::{Release, Xcode};

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
                return Err(PistonError::XcodeBuildError(format!(
                    "Xcode's license needs your review.\n\n\
                     1. Run this in an interactive terminal:\n\
                          sudo '{}' -license\n\n\
                     2. Review the license, accept it if you agree, then rerun Piston.",
                    developer.join("usr/bin/xcodebuild").display(),
                )));
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
                return Err(PistonError::XcodeInstallError(format!(
                    "Xcode {} is required.\n\n\
                     1. Download the archive:\n   {}\n\n\
                     2. Set PISTON_XCODE_ARCHIVE=/absolute/path/{filename}\n\
                        and rerun this command.\n\n\
                     Piston will extract the archive and install Xcode.",
                    release.name, host::Cli::link("Download Xcode", &release.url),
                )));
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
