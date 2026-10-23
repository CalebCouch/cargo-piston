use clap::Parser;
use crate::apple::app_store::release::cli::ReleaseCli;
use crate::apple::app_store::setup::cli::AccountCli;
use crate::apple::app_store::testflight::cli::TestFlightCli;
use crate::apple::{AppStoreRelease, IOSArtwork, TestFlight};
use crate::ios::cli::IOSCli;
use crate::ios::distribution::cli::DistributionCli;
use crate::ios::{ApplicationArgs, IOSApplication, IOSStore};
use crate::manifest::Manifest;
use crate::{PistonError, Result};

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
    /// Save this project's App Store Connect account setup in Cargo.toml before deployment.
    #[arg(long)]
    setup: bool,
    /// Start a new TestFlight build instead of resuming a pending upload.
    #[arg(long, conflicts_with = "screenshots_only")]
    new_build: bool,
}

pub struct DeploymentCli;

impl DeploymentCli {
    pub fn start(args: DeploymentArgs) -> Result<()> {
        if args.destination == Destination::Testflight { return Self::testflight(args); }
        if args.new_build {
            return Err(PistonError::Generic(
                "--new-build is only needed for TestFlight deployments.".into(),
            ));
        }
        let application = IOSApplication::new(args.application)?;
        let root = application.package().manifest_path.parent().unwrap().as_std_path();
        let artwork = IOSArtwork::new(root)?;
        let identifier = application.store_bundle_id()?;
        let key = AccountCli::from_project(root, args.setup)?;
        let version = &application.package().version;
        let version = format!("{}.{}.{}", version.major, version.minor, version.patch);
        println!("Checking App Store Connect for {identifier} {version}");
        let mut release = AppStoreRelease::new(&key, identifier, &version)?;
        let build_number = if args.screenshots_only {
            None
        } else {
            Some(release.next_build(application.build_number())?)
        };
        let output = application.work().join("store/screenshots");
        let screenshots = artwork.prepare(&output, &release.sizes())?;
        ReleaseCli::validate(&release, &screenshots)?;
        println!("Prepared {} screenshots in {}", screenshots.len(), output.display());
        if args.prepare_only {
            println!("Screenshot and account checks passed. No app was built or uploaded.");
            if let Some(number) = build_number {
                println!("Next upload: version {version}, build {number}.");
            }
            return Ok(());
        }
        if args.screenshots_only {
            ReleaseCli::upload(&mut release, &screenshots)?;
        } else {
            let developer = IOSCli::prepare(&application, true)?;
            let mut store = IOSStore::new(application, &key, developer, build_number.unwrap())?;
            let ipa = DistributionCli::build(&mut store)?;
            println!("Prepared App Store build: {}", ipa.display());
            ReleaseCli::upload(&mut release, &screenshots)?;
            DistributionCli::upload(&store)?;
        }
        println!(
            "Uploaded to App Store Connect. Review submission is separate; open \
            https://appstoreconnect.apple.com/apps to complete the listing and select the build."
        );
        Ok(())
    }

    fn testflight(args: DeploymentArgs) -> Result<()> {
        if args.screenshots_only {
            return Err(PistonError::Generic(
                "TestFlight does not need App Store screenshots. Use cargo piston deploy app-store \
                 --screenshots-only to update the store gallery.".into()
            ));
        }
        let mut application = IOSApplication::new(args.application)?;
        application.store_release()?;
        let root = application.package().manifest_path.parent().unwrap().as_std_path();
        let key = AccountCli::from_project(root, args.setup)?;
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
            IOSCli::prepare(&application, true)?;
            println!("TestFlight account and toolchain checks passed. No build was uploaded.");
            if flight.uploaded {
                println!("Will resume version {version}, build {}.", flight.build_number);
            } else {
                println!("Next upload: version {version}, build {}.", flight.build_number);
            }
            println!("{}", flight.url());
            return Ok(());
        }
        if flight.uploaded {
            println!(
                "Resuming version {version}, build {}. Use --new-build to upload code changes instead.",
                flight.build_number
            );
        } else {
            println!("Preparing version {version}, build {}.", flight.build_number);
            let developer = IOSCli::prepare(&application, true)?;
            let mut store =
                IOSStore::new(application, &key, developer, flight.build_number.clone())?;
            let ipa = DistributionCli::build(&mut store)?;
            println!("Prepared TestFlight build: {}", ipa.display());
            DistributionCli::upload(&store)?;
            flight.record_upload(&manifest)?;
        }
        TestFlightCli::wait(&flight, &manifest)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
enum Destination { #[default] AppStore, Testflight, }
