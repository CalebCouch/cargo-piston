use std::thread;
use std::time::{Duration, Instant};
use crate::host;
use crate::manifest::Manifest;
use crate::{PistonError, Result};
use super::{FlightStatus, TestFlight};

pub struct Cli;

impl Cli {
    pub fn wait(flight: &TestFlight, manifest: &Manifest) -> Result<()> {
        let start = Instant::now();
        let mut compliance_shown = false;
        let mut previous = String::new();
        println!(
            "Waiting for TestFlight. Status changes will appear below.\n\n{}\n\n\
             Ctrl-C stops monitoring. Rerun this deploy command to resume.\n",
            host::Cli::link("TestFlight", &flight.url()),
        );
        loop {
            let FlightStatus { state, url } = flight.status()?;
            if state != previous {
                let description = match state.as_str() {
                    "NOT_VISIBLE" => "Upload accepted; waiting for Apple to list the build",
                    "PROCESSING" => "Apple is processing the build",
                    "MISSING_EXPORT_COMPLIANCE" => {
                        "Waiting for your export-compliance answers in App Store Connect"
                    }
                    "IN_EXPORT_COMPLIANCE_REVIEW" => "Apple is reviewing export compliance",
                    "READY_FOR_BETA_TESTING" | "IN_BETA_TESTING" => {
                        "Ready for internal TestFlight testing"
                    }
                    _ => state.as_str(),
                };
                let elapsed = start.elapsed().as_secs();
                println!(
                    "TestFlight {} ({}) [{:02}:{:02}]: {description}",
                    flight.version,
                    flight.build_number,
                    elapsed / 60,
                    elapsed % 60
                );
                previous = state.clone();
            }
            match state.as_str() {
                "READY_FOR_BETA_TESTING" | "IN_BETA_TESTING" => {
                    flight.record_state(manifest, "complete")?;
                    println!(
                        "Deployment complete: version {} (build {}) is ready for internal testing.\n\n\
                         {}\n\n\
                         Internal testing: automatic groups receive eligible builds.\n\
                         Otherwise add this build to an Internal Testing group.\n\n\
                         External testing: choose a group, complete beta information,\n\
                         and submit the build for Apple's beta review.",
                        flight.version, flight.build_number, host::Cli::link("TestFlight build", &url),
                    );
                    return Ok(());
                }
                "MISSING_EXPORT_COMPLIANCE" => {
                    if !compliance_shown {
                        println!(
                            "\nExport compliance needs your answers.\n\n\
                             1. Open this build:\n   {}\n\n\
                             2. Select \"Provide Export Compliance Information\".\n\
                                Answer the questions for this build.\n\n\
                             Piston will continue when Apple clears the build.\n",
                            host::Cli::link("TestFlight build", &url),
                        );
                        compliance_shown = true;
                    }
                }
                "FAILED" | "INVALID" | "PROCESSING_EXCEPTION" | "EXPIRED" => {
                    flight.record_state(manifest, "failed")?;
                    return Err(PistonError::Generic(format!(
                        "Apple reports {state} for version {} (build {}).\n\n\
                         1. Check Apple's build details and resolve any reported issue:\n   {}\n\n\
                         2. Rerun cargo piston deploy testflight.\n\n\
                         Piston selects a new build number; package.version stays unchanged.",
                        flight.version, flight.build_number, host::Cli::link("TestFlight build", &url),
                    )));
                }
                "NOT_VISIBLE" | "PROCESSING" | "IN_EXPORT_COMPLIANCE_REVIEW" => {}
                _ => {
                    return Err(PistonError::Generic(format!(
                        "Apple returned TestFlight state {state}.\n\n\
                         Check the build before retrying:\n{}",
                        host::Cli::link("TestFlight build", &url),
                    )));
                }
            }
            thread::sleep(Duration::from_secs(15));
        }
    }
}
