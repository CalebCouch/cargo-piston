use base64::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::Result;

pub mod cli;
pub mod release;
pub use release::AppStoreRelease;
mod artwork;
pub use artwork::IOSArtwork;
pub mod testflight;
pub use testflight::TestFlight;
pub mod setup;

#[derive(Clone, Deserialize, Serialize)]
pub struct AscApiKey {
    key_id: String,
    issuer_id: String,
    priv_key: String,
}

impl AscApiKey {
    pub fn from_hm(env: &HashMap<String, String>) -> Result<Self> {
        let key_id = env
            .get("asc_key_id")
            .ok_or_else(|| crate::cli::Error::MissingSetting { key: "ASC_KEY_ID".into() })?
            .clone();

        let issuer_id = env
            .get("asc_issuer_id")
            .ok_or_else(|| crate::cli::Error::MissingSetting { key: "ASC_ISSUER_ID".into() })?
            .clone();

        let p8_path = env
            .get("asc_key_path")
            .ok_or_else(|| crate::cli::Error::MissingSetting { key: "ASC_KEY_PATH".into() })?;

        let priv_key = fs::read_to_string(p8_path).map_err(|e| {
            cli::Error::StoreKeyUnreadable { path: p8_path.into(), source: e }
        })?;

        Ok(Self { key_id, issuer_id, priv_key })
    }

    pub fn key_id(&self) -> &str { &self.key_id }
    pub fn issuer_id(&self) -> &str { &self.issuer_id }
    pub fn temporary_key(&self) -> Result<tempfile::NamedTempFile> {
        let mut file = tempfile::Builder::new().prefix("piston-asc-").suffix(".p8").tempfile()
            .map_err(cli::Error::StoreTemporaryKey)?;
        file.write_all(self.priv_key.as_bytes()).map_err(cli::Error::StoreTemporaryKey)?;
        Ok(file)
    }

    fn token(&self) -> Result<String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as usize;
        let claims = Claims {
            iss: self.issuer_id.clone(),
            iat: now,
            exp: now + 900,
            aud: "appstoreconnect-v1".into(),
        };
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(self.key_id.clone());
        let key = jsonwebtoken::EncodingKey::from_ec_pem(self.priv_key.as_bytes())
            .map_err(|source| cli::Error::StoreKeyInvalid { key_id: self.key_id.clone(), source })?;
        jsonwebtoken::encode(&header, &claims, &key)
            .map_err(|error| cli::Error::ASCClientJWTEncode(error.to_string()).into())
    }
}

impl std::fmt::Debug for AscApiKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AscApiKey")
            .field("key_id", &self.key_id)
            .field("issuer_id", &self.issuer_id)
            .finish_non_exhaustive()
    }
}

pub struct AppStoreConnect {
    token: String,
    key: AscApiKey,
    http: ureq::Agent,
}

impl AppStoreConnect {
    pub fn new(api_key: &AscApiKey) -> Result<Self> {
        let token = api_key.token()?;
        let http = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(60)).build();
        Ok(Self { token, key: api_key.clone(), http })
    }

    pub fn next_build(&self, app_id: &str, version: &str, previous: Option<&str>) -> Result<String> {
        let requested = Self::version_number(version)?;
        let releases = self.list(
            &format!("/v1/apps/{app_id}/appStoreVersions"),
            &[
                ("filter[platform]", "IOS"),
                ("filter[appVersionState]", "READY_FOR_DISTRIBUTION,REPLACED_WITH_NEW_VERSION"),
                ("fields[appStoreVersions]", "versionString"),
            ],
        )?;
        let mut latest = None;
        for release in releases {
            let number = release["attributes"]["versionString"].as_str().ok_or_else(|| {
                cli::Error::StoreResponseField { field: "appStoreVersions.versionString".into() }
            })?;
            let numeric = Self::version_number(number)?;
            if latest.as_ref().is_none_or(|(value, _)| numeric > *value) {
                latest = Some((numeric, number.to_string()));
            }
        }
        if let Some((numeric, latest)) = latest && requested <= numeric {
            return Err(cli::Error::StoreReleaseExists {
                requested: version.into(), released: latest,
            }.into());
        }
        let mut highest = previous.and_then(|number| Self::version_number(number).ok())
            .unwrap_or([0; 3])[0];
        let builds = self.list(
            "/v1/builds",
            &[
                ("filter[app]", app_id),
                ("filter[preReleaseVersion.platform]", "IOS"),
                ("fields[builds]", "version"),
            ],
        )?;
        for build in builds {
            let number = build["attributes"]["version"].as_str().ok_or_else(|| {
                cli::Error::StoreResponseField { field: "builds.version".into() }
            })?;
            highest = highest.max(Self::version_number(number)?[0]);
        }
        highest.checked_add(1).map(|number| number.to_string())
            .ok_or_else(|| cli::Error::StoreBuildNumberOverflow.into())
    }

    pub fn list(&self, path: &str, query: &[(&str, &str)]) -> Result<Vec<Value>> {
        let mut response = self.request("GET", path, query, None)?;
        let mut entries = Vec::new();
        for _ in 0..100 {
            entries.extend(
                response["data"]
                    .as_array()
                    .ok_or_else(|| cli::Error::StoreResponseCollection { path: path.into() })?
                    .iter()
                    .cloned(),
            );
            let Some(next) = response["links"]["next"].as_str() else { return Ok(entries); };
            let path = next
                .strip_prefix("https://api.appstoreconnect.apple.com")
                .filter(|path| path.starts_with('/'))
                .ok_or_else(|| cli::Error::StorePaginationUrl { url: next.into() })?;
            response = self.request("GET", path, &[], None)?;
        }
        Err(cli::Error::StorePaginationLimit.into())
    }

    pub fn request(&self, method: &str, path: &str, query: &[(&str, &str)], body: Option<Value>) -> Result<Value> {
        for attempt in 0..3 {
            let token = self.key.token()?;
            let mut request = self
                .http
                .request(method, &format!("https://api.appstoreconnect.apple.com{path}"))
                .set("Authorization", &format!("Bearer {token}"));
            for (key, value) in query { request = request.query(key, value); }
            let response = match &body {
                Some(body) => request.send_json(body),
                None => request.call(),
            };
            match response {
                Ok(response) if response.status() == 204 => return Ok(Value::Null),
                Ok(response) => {
                    return response.into_json().map_err(|error| {
                        cli::Error::StoreResponseJson {
                            method: method.into(), path: path.into(), source: error,
                        }.into()
                    });
                }
                Err(ureq::Error::Status(status, response)) => {
                    if method == "GET" && matches!(status, 429 | 502 | 503 | 504) && attempt < 2 {
                        let seconds = response
                            .header("Retry-After")
                            .and_then(|value| value.parse::<u64>().ok())
                            .unwrap_or(2 << attempt)
                            .min(30);
                        thread::sleep(Duration::from_secs(seconds));
                        continue;
                    }
                    let mut contents = String::new();
                    let _ = response.into_reader().take(65536).read_to_string(&mut contents);
                    let response: Value = serde_json::from_str(&contents).unwrap_or(Value::Null);
                    let error = cli::Error::StoreRequestFailed {
                        method: method.into(), path: path.into(), status, response,
                    };
                    return Err(if status == 401 {
                        cli::Error::ASCAuthentication(Box::new(error))
                    } else {
                        error
                    }.into());
                }
                Err(ureq::Error::Transport(_)) if method == "GET" && attempt < 2 => {
                    thread::sleep(Duration::from_secs(2 << attempt));
                }
                Err(ureq::Error::Transport(source)) => {
                    return Err(cli::Error::StoreConnectionFailure {
                        method: method.into(), path: path.into(), source,
                    }.into());
                }
            }
        }
        unreachable!()
    }

    pub fn distribution_certificate(&self) -> Result<Option<String>> {
        let list_resp = ureq::get("https://api.appstoreconnect.apple.com/v1/certificates")
            .set("Authorization", &format!("Bearer {}", self.token))
            .query("filter[certificateType]", "DISTRIBUTION")
            .call()
            .map_err(|e| cli::Error::StoreHttp {
                method: "GET", path: "/v1/certificates".into(), source: e,
            })?;

        let json: serde_json::Value =
            list_resp.into_json().map_err(|e| crate::cli::Error::IntoJSON(e.to_string()))?;

        json["data"].as_array().and_then(|certificates| certificates.first())
            .map(|certificate| -> Result<String> {
                certificate["attributes"]["name"].as_str().map(str::to_owned).ok_or_else(|| {
                    cli::Error::StoreResponseField { field: "certificates.name".into() }.into()
                })
            }).transpose()
    }

    pub fn create_distribution_certificate(&self, csr_content: &str) -> Result<(String, Vec<u8>)> {
        let body = json!({
            "data": {
                "type": "certificates",
                "attributes": { "certificateType": "DISTRIBUTION", "csrContent": csr_content }
            }
        });

        let create_resp = ureq::post("https://api.appstoreconnect.apple.com/v1/certificates")
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| cli::Error::StoreHttp {
                method: "POST", path: "/v1/certificates".into(), source: e,
            })?;

        let json: serde_json::Value =
            create_resp.into_json().map_err(|e| crate::cli::Error::IntoJSON(e.to_string()))?;

        let cert_name = json["data"]["attributes"]["name"].as_str().ok_or_else(|| {
            cli::Error::StoreResponseField { field: "certificates.name".into() }
        })?.to_string();

        let cert_b64 = json["data"]["attributes"]["certificateContent"]
            .as_str()
            .ok_or_else(|| cli::Error::StoreResponseField { field: "certificateContent".into() })?;

        let cert_der = BASE64_STANDARD
            .decode(cert_b64)
            .map_err(|e| cli::Error::Base64Decode(e.to_string()))?;

        Ok((cert_name, cert_der))
    }

    pub fn ios_ad_hoc_profile(&self, device_id: &str, bundle_id: &str, app_name: &str, certificate_id: &str) -> Result<(String, Vec<u8>)> {
        let device_resource_id = {
            let check = ureq::get("https://api.appstoreconnect.apple.com/v1/devices")
                .set("Authorization", &format!("Bearer {}", self.token))
                .query("filter[udid]", device_id)
                .call()
                .map_err(|e| cli::Error::StoreHttp {
                    method: "GET", path: "/v1/devices".into(), source: e,
                })?;

            let json: serde_json::Value =
                check.into_json().map_err(|e| crate::cli::Error::IntoJSON(e.to_string()))?;

            if let Some(existing) = json["data"].as_array().and_then(|a| a.first()) {
                existing["id"].as_str().unwrap().to_string()
            } else {
                let body = json!({
                    "data": {
                        "type": "devices",
                        "attributes": { "name": app_name, "udid": device_id, "platform": "IOS" }
                    }
                });

                let resp = ureq::post("https://api.appstoreconnect.apple.com/v1/devices")
                    .set("Authorization", &format!("Bearer {}", self.token))
                    .set("Content-Type", "application/json")
                    .send_json(&body)
                    .map_err(|e| cli::Error::StoreHttp {
                        method: "POST", path: "/v1/devices".into(), source: e,
                    })?;

                let json: serde_json::Value =
                    resp.into_json().map_err(|e| crate::cli::Error::IntoJSON(e.to_string()))?;

                json["data"]["id"].as_str().unwrap().to_string()
            }
        };
        let bundle_resource_id = {
            let check = ureq::get("https://api.appstoreconnect.apple.com/v1/bundleIds")
                .set("Authorization", &format!("Bearer {}", self.token))
                .query("filter[identifier]", bundle_id)
                .call()
                .map_err(|e| cli::Error::StoreHttp {
                    method: "GET", path: "/v1/bundleIds".into(), source: e,
                })?;

            let json: serde_json::Value =
                check.into_json().map_err(|e| crate::cli::Error::IntoJSON(e.to_string()))?;

            if let Some(existing) = json["data"].as_array().and_then(|a| a.first()) {
                existing["id"].as_str().unwrap().to_string()
            } else {
                let body = json!({
                    "data": {
                        "type": "bundleIds",
                        "attributes": {
                            "identifier": bundle_id,
                            "name": app_name,
                            "platform": "IOS"
                        }
                    }
                });

                let resp = ureq::post("https://api.appstoreconnect.apple.com/v1/bundleIds")
                    .set("Authorization", &format!("Bearer {}", self.token))
                    .set("Content-Type", "application/json")
                    .send_json(&body)
                    .map_err(|e| cli::Error::StoreHttp {
                        method: "POST", path: "/v1/bundleIds".into(), source: e,
                    })?;

                let status = resp.status();

                if (200..300).contains(&status) {
                    let json: serde_json::Value =
                        resp.into_json().map_err(|e| crate::cli::Error::IntoJSON(e.to_string()))?;
                    json["data"]["id"].as_str().unwrap().to_string()
                } else {
                    let error_body = resp.into_string().unwrap_or_default();
                    return Err(cli::Error::StoreHttpStatus {
                        method: "POST", path: "/v1/bundleIds".into(), status, response: error_body,
                    }.into());
                }
            }
        };

        let profile_name = format!(
            "{}-AdHoc-{}",
            app_name,
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
        );
        let profile_id = {
            let check = ureq::get(&format!(
                "https://api.appstoreconnect.apple.com/v1/bundleIds/{}/profiles",
                bundle_resource_id
            ))
            .set("Authorization", &format!("Bearer {}", self.token))
            .call()
            .map_err(|e| cli::Error::StoreHttp {
                method: "GET", path: format!("/v1/bundleIds/{bundle_resource_id}/profiles"), source: e,
            })?;

            let json: serde_json::Value =
                check.into_json().map_err(|e| crate::cli::Error::IntoJSON(e.to_string()))?;

            if let Some(existing) = json["data"].as_array().and_then(|arr| {
                arr.iter()
                    .find(|p| p["attributes"]["profileType"].as_str() == Some("IOS_APP_ADHOC"))
            }) {
                existing["id"].as_str().unwrap().to_string()
            } else {
                let cert_name_trim = certificate_id
                    .replace("Apple Distribution: ", "")
                    .split('(')
                    .next()
                    .unwrap_or(certificate_id)
                    .trim()
                    .to_string();
                let cert_list = ureq::get("https://api.appstoreconnect.apple.com/v1/certificates")
                    .set("Authorization", &format!("Bearer {}", self.token))
                    .query("filter[certificateType]", "DISTRIBUTION")
                    .call()
                    .map_err(|e| cli::Error::StoreHttp {
                        method: "GET", path: "/v1/certificates".into(), source: e,
                    })?;

                let cert_json: serde_json::Value =
                    cert_list.into_json().map_err(|e| crate::cli::Error::IntoJSON(e.to_string()))?;

                let certificate_internal_id = cert_json["data"]
                    .as_array()
                    .and_then(|arr| {
                        arr.iter().find(|cert| {
                            cert["attributes"]["displayName"]
                                .as_str()
                                .is_some_and(|name| name.contains(&cert_name_trim))
                        })
                    })
                    .and_then(|cert| cert["id"].as_str())
                    .ok_or_else(|| cli::Error::StoreCertificateMissing {
                        name: cert_name_trim.clone(),
                    })?
                    .to_string();

                let body = json!({
                    "data": {
                        "type": "profiles",
                        "attributes": { "name": profile_name, "profileType": "IOS_APP_ADHOC" },
                        "relationships": {
                            "bundleId": { "data": { "type": "bundleIds", "id": bundle_resource_id } },
                            "certificates": { "data": [{ "type": "certificates", "id": certificate_internal_id }] },
                            "devices": { "data": [{ "type": "devices", "id": device_resource_id }] }
                        }
                    }
                });

                let create_result = ureq::post("https://api.appstoreconnect.apple.com/v1/profiles")
                    .set("Authorization", &format!("Bearer {}", self.token))
                    .set("Content-Type", "application/json")
                    .send_json(&body);
                let create_resp = match create_result {
                    Ok(resp) => resp,
                    Err(ureq::Error::Status(code, resp)) => {
                        let body = resp.into_string().unwrap_or_default();

                        return Err(cli::Error::StoreHttpStatus {
                            method: "POST", path: "/v1/profiles".into(), status: code, response: body,
                        }.into());
                    }
                    Err(e) => {
                        return Err(cli::Error::StoreHttp {
                            method: "POST", path: "/v1/profiles".into(), source: e,
                        }.into());
                    }
                };

                let json: serde_json::Value = create_resp
                    .into_json()
                    .map_err(|e| crate::cli::Error::IntoJSON(e.to_string()))?;

                json["data"]["id"].as_str().unwrap().to_string()
            }
        };

        let dl_resp =
            ureq::get(&format!("https://api.appstoreconnect.apple.com/v1/profiles/{}", profile_id))
                .set("Authorization", &format!("Bearer {}", self.token))
                .call()
                .map_err(|e| cli::Error::StoreHttp {
                    method: "GET", path: format!("/v1/profiles/{profile_id}"), source: e,
                })?;

        let dl_json: serde_json::Value =
            dl_resp.into_json().map_err(|e| crate::cli::Error::IntoJSON(e.to_string()))?;

        let b64 = dl_json["data"]["attributes"]["profileContent"]
            .as_str()
            .ok_or_else(|| cli::Error::StoreResponseField { field: "profileContent".into() })?;

        let profile_data = BASE64_STANDARD
            .decode(b64)
            .map_err(|e| cli::Error::Base64Decode(e.to_string()))?;

        Ok((profile_name, profile_data))
    }
}

#[derive(Serialize)]
struct Claims {
    iss: String,
    iat: usize,
    exp: usize,
    aud: String,
}

impl AppStoreConnect {
    fn version_number(value: &str) -> Result<[u64; 3]> {
        let mut result = [0; 3];
        for (index, part) in value.split('.').enumerate() {
            let number = part.parse::<u64>().ok().filter(|_| {
                !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()) && index < 3
            }).ok_or_else(|| cli::Error::StoreVersionInvalid { version: value.into() })?;
            result[index] = number;
        }
        Ok(result)
    }
}
