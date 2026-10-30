use clap::Parser;
use std::fmt;
use crate::Result;
use crate::apple::app_store::{release, setup, testflight};
use crate::apple::{AppStoreRelease, IOSArtwork, TestFlight};
use crate::ios::{self, distribution, ApplicationArgs, IOSStore};
use crate::manifest::Manifest;
use crate::developer;
use crate::host;
use crate::project::Project;

#[derive(Debug)]
pub enum Error { NewBuildDestination, ScreenshotsDestination }

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NewBuildDestination => "--new-build is only needed for TestFlight deployments.",
            Self::ScreenshotsDestination =>
                "TestFlight does not use App Store screenshots.\n\n\
                 To update the store gallery, run:\n\
                   cargo piston deploy app-store --screenshots-only",
        })
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, Parser)]
pub struct DeploymentArgs {
    /// Deployment destination. Omitting it preserves the App Store workflow.
    #[arg(value_enum, default_value_t = Destination::AppStore)]
    destination: Destination,
    #[command(flatten)]
    application: ApplicationArgs,
    /// Prepare deployment setup without building or uploading (App Store also prepares screenshots).
    #[arg(long, conflicts_with = "screenshots_only")]
    prepare_only: bool,
    /// Upload screenshots for the Cargo version without building or uploading a binary.
    #[arg(long)]
    screenshots_only: bool,
    /// Import or replace App Store Connect and enabled APNs credentials in AIR.
    #[arg(long)]
    setup: bool,
    /// Start a new TestFlight build instead of resuming a pending upload.
    #[arg(long, conflicts_with = "screenshots_only")]
    new_build: bool,
}

pub struct Cli;

impl Cli {
    pub fn start(mut args: DeploymentArgs) -> Result<()> {
        let manifest = Project::manifest(&args.application.cargo)?;
        let (_, team) = developer::Cli::credentials(&manifest, args.application.team.as_deref(), false)?;
        args.application.team = Some(team);
        if args.destination == Destination::Testflight { return Self::testflight(args); }
        if args.new_build {
            return Err(Error::NewBuildDestination.into());
        }
        let mut application = ios::Cli::application(args.application, true)?;
        let root = application.package().manifest_path.parent().unwrap().as_std_path();
        let artwork = IOSArtwork::new(root)?;
        let identifier = application.store_bundle_id()?.to_owned();
        let (key, team) = setup::Cli::from_project(root, application.team(), args.setup)?;
        application.remember_team(&team)?;
        if !args.screenshots_only {
            ios::Cli::notifications(&mut application, &team, false, args.setup)?;
        }
        let version = &application.package().version;
        let version = format!("{}.{}.{}", version.major, version.minor, version.patch);
        println!("Checking App Store Connect for {identifier} {version}");
        let mut release = AppStoreRelease::new(&key, &identifier, &version)?;
        let build_number = if args.screenshots_only {
            None
        } else {
            Some(release.next_build(application.build_number())?)
        };
        let output = application.work().join("store/screenshots");
        let screenshots = artwork.prepare(&output, &release.sizes())?;
        release::Cli::validate(&release, &screenshots)?;
        println!("Prepared {} screenshots in {}", screenshots.len(), output.display());
        if args.prepare_only {
            println!("Screenshot and credential checks passed. No app was built or uploaded.");
            if let Some(number) = build_number {
                println!("Next upload: version {version}, build {number}.");
            }
            return Ok(());
        }
        if args.screenshots_only {
            release::Cli::upload(&mut release, &screenshots)?;
        } else {
            let developer = ios::Cli::prepare(&application, true)?;
            let mut store = IOSStore::new(application, &key, developer, build_number.unwrap())?;
            let ipa = distribution::Cli::build(&mut store)?;
            println!("Prepared App Store build: {}", ipa.display());
            release::Cli::upload(&mut release, &screenshots)?;
            distribution::Cli::upload(&store)?;
        }
        println!(
            "Uploaded to App Store Connect.\n\n\
             To submit for review:\n\
             1. Open your app:\n   {}\n\n\
             2. Complete the listing, select the build, and submit for review.",
            host::Cli::link("App Store Connect", "https://appstoreconnect.apple.com/apps"),
        );
        Ok(())
    }

    fn testflight(args: DeploymentArgs) -> Result<()> {
        if args.screenshots_only {
            return Err(Error::ScreenshotsDestination.into());
        }
        let mut application = ios::Cli::application(args.application, true)?;
        application.store_release()?;
        let root = application.package().manifest_path.parent().unwrap().as_std_path();
        let (key, team) = setup::Cli::from_project(root, application.team(), args.setup)?;
        application.remember_team(&team)?;
        ios::Cli::notifications(&mut application, &team, false, args.setup)?;
        let version = &application.package().version;
        let version = format!("{}.{}.{}", version.major, version.minor, version.patch);
        let identifier = application.store_bundle_id()?;
        println!("Checking TestFlight for {identifier} {version}");
        let manifest = Manifest::new(application.package().manifest_path.as_std_path())?;
        let flight = TestFlight::new(
            &key,
            identifier,
            &version,
            &manifest,
            application.build_number(),
            args.new_build,
        )?;
        if let Some(number) = &flight.replaced_build {
            println!(
                "Previous TestFlight build {number} can no longer be tested; preparing a new build."
            );
        }
        if args.prepare_only {
            ios::Cli::prepare(&application, true)?;
            println!("TestFlight credential and toolchain checks passed. No build was uploaded.");
            if flight.uploaded {
                println!("Will resume version {version}, build {}.", flight.build_number);
            } else {
                println!("Next upload: version {version}, build {}.", flight.build_number);
            }
            println!("\n{}", host::Cli::link("TestFlight", &flight.url()));
            return Ok(());
        }
        if flight.uploaded {
            println!(
                "Resuming version {version}, build {}.\n\
                 To upload code changes, rerun with --new-build.\n",
                flight.build_number
            );
        } else {
            println!("Preparing version {version}, build {}.", flight.build_number);
            let developer = ios::Cli::prepare(&application, true)?;
            let mut store =
                IOSStore::new(application, &key, developer, flight.build_number.clone())?;
            let ipa = distribution::Cli::build(&mut store)?;
            println!("Prepared TestFlight build: {}", ipa.display());
            distribution::Cli::upload(&store)?;
            flight.record_upload(&manifest)?;
        }
        testflight::Cli::wait(&flight, &manifest)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
enum Destination { #[default] AppStore, Testflight, }
