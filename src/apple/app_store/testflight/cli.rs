use std::thread;
use std::time::{Duration, Instant};
use crate::host::cli::HostCli;
use crate::manifest::Manifest;
use crate::{PistonError, Result};
use super::{FlightStatus, TestFlight};

pub struct TestFlightCli;

impl TestFlightCli {
    pub fn wait(flight: &TestFlight, manifest: &Manifest) -> Result<()> {
        let start = Instant::now();
        let mut compliance_opened = false;
        let mut previous = String::new();
        println!(
            "Waiting for TestFlight readiness. Status changes will appear below.\n\
            Ctrl-C stops local monitoring; rerun the same deploy command to resume.\n{}",
            flight.url()
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
                        "Deployment complete: version {} (build {}) is ready for internal TestFlight testing.\n{}\n\
                        Existing groups with automatic distribution receive eligible builds. \
                        Otherwise select an Internal Testing group and add this build in App Store Connect. \
                        External testing requires its own group, beta information and Apple's beta review.",
                        flight.version, flight.build_number, url
                    );
                    return Ok(());
                }
                "MISSING_EXPORT_COMPLIANCE" => {
                    if !compliance_opened {
                        println!(
                            "Action required: open {url}, click \"Provide Export Compliance \
                            Information\", and answer the questions for this build. Piston will \
                            continue automatically when Apple clears the build."
                        );
                        HostCli::open_url(&url);
                        compliance_opened = true;
                    }
                }
                "FAILED" | "INVALID" | "PROCESSING_EXCEPTION" | "EXPIRED" => {
                    flight.record_state(manifest, "failed")?;
                    HostCli::open_url(&url);
                    return Err(PistonError::Generic(format!(
                        "Apple reports {state} for version {} (build {}). Check {url}. \
                         Resolve any reported issue, then rerun cargo piston deploy testflight. \
                         Piston will choose a new build number without changing package.version.",
                        flight.version, flight.build_number,
                    )));
                }
                "NOT_VISIBLE" | "PROCESSING" | "IN_EXPORT_COMPLIANCE_REVIEW" => {}
                _ => {
                    return Err(PistonError::Generic(format!(
                        "Apple returned TestFlight state {state}. Check {url} before retrying.",
                    )));
                }
            }
            thread::sleep(Duration::from_secs(15));
        }
    }
}
