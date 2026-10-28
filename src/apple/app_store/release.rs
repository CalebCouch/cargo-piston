use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};
use crate::apple::app_store;
use crate::Result;
use super::artwork::{ScreenshotFamily, StoreScreenshot};
use super::{AppStoreConnect, AscApiKey};

pub mod cli;
pub use cli::Cli;

pub struct AppStoreRelease {
    client: AppStoreConnect,
    app_id: String,
    library_id: String,
    locale: String,
    version: String,
    version_id: Option<String>,
    localization_id: Option<String>,
    groups: Vec<ScreenshotGroup>,
}

impl AppStoreRelease {
    pub fn new(key: &AscApiKey, bundle_id: &str, version: &str) -> Result<Self> {
        let mut release = Self {
            client: AppStoreConnect::new(key)?,
            app_id: String::new(),
            library_id: String::new(),
            locale: String::new(),
            version: version.into(),
            version_id: None,
            localization_id: None,
            groups: Vec::new(),
        };
        let apps = release.client.list(
            "/v1/apps",
            &[("filter[bundleId]", bundle_id), ("fields[apps]", "name,bundleId,primaryLocale")],
        )?;
        let app = apps.first().ok_or_else(|| app_store::cli::Error::StoreAppMissing {
            bundle_id: bundle_id.into(), testflight: false,
        })?;
        release.app_id = Self::string(app, "id")?;
        release.locale = Self::string(&app["attributes"], "primaryLocale")?;
        let library = release.client.request(
            "GET",
            &format!("/v1/apps/{}/assetLibrary", release.app_id),
            &[],
            None,
        )?;
        release.library_id = Self::string(&library["data"], "id")?;
        let versions = release.client.list(
            &format!("/v1/apps/{}/appStoreVersions", release.app_id),
            &[
                ("filter[platform]", "IOS"),
                ("filter[versionString]", version),
                ("fields[appStoreVersions]", "versionString,platform,appVersionState"),
            ],
        )?;
        if let Some(version) = versions.first() {
            Self::check_editable(version)?;
            release.version_id = Some(Self::string(version, "id")?);
            release.localization_id = release.find_localization()?;
        }
        let catalog = release
            .client
            .list("/v1/appAssetLibraryRefData", &[("filter[features]", "APP_STORE_VERSIONS")])?;
        let attributes = catalog.first().map(|entry| &entry["attributes"]).ok_or_else(|| {
            cli::Error::CatalogEmpty
        })?;
        release.groups = ScreenshotGroup::read(attributes)?;
        Ok(release)
    }

    pub fn sizes(&self) -> Vec<(ScreenshotFamily, u32, u32)> {
        let mut sizes = Vec::new();
        for group in &self.groups {
            for spec in &group.specs {
                let size = (group.family, spec.width, spec.height);
                if !sizes.contains(&size) { sizes.push(size); }
            }
        }
        sizes
    }

    pub fn next_build(&self, previous: Option<&str>) -> Result<String> {
        self.client.next_build(&self.app_id, &self.version, previous)
    }

    pub fn validate(&self, screenshots: &[StoreScreenshot]) -> Result<Vec<String>> {
        let mut preserved_groups = Vec::new();
        if screenshots.is_empty() {
            return Err(cli::Error::ScreenshotsMissing.into());
        }
        let existing = if let Some(localization) = &self.localization_id {
            self.client.list(
                &format!("/v1/appStoreVersionLocalizations/{localization}/placements"),
                &[("filter[placementType]", "APP_SCREENSHOT")],
            )?
        } else {
            Vec::new()
        };
        for family in [ScreenshotFamily::Iphone, ScreenshotFamily::Ipad] {
            let images: Vec<_> =
                screenshots.iter().filter(|image| image.family() == family).collect();
            if images.is_empty() { continue; }
            let group = self.group(family, &images)?;
            let mut preserved = Vec::new();
            for placement in &existing {
                let Some(id) = placement["attributes"]["placementGroup"].as_str() else {
                    continue;
                };
                if id != group.id
                    && !preserved.contains(&id)
                    && self.groups.iter().any(|other| other.id == id && other.family == family)
                { preserved.push(id); }
            }
            preserved_groups.extend(preserved.into_iter().map(str::to_owned));
            if images.len() > group.max_count {
                return Err(cli::Error::ScreenshotGroupCount {
                    group: group.id.clone(), count: images.len(), maximum: group.max_count,
                }.into());
            }
            for image in images {
                let size = fs::metadata(image.path())
                    .map_err(|error| {
                        crate::cli::Error::ReadFile { path: image.path().into(), source: error }
                    })?
                    .len();
                let spec = group.specs.iter().find(|spec| spec.accepts(image)).unwrap();
                if size == 0 || size > spec.max_file_size {
                    return Err(cli::Error::ScreenshotFileSize {
                        path: image.source().into(), size, maximum: spec.max_file_size,
                    }.into());
                }
            }
        }
        Ok(preserved_groups)
    }

    fn commit(&self, staged: &[(String, Vec<String>)]) -> Result<()> {
        let mut originals = Vec::new();
        for (group, _) in staged { originals.push((group.clone(), self.placements(group)?)); }
        for (index, (group, images)) in staged.iter().enumerate() {
            if let Err(error) =
                self.sync_group(group, images, &originals[index].1, &originals[index].1)
            {
                let mut recovery = Vec::new();
                for (group, original) in originals.iter().take(index + 1).rev() {
                    let images: Vec<_> =
                        original.iter().filter_map(|placement| placement.image.clone()).collect();
                    let restored = self
                        .placements(group)
                        .and_then(|current| self.sync_group(group, &images, &current, original));
                    if let Err(error) = restored { recovery.push((group.clone(), error)); }
                }
                return Err(cli::Error::ScreenshotSync {
                    source: error, recovery,
                }.into());
            }
        }
        Ok(())
    }

    fn group(&self, family: ScreenshotFamily, images: &[&StoreScreenshot]) -> Result<&ScreenshotGroup> {
        self.groups
            .iter()
            .filter(|group| group.family == family)
            .filter(|group| {
                images.iter().all(|image| group.specs.iter().any(|spec| spec.accepts(image)))
            })
            .max_by(|left, right| left.display_size.total_cmp(&right.display_size))
            .ok_or_else(|| cli::Error::ScreenshotGroupMismatch {
                images: images.iter().map(|image| {
                    (image.source().into(), image.width(), image.height())
                }).collect(),
            }.into())
    }

    fn check_editable(version: &Value) -> Result<()> {
        let state = Self::string(&version["attributes"], "appVersionState")?;
        if matches!(
            state.as_str(),
            "PREPARE_FOR_SUBMISSION"
                | "INVALID_BINARY"
                | "REJECTED"
                | "METADATA_REJECTED"
                | "DEVELOPER_REJECTED"
        ) { return Ok(()); }
        Err(cli::Error::VersionNotEditable {
            version: version["attributes"]["versionString"].as_str().unwrap_or("").into(), state,
        }.into())
    }

    fn find_localization(&self) -> Result<Option<String>> {
        let Some(version) = &self.version_id else { return Ok(None); };
        let localizations = self.client.list(
            &format!("/v1/appStoreVersions/{version}/appStoreVersionLocalizations"),
            &[("filter[locale]", &self.locale)],
        )?;
        localizations.first().map(|locale| Self::string(locale, "id")).transpose()
    }

    fn prepare_localization(&mut self) -> Result<()> {
        if let Some(id) = &self.version_id {
            let version =
                self.client.request("GET", &format!("/v1/appStoreVersions/{id}"), &[], None)?;
            Self::check_editable(&version["data"])?;
        } else {
            let version = self.client.request(
                "POST",
                "/v1/appStoreVersions",
                &[],
                Some(json!({"data": {
                    "type": "appStoreVersions", "attributes": {
                        "platform": "IOS", "versionString": self.version, "releaseType": "MANUAL",
                    }, "relationships": {"app": {"data": {"type": "apps", "id": self.app_id}}},
                }})),
            )?;
            self.version_id = Some(Self::string(&version["data"], "id")?);
            self.localization_id = self.find_localization()?;
        }
        if self.localization_id.is_none() {
            let localization = self.client.request(
                "POST",
                "/v1/appStoreVersionLocalizations",
                &[],
                Some(json!({"data": {
                    "type": "appStoreVersionLocalizations", "attributes": {"locale": self.locale},
                    "relationships": {"appStoreVersion": {"data": {
                        "type": "appStoreVersions", "id": self.version_id.as_ref().unwrap(),
                    }}},
                }})),
            )?;
            self.localization_id = Some(Self::string(&localization["data"], "id")?);
        }
        Ok(())
    }

    fn upload_image(&self, image: &StoreScreenshot, group: &ScreenshotGroup) -> Result<String> {
        let bytes = fs::read(image.path()).map_err(|error| {
            crate::cli::Error::ReadFile { path: image.path().into(), source: error }
        })?;
        let reference = format!("piston-{:x}", Sha256::digest(&bytes));
        let existing = self.client.list(
            &format!("/v1/appAssetLibraries/{}/images", self.library_id),
            &[
                ("filter[referenceName]", &reference),
                ("filter[category]", "APP_SCREENSHOTS_AND_PREVIEWS"),
            ],
        )?;
        let matching: Vec<_> = existing
            .iter()
            .filter(|asset| {
                asset["attributes"]["referenceName"] == reference
                    && asset["attributes"]["fileSize"].as_u64() == Some(bytes.len() as u64)
                    && asset["attributes"]["category"] == "APP_SCREENSHOTS_AND_PREVIEWS"
            })
            .collect();
        for asset in &matching {
            if let Some("PREPARE_FOR_SUBMISSION" | "APPROVED" | "UPLOAD_COMPLETE") =
                asset["attributes"]["state"].as_str() {
                return self.wait_image(&Self::string(asset, "id")?, group);
            }
        }
        let pending =
            matching.iter().find(|asset| asset["attributes"]["state"] == "AWAITING_UPLOAD");
        let mut asset = if let Some(asset) = pending {
            self.client.request(
                "GET",
                &format!("/v1/appAssetLibraryImages/{}", Self::string(asset, "id")?),
                &[],
                None,
            )?
        } else {
            self.reserve_image(image, bytes.len(), &reference)?
        };
        for attempt in 0..2 {
            let id = Self::string(&asset["data"], "id")?;
            if self.transfer(&asset["data"]["attributes"]["uploadOperations"], &bytes, &id)? {
                self.client.request(
                    "PATCH",
                    &format!("/v1/appAssetLibraryImages/{id}"),
                    &[],
                    Some(json!({"data": {
                        "type": "appAssetLibraryImages", "id": id, "attributes": {"uploaded": true},
                    }})),
                )?;
                return self.wait_image(&id, group);
            }
            if attempt == 0 {
                self.client.request(
                    "DELETE",
                    &format!("/v1/appAssetLibraryImages/{id}"),
                    &[],
                    None,
                )?;
                asset = self.reserve_image(image, bytes.len(), &reference)?;
            }
        }
        Err(cli::Error::UploadUrlsRejected.into())
    }

    fn reserve_image(&self, image: &StoreScreenshot, size: usize, reference: &str) -> Result<Value> {
        self.client.request("POST", "/v1/appAssetLibraryImages", &[], Some(json!({"data": {
            "type": "appAssetLibraryImages", "attributes": {
                "fileName": image.path().file_name().unwrap().to_string_lossy(),
                "fileSize": size, "category": "APP_SCREENSHOTS_AND_PREVIEWS", "referenceName": reference,
            }, "relationships": {"assetLibrary": {"data": {"type": "appAssetLibraries", "id": self.library_id}}},
        }})))
    }

    fn transfer(&self, operations: &Value, bytes: &[u8], image_id: &str) -> Result<bool> {
        let operations = operations.as_array().ok_or_else(|| {
            cli::Error::UploadOperationsMissing { image_id: image_id.into() }
        })?;
        let mut parts = Vec::new();
        for operation in operations {
            let offset = operation["offset"].as_u64();
            let length = operation["length"].as_u64();
            let Some((offset, length)) = offset.zip(length) else {
                return Err(cli::Error::UploadRangeInvalid.into());
            };
            let end = offset
                .checked_add(length)
                .filter(|end| *end <= bytes.len() as u64)
                .ok_or_else(|| {
                    cli::Error::UploadRangeTooLarge
                })?;
            parts.push((offset as usize, end as usize, operation));
        }
        parts.sort_by_key(|part| part.0);
        let mut expected = 0;
        for (start, end, _) in &parts {
            if *start != expected || end <= start {
                return Err(cli::Error::UploadRangeDiscontinuous.into());
            }
            expected = *end;
        }
        if expected != bytes.len() {
            return Err(cli::Error::UploadRangeIncomplete.into());
        }
        for (start, end, operation) in parts {
            let method = Self::string(operation, "method")?;
            let url = Self::string(operation, "url")?;
            if !url.starts_with("https://") {
                return Err(cli::Error::UploadInsecureUrl.into());
            }
            let mut request = self.client.http.request(&method, &url);
            for header in operation["requestHeaders"].as_array().into_iter().flatten() {
                let name = Self::string(header, "name")?;
                if name.eq_ignore_ascii_case("authorization") {
                    return Err(cli::Error::UploadAuthorization.into());
                }
                request = request.set(&name, &Self::string(header, "value")?);
            }
            match request.send_bytes(&bytes[start..end]) {
                Ok(_) => {}
                Err(ureq::Error::Status(403, _)) => return Ok(false),
                Err(error) => {
                    let status = match error {
                        ureq::Error::Status(status, _) => Some(status),
                        ureq::Error::Transport(_) => None,
                    };
                    return Err(cli::Error::ScreenshotUpload {
                        image_id: image_id.into(), status,
                    }.into());
                }
            }
        }
        Ok(true)
    }

    fn wait_image(&self, id: &str, group: &ScreenshotGroup) -> Result<String> {
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            let image = self.client.request(
                "GET",
                &format!("/v1/appAssetLibraryImages/{id}"),
                &[],
                None,
            )?;
            let attributes = &image["data"]["attributes"];
            let state = Self::string(attributes, "state")?;
            match state.as_str() {
                "PREPARE_FOR_SUBMISSION" | "APPROVED" => {
                    let spec = Self::string(attributes, "specId")?;
                    if !group.specs.iter().any(|candidate| candidate.id == spec) {
                        return Err(cli::Error::ImageSpecificationMismatch {
                            image_id: id.into(), group: group.id.clone(), spec,
                        }.into());
                    }
                    return Ok(id.into());
                }
                "UPLOAD_COMPLETE" if Instant::now() < deadline => {
                    thread::sleep(Duration::from_secs(2))
                }
                "UPLOAD_COMPLETE" => {
                    return Err(cli::Error::ImageProcessingTimeout { image_id: id.into() }.into());
                }
                _ => {
                    return Err(cli::Error::ImageRejected {
                        image_id: id.into(), state, details: attributes["stateDetails"].clone(),
                    }.into());
                }
            }
        }
    }

    fn placements(&self, group: &str) -> Result<Vec<Placement>> {
        let entries = self.client.list(
            &format!(
                "/v1/appStoreVersionLocalizations/{}/placements",
                self.localization_id.as_ref().unwrap()
            ),
            &[
                ("filter[placementGroup]", group),
                ("sort", "placementGroupPosition"),
                ("include", "image"),
            ],
        )?;
        entries
            .iter()
            .map(|entry| {
                Ok(Placement {
                    id: Self::string(entry, "id")?,
                    image: if entry["attributes"]["placementType"] == "APP_SCREENSHOT" {
                        Some(Self::string(&entry["relationships"]["image"]["data"], "id")?)
                    } else {
                        None
                    },
                })
            })
            .collect()
    }

    fn sync_group(&self, group: &str, images: &[String], current: &[Placement], layout: &[Placement]) -> Result<()> {
        let mut available: Vec<_> =
            current.iter().filter(|placement| placement.image.is_some()).collect();
        let mut retained = Vec::new();
        for image in images {
            let index =
                available.iter().position(|placement| placement.image.as_ref() == Some(image));
            retained.push(index.map(|index| available.remove(index).id.clone()));
        }
        for placement in available {
            self.client.request(
                "DELETE",
                &format!("/v1/appAssetLibraryPlacements/{}", placement.id),
                &[],
                None,
            )?;
        }
        let mut screenshots = Vec::new();
        for (image, placement) in images.iter().zip(retained) {
            let id = if let Some(id) = placement {
                id
            } else {
                let placement = self.client.request("POST", "/v1/appAssetLibraryPlacements", &[], Some(json!({"data": {
                    "type": "appAssetLibraryPlacements", "attributes": {
                        "placementType": "APP_SCREENSHOT", "placementGroup": group,
                    }, "relationships": {
                        "image": {"data": {"type": "appAssetLibraryImages", "id": image}},
                        "appStoreVersionLocalization": {"data": {
                            "type": "appStoreVersionLocalizations", "id": self.localization_id.as_ref().unwrap(),
                        }},
                    },
                }})))?;
                Self::string(&placement["data"], "id")?
            };
            screenshots.push(id);
        }
        let mut screenshots = screenshots.into_iter();
        let mut ordered = Vec::new();
        for placement in layout {
            if placement.image.is_some() {
                if let Some(id) = screenshots.next() { ordered.push(id); }
            } else {
                ordered.push(placement.id.clone());
            }
        }
        ordered.extend(screenshots);
        let current_ids: Vec<_> = current.iter().map(|placement| placement.id.clone()).collect();
        if !ordered.is_empty() && current_ids != ordered {
            let ids: Vec<_> = ordered
                .iter()
                .map(|id| json!({"type": "appAssetLibraryPlacements", "id": id}))
                .collect();
            self.client.request("POST", "/v1/appAssetLibraryPlacementOrderingRequests", &[], Some(json!({"data": {
                "type": "appAssetLibraryPlacementOrderingRequests", "attributes": {"placementGroup": group},
                "relationships": {
                    "orderedPlacements": {"data": ids}, "appStoreVersionLocalization": {"data": {
                        "type": "appStoreVersionLocalizations", "id": self.localization_id.as_ref().unwrap(),
                    }},
                },
            }})))?;
        }
        let actual = self.placements(group)?;
        if !actual.iter().map(|placement| &placement.id).eq(ordered.iter())
            || !actual.iter().filter_map(|placement| placement.image.as_ref()).eq(images.iter())
        {
            return Err(cli::Error::ScreenshotOrder { group: group.into() }.into());
        }
        Ok(())
    }

    fn string(value: &Value, key: &str) -> Result<String> {
        value[key]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| app_store::cli::Error::StoreResponseField { field: key.into() }.into())
    }
}

struct ScreenshotSpec {
    id: String,
    width: u32,
    height: u32,
    max_file_size: u64,
}

impl ScreenshotSpec {
    fn accepts(&self, image: &StoreScreenshot) -> bool {
        self.width == image.width() && self.height == image.height()
    }
}

struct ScreenshotGroup {
    id: String,
    family: ScreenshotFamily,
    display_size: f64,
    max_count: usize,
    specs: Vec<ScreenshotSpec>,
}

impl ScreenshotGroup {
    fn read(attributes: &Value) -> Result<Vec<Self>> {
        let feature = attributes["features"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|feature| feature["featureId"] == "APP_STORE_VERSIONS")
            .ok_or(cli::Error::CatalogFeatureMissing)?;
        let policy = feature["placementPolicies"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|policy| policy["placementType"] == "APP_SCREENSHOT")
            .ok_or(cli::Error::CatalogPolicyMissing)?;
        let mut limits = BTreeMap::new();
        for limit in policy["groupLimits"].as_array().into_iter().flatten() {
            let count = limit["maxCount"].as_u64().ok_or_else(|| {
                app_store::cli::Error::StoreResponseField {
                    field: "placementPolicies.groupLimits.maxCount".into(),
                }
            })?;
            for group in
                limit["groupIds"].as_array().into_iter().flatten().filter_map(Value::as_str)
            { limits.insert(group, count as usize); }
        }
        let placement = attributes["placementTypes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|placement| placement["placementTypeId"] == "APP_SCREENSHOT")
            .ok_or(cli::Error::CatalogSpecificationsMissing)?;
        if !placement["acceptsAssetCategories"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|category| category == "APP_SCREENSHOTS_AND_PREVIEWS")
        {
            return Err(cli::Error::CatalogCategoryRejected.into());
        }
        let mut groups = Vec::new();
        for group in attributes["placementProfileGroups"].as_array().into_iter().flatten() {
            let id = AppStoreRelease::string(group, "placementProfileGroupId")?;
            let Some(count) = limits.get(id.as_str()) else { continue; };
            let display = attributes["displayClasses"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|display| display["displayClassId"] == group["displayClassId"]);
            let Some(display) = display else { continue; };
            let family = match display["deviceFamily"].as_str() {
                Some("IPHONE") => ScreenshotFamily::Iphone,
                Some("IPAD") => ScreenshotFamily::Ipad,
                _ => continue,
            };
            let mapping = placement["specMappings"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|mapping| mapping["placementGroupId"] == id);
            let Some(mapping) = mapping else { continue; };
            let ids: Vec<_> = mapping["specs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let mut specs = Vec::new();
            for spec in attributes["imageSpecs"].as_array().into_iter().flatten() {
                if !ids.contains(&spec["specId"].as_str().unwrap_or("")) { continue; }
                if !spec["fileExtensions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|extension| extension == ".png")
                    || !spec["mimeTypes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|mime| mime == "image/png")
                { continue; }
                let dimensions = &spec["dimensions"];
                if dimensions["minWidth"] != dimensions["maxWidth"]
                    || dimensions["minHeight"] != dimensions["maxHeight"]
                { continue; }
                let width =
                    dimensions["minWidth"].as_u64().and_then(|value| u32::try_from(value).ok());
                let height =
                    dimensions["minHeight"].as_u64().and_then(|value| u32::try_from(value).ok());
                let Some((width, height)) =
                    width.zip(height).filter(|(width, height)| *width > 0 && *height > 0)
                else {
                    return Err(cli::Error::CatalogDimensionsInvalid.into());
                };
                let max_file_size = spec["maxFileSize"].as_u64().ok_or_else(|| {
                    app_store::cli::Error::StoreResponseField { field: "imageSpecs.maxFileSize".into() }
                })?;
                specs.push(ScreenshotSpec {
                    id: AppStoreRelease::string(spec, "specId")?,
                    width,
                    height,
                    max_file_size,
                });
            }
            if specs.is_empty() { continue; }
            let display_size = display["screenDimensions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|size| size.as_str().and_then(|size| size.parse::<f64>().ok()))
                .filter(|size| size.is_finite())
                .max_by(f64::total_cmp)
                .unwrap_or(0.0);
            groups.push(Self { id, family, display_size, max_count: *count, specs });
        }
        if groups.is_empty() {
            return Err(cli::Error::CatalogSpecificationsUnavailable.into());
        }
        Ok(groups)
    }
}

struct Placement {
    id: String,
    image: Option<String>,
}
