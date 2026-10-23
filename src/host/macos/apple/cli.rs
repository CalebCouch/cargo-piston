use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use crate::host::cli::HostCli;
use crate::{PistonError, Result};
use super::{Release, Xcode};

pub struct XcodeCli;

impl XcodeCli {
    pub fn prepare(sdk: &str, minimum: u32) -> Result<PathBuf> {
        let xcode = Xcode::new()?;
        let developer =
            if let Some(path) = xcode.selected(minimum, &mut |message| eprintln!("{message}"))? {
                path
            } else {
                eprintln!("Finding the newest compatible Xcode release");
                let release = xcode.latest(sdk, minimum)?;
                let archive = Self::download(&xcode, &release)?;
                xcode.install(&release, &archive, &mut |message| eprintln!("{message}"))?
            };
        eprintln!("Using Xcode at {}", developer.display());
        if !xcode.license_accepted(&developer)? {
            if !HostCli::interactive() {
                return Err(PistonError::XcodeBuildError(format!(
                    "Review and accept Apple's license interactively with sudo '{}' -license, then rerun. \
                     Piston does not accept license agreements for you.",
                    developer.join("usr/bin/xcodebuild").display(),
                )));
            }
            eprintln!(
                "Apple requires license acceptance. Review the license below; macOS may ask for \
                your administrator password. Piston continues after you accept."
            );
            xcode.review_license(&developer)?;
        }
        if !xcode.components_ready(&developer)? {
            eprintln!(
                "Installing Xcode components. Log: {}",
                xcode.home.join("Library/Logs/Piston/Xcode/components.log").display()
            );
            if let Err(error) = xcode.install_components(&developer, false) {
                if !HostCli::interactive() || !xcode.components_need_admin() { return Err(error); }
                eprintln!("Xcode component installation needs administrator approval.");
                xcode.install_components(&developer, true)?;
            }
        }
        xcode.verify_sdk(&developer, sdk, minimum)?;
        Ok(developer)
    }

    fn download(xcode: &Xcode, release: &Release) -> Result<PathBuf> {
        let filename = release.url.rsplit('/').next().unwrap();
        let mut opened = false;
        loop {
            if let Some(path) = xcode.archive(release, &mut |message| {
                if !opened { eprintln!("{message}"); }
            })? {
                return Ok(path);
            }
            if !HostCli::interactive() {
                return Err(PistonError::XcodeInstallError(format!(
                    "Xcode {} is required. Download {} and rerun with \
                     PISTON_XCODE_ARCHIVE=/absolute/path/{filename} to extract and install it. \
                     Interactive terminals open the download and continue automatically.",
                    release.name, release.url,
                )));
            }
            if !opened {
                eprintln!(
                    "Downloading Xcode {} for {}: {}",
                    release.name, xcode.architecture, release.url
                );
                eprintln!(
                    "Complete Apple sign-in if asked. Waiting for {filename} in Downloads or Spotlight. \
                    If you save elsewhere, move it to Downloads or rerun with PISTON_XCODE_ARCHIVE. \
                    Press Ctrl-C to cancel."
                );
                HostCli::open_url(&release.url);
                opened = true;
            }
            thread::sleep(Duration::from_secs(3));
        }
    }
}
