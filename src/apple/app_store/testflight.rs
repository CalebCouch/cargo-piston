use serde_json::Value;
use crate::apple::app_store;
use crate::manifest::Manifest;
use crate::Result;
use super::{AppStoreConnect, AscApiKey};

pub mod cli;
pub use cli::Cli;

pub struct FlightStatus {
    pub state: String,
    pub url: String,
}

pub struct TestFlight {
    pub build_number: String,
    pub uploaded: bool,
    client: AppStoreConnect,
    app_id: String,
    pub version: String,
    pub replaced_build: Option<String>,
}

impl TestFlight {
    pub fn new(key: &AscApiKey, bundle_id: &str, version: &str, manifest: &Manifest, previous: Option<&str>, new_build: bool) -> Result<Self> {
        let client = AppStoreConnect::new(key)?;
        let apps = client.list(
            "/v1/apps",
            &[("filter[bundleId]", bundle_id), ("fields[apps]", "name,bundleId")],
        )?;
        let app = apps.first().ok_or_else(|| app_store::cli::Error::StoreAppMissing {
            bundle_id: bundle_id.into(), testflight: true,
        })?;
        let app_id = app["id"]
            .as_str()
            .ok_or_else(|| {
                app_store::cli::Error::StoreResponseField { field: "apps.id".into() }
            })?
            .to_string();
        let upload = manifest.settings(&["testflight"])?;
        let pending = upload.get("build_number").filter(|number| {
            !new_build
                && AppStoreConnect::version_number(number).is_ok()
                && upload.get("app_id") == Some(&app_id)
                && upload.get("version").is_some_and(|saved| saved == version)
                && upload.get("state").is_some_and(|state| state == "uploaded")
        });
        let build_number = match pending {
            Some(number) => number.clone(),
            None => client.next_build(&app_id, version, previous)?,
        };
        let mut flight = Self {
            client,
            app_id,
            version: version.into(),
            build_number,
            uploaded: pending.is_some(),
            replaced_build: None,
        };
        if flight.uploaded
            && flight.build()?.is_some_and(|build| {
                build["attributes"]["expired"].as_bool() == Some(true)
                    || matches!(
                        build["attributes"]["processingState"].as_str(),
                        Some("FAILED" | "INVALID")
                    )
            })
        {
            flight.replaced_build = Some(flight.build_number.clone());
            flight.build_number = flight.client.next_build(&flight.app_id, version, previous)?;
            flight.uploaded = false;
        }
        Ok(flight)
    }

    pub fn url(&self) -> String {
        format!("https://appstoreconnect.apple.com/apps/{}/testflight/ios", self.app_id)
    }

    pub fn record_upload(&self, manifest: &Manifest) -> Result<()> {
        manifest.save::<&str>(
            &["testflight"],
            &[
                ("app_id", &self.app_id),
                ("version", &self.version),
                ("build_number", &self.build_number),
                ("state", "uploaded"),
            ],
        )
    }

    pub fn status(&self) -> Result<FlightStatus> {
        let mut url = self.url();
        let state = if let Some(build) = self.build()? {
            let id = build["id"].as_str().ok_or_else(|| {
                app_store::cli::Error::StoreResponseField { field: "builds.id".into() }
            })?;
            url = format!("{url}/{id}");
            if build["attributes"]["expired"].as_bool() == Some(true) {
                "EXPIRED".into()
            } else {
                let processing =
                    build["attributes"]["processingState"].as_str().ok_or_else(|| {
                        app_store::cli::Error::StoreResponseField { field: "builds.processingState".into() }
                    })?;
                if processing == "VALID" {
                    let details = self.client.request(
                        "GET",
                        &format!("/v1/builds/{id}/buildBetaDetail"),
                        &[],
                        None,
                    )?;
                    details["data"]["attributes"]["internalBuildState"]
                        .as_str()
                        .ok_or_else(|| {
                            app_store::cli::Error::StoreResponseField {
                                field: "buildBetaDetail.internalBuildState".into(),
                            }
                        })?
                        .to_string()
                } else {
                    processing.into()
                }
            }
        } else {
            "NOT_VISIBLE".into()
        };
        Ok(FlightStatus { state, url })
    }

    pub fn record_state(&self, manifest: &Manifest, state: &str) -> Result<()> {
        manifest.save(&["testflight"], &[("state", state)])
    }

    fn build(&self) -> Result<Option<Value>> {
        let builds = self.client.list(
            "/v1/builds",
            &[
                ("filter[app]", &self.app_id),
                ("filter[preReleaseVersion.platform]", "IOS"),
                ("filter[preReleaseVersion.version]", &self.version),
                ("filter[version]", &self.build_number),
                ("fields[builds]", "version,processingState,expired"),
            ],
        )?;
        Ok(builds.into_iter().next())
    }
}
