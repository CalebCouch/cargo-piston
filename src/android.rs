use cargo_metadata::{Metadata, MetadataCommand};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{read_dir, File, copy, create_dir_all, remove_file, rename};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::project::Project;
use crate::host::Host;
use crate::PistonError;

mod development;
pub use development::AndroidDevelopment;
pub mod cli;

pub struct AndroidBuilder {
    release: bool,
    target: String,
    cwd: PathBuf,
    build_path: PathBuf,
    output_path: PathBuf,
    icon_path: String,
    assets: String,
    app_name: String,
    lib_name: String,
    manifest: AndroidManifest,
    manifest_path: PathBuf,
    key_set: KeySet,
    ndk_path: String,
    sdk_path: String,
    java_path: String,
    resources: PathBuf,
    build_tools_version: String,
    bundletool_path: String,
}

impl AndroidBuilder {
    fn new(release: bool, target: String, cwd: PathBuf, env_vars: HashMap<String, String>) -> Result<Self, PistonError> {
        let ndk_path: &String = AndroidBuilder::get_or_err(&env_vars, "ndk_path")?;
        let sdk_path: &String = AndroidBuilder::get_or_err(&env_vars, "sdk_path")?;
        let java_path: &String = AndroidBuilder::get_or_err(&env_vars, "java_path")?;
        let bundletool_path: &String = AndroidBuilder::get_or_err(&env_vars, "bundletool_path")?;
        let build_tools_version: String = AndroidBuilder::get_build_tools_version(sdk_path)?;
        let key_set = KeySet::new(release, &env_vars)?;
        let metadata: Metadata = MetadataCommand::new()
            .current_dir(&cwd)
            .exec()
            .map_err(|e| PistonError::CargoParseError(e.to_string()))?;

        let lib_name = Project::get_lib_name(&metadata)?;
        let icon_path = Project::get_icon_path(&metadata, &cwd);
        let assets = Project::get_assets_path(&metadata);
        let app_name = Project::get_app_name(&metadata)?;
        let app_version = Project::get_app_version(&metadata)?;
        let manifest = AndroidManifest::build(&metadata, &app_name, &app_version)?;
        let build_path: PathBuf = cwd
            .join("target")
            .join(if release { "release" } else { "debug" })
            .join("android")
            .join("androidbuilder");
        Project::empty_directory(build_path.as_path(), &["assets"])?;
        create_dir_all(&build_path)
            .map_err(|e| PistonError::CreateDirAllError { path: build_path.clone(), source: e })?;
        let manifest_path: PathBuf = build_path.join("AndroidManifest.xml");
        let resources_path: PathBuf = build_path.join("app").join("src").join("main").join("res");
        manifest.write_to(&manifest_path)?;
        let profile = if release { "release" } else { "debug" };
        let output_path = cwd.join(format!("target/{}/android", profile));
        Ok(AndroidBuilder {
            release,
            target,
            cwd,
            build_path,
            output_path,
            icon_path,
            assets,
            app_name,
            lib_name,
            manifest,
            manifest_path,
            key_set,
            ndk_path: ndk_path.to_string(),
            sdk_path: sdk_path.to_string(),
            java_path: java_path.to_string(),
            resources: resources_path,
            build_tools_version,
            bundletool_path: bundletool_path.to_string(),
        })
    }

    fn pre_build(&self) -> Result<(), PistonError> {
        let path = self.resources.as_path();
        Project::empty_directory(path, &[])?;
        create_dir_all(&self.resources).map_err(|e| PistonError::CreateDirAllError {
            path: self.resources.clone(),
            source: e,
        })?;
        let hdpi_path: PathBuf = self.resources.join("mipmap-hdpi");
        let mdpi_path: PathBuf = self.resources.join("mipmap-mdpi");
        let xhdpi_path: PathBuf = self.resources.join("mipmap-xhdpi");
        let xxhdpi_path: PathBuf = self.resources.join("mipmap-xxhdpi");
        let xxxhdpi_path: PathBuf = self.resources.join("mipmap-xxxhdpi");
        create_dir_all(&hdpi_path)
            .map_err(|e| PistonError::CreateDirAllError { path: hdpi_path.clone(), source: e })?;
        create_dir_all(&mdpi_path)
            .map_err(|e| PistonError::CreateDirAllError { path: mdpi_path.clone(), source: e })?;
        create_dir_all(&xhdpi_path)
            .map_err(|e| PistonError::CreateDirAllError { path: xhdpi_path.clone(), source: e })?;
        create_dir_all(&xxhdpi_path)
            .map_err(|e| PistonError::CreateDirAllError { path: xxhdpi_path.clone(), source: e })?;
        create_dir_all(&xxxhdpi_path).map_err(|e| PistonError::CreateDirAllError {
            path: xxxhdpi_path.clone(),
            source: e,
        })?;
        let hdpi_target: PathBuf = hdpi_path.join("ic_launcher.png");
        Project::resize_png(self.icon_path.as_ref(), &hdpi_target.display().to_string(), 48, 48)?;
        let mdpi_target: PathBuf = mdpi_path.join("ic_launcher.png");
        Project::resize_png(self.icon_path.as_ref(), &mdpi_target.display().to_string(), 72, 72)?;
        let xhdpi_target: PathBuf = xhdpi_path.join("ic_launcher.png");
        Project::resize_png(self.icon_path.as_ref(), &xhdpi_target.display().to_string(), 96, 96)?;
        let xxhdpi_target: PathBuf = xxhdpi_path.join("ic_launcher.png");
        Project::resize_png(
            self.icon_path.as_ref(),
            &xxhdpi_target.display().to_string(),
            144,
            144,
        )?;
        let xxxhdpi_target: PathBuf = xxxhdpi_path.join("ic_launcher.png");
        Project::resize_png(
            self.icon_path.as_ref(),
            &xxxhdpi_target.display().to_string(),
            192,
            192,
        )?;
        Ok(())
    }

    fn build(&self) -> Result<PathBuf, PistonError> {
        self.build_so()?;
        let resources = self.compile_resources()?;
        let base_dir = self.build_path.join("base");
        Project::empty_directory(&base_dir, &["assets"])?;
        create_dir_all(&base_dir)
            .map_err(|e| PistonError::CreateDirAllError { path: base_dir.clone(), source: e })?;
        self.link_manifest_and_resources(&resources, &base_dir)?;
        let assets_src = Path::new(&self.assets);
        let assets_base = &base_dir.join("assets");
        Project::sync_assets(assets_src, assets_base)?;
        self.add_lib(&base_dir, self.target.as_ref())?;
        // Disabled until Maverick provides its Android runtime integration.
        // self.add_activity_dex(&base_dir)?;
        let base_zip = self.build_path.join("base.zip");
        self.zip_base(&base_dir)?;
        let aab_path = self.output_path.join(format!("{}.aab", self.app_name));
        self.build_bundle(&base_zip, &aab_path)?;

        Ok(aab_path)
    }

    fn post_build(&self, aab_path: PathBuf) -> Result<(), PistonError> {
        let key_path_exists = Path::new(&self.key_set.key_path).exists();
        let key_alias_exists = self.verify_key_alias()?;

        if !key_path_exists || !key_alias_exists { self.create_signing_key()?; }
        self.sign_aab(aab_path)
    }

    fn build_so(&self) -> Result<(), PistonError> {
        let host_platform = AndroidBuilder::get_host_platform(self.ndk_path.as_ref())?;
        let api_level = self.manifest.min_sdk_version.to_string();
        let linker_name = if self.target == "armv7-linux-androideabi" {
            format!("armv7a-linux-androideabi{}-clang", api_level)
        } else {
            format!("{}{}-clang", self.target, api_level)
        };
        let ndk_path_buf = PathBuf::from(&self.ndk_path);
        let linker_path = ndk_path_buf
            .join("toolchains/llvm/prebuilt")
            .join(&host_platform)
            .join("bin")
            .join(linker_name);
        let ar_path = ndk_path_buf
            .join("toolchains/llvm/prebuilt")
            .join(&host_platform)
            .join("bin")
            .join("llvm-ar");
        if !linker_path.exists() {
            return Err(PistonError::BuildError(format!(
                "Linker not found at {}",
                linker_path.display()
            )));
        }
        if !ar_path.exists() {
            return Err(PistonError::BuildError(format!("AR not found at {}", ar_path.display())));
        }

        // handle cc crate linker for rusqlite
        let target_underscored = self.target.replace('-', "_");

        let cxx_name = if self.target == "armv7-linux-androideabi" {
            format!("armv7a-linux-androideabi{}-clang++", api_level)
        } else {
            format!("{}{}-clang++", self.target, api_level)
        };

        let cxx_path = ndk_path_buf
            .join("toolchains/llvm/prebuilt")
            .join(&host_platform)
            .join("bin")
            .join(cxx_name);

        if !cxx_path.exists() {
            return Err(PistonError::BuildError(format!(
                "C++ compiler not found at {}",
                cxx_path.display()
            )));
        }

        let cc_env_key = format!("CC_{}", target_underscored);
        let cxx_env_key = format!("CXX_{}", target_underscored);
        let ar_env_key_cc = format!("AR_{}", target_underscored);

        let target_upper = self.target.to_uppercase().replace("-", "_");
        let linker_env_key = format!("CARGO_TARGET_{}_LINKER", target_upper);
        let ar_env_key = format!("CARGO_TARGET_{}_AR", target_upper);
        let release = if self.release { "--release" } else { "" };
        let cargo_command = format!("cargo build --target {}  {} --lib", self.target, release);
        let builder = Command::new("bash")
            .arg("-c")
            .arg(&cargo_command)
            .current_dir(&self.build_path)
            .env("JAVA_HOME", &self.java_path)
            .env("ANDROID_HOME", &self.sdk_path)
            .env("NDK_HOME", &self.ndk_path)
            .env(&linker_env_key, linker_path.to_str().unwrap())
            .env(&ar_env_key, ar_path.to_str().unwrap())
            .env(&cc_env_key, linker_path.to_str().unwrap())
            .env(&cxx_env_key, cxx_path.to_str().unwrap())
            .env(&ar_env_key_cc, ar_path.to_str().unwrap())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| PistonError::BuildError(format!("Cargo build failed: {}", e)))?;
        if !builder.status.success() {
            return Err(PistonError::BuildError(format!(
                "Cargo build failed: {}",
                String::from_utf8_lossy(&builder.stderr)
            )));
        }
        Ok(())
    }

    fn compile_resources(&self) -> Result<PathBuf, PistonError> {
        let compiled_res = self.build_path.join("compiled_resources.zip");
        if compiled_res.exists() {
            remove_file(&compiled_res).map_err(|e| PistonError::RemoveFileError {
                path: compiled_res.to_path_buf(),
                source: e,
            })?;
        }
        let sdk = PathBuf::from(&self.sdk_path);
        let aapt2_path = sdk.join(format!("build-tools/{}/aapt2", self.build_tools_version));

        let compile_command = format!(
            "{} compile --dir {} -o {}",
            aapt2_path.display(),
            self.resources.display(),
            compiled_res.display()
        );

        Command::new("bash")
            .arg("-c")
            .arg(&compile_command)
            .current_dir(&self.build_path)
            .env("ANDROID_HOME", &self.sdk_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| PistonError::BuildError(format!("aapt2 compile failed: {}", e)))?;
        Ok(compiled_res)
    }

    fn link_manifest_and_resources(&self, compiled_res: &Path, base_dir: &Path) -> Result<(), PistonError> {
        let aapt2_path: PathBuf = PathBuf::from(&self.sdk_path)
            .join(format!("build-tools/{}/aapt2", self.build_tools_version));
        let android_jar: PathBuf = PathBuf::from(&self.sdk_path)
            .join(format!("platforms/android-{}/android.jar", self.manifest.target_sdk_version));

        let res_arg = if compiled_res.exists() {
            format!(" {}", compiled_res.display())
        } else {
            String::new()
        };
        let link_command = format!(
            "{} link --proto-format --output-to-dir -o {} --manifest {} -I {} {}",
            aapt2_path.display(),
            base_dir.display(),
            self.manifest_path.display(),
            android_jar.display(),
            res_arg
        );

        Command::new("bash")
            .arg("-c")
            .arg(&link_command)
            .current_dir(&self.build_path)
            .env("ANDROID_HOME", &self.sdk_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| PistonError::ProtoLinkError(format!("aapt2 link failed: {}", e)))?;

        let proto_manifest_root = base_dir.join("AndroidManifest.xml");
        if proto_manifest_root.exists() {
            let manifest_dir = base_dir.join("manifest");
            Project::empty_directory(&manifest_dir, &[])?;
            create_dir_all(&manifest_dir).map_err(|e| PistonError::CreateDirAllError {
                path: manifest_dir.clone(),
                source: e,
            })?;
            rename(&proto_manifest_root, manifest_dir.join("AndroidManifest.xml")).map_err(
                |e| PistonError::RenameFileError { path: proto_manifest_root.clone(), source: e },
            )?;
        } else {
            return Err(PistonError::ProtoLinkError(
                "Proto AndroidManifest.xml not generated and linked by AAPT2".to_string(),
            ));
        }
        Ok(())
    }

    fn add_lib(&self, base_dir: &Path, target: &str) -> Result<(), PistonError> {
        let abi = match target {
            "aarch64-linux-android" => "arm64-v8a",
            _ => {
                return Err(PistonError::UnsupportedTargetError(format!(
                    "Unsupported target {}",
                    target
                )));
            }
        };
        let lib_dir = base_dir.join("lib").join(abi);
        Project::empty_directory(&lib_dir, &[])?;
        create_dir_all(&lib_dir)
            .map_err(|e| PistonError::CreateDirAllError { path: lib_dir.clone(), source: e })?;

        let lib_file = format!("lib{}.so", self.lib_name);
        let so_path = self
            .cwd
            .join("target")
            .join(target)
            .join(if self.release { "release" } else { "debug" })
            .join(&lib_file);
        copy(&so_path, lib_dir.join(&lib_file))
            .map_err(|e| PistonError::BuildError(format!("Failed to copy .so: {}", e)))?;

        Ok(())
    }

    /* Retained until the photo picker moves into Maverick's Android integration.
    fn add_activity_dex(&self, base_dir: &Path) -> Result<(), PistonError> {
        let dex_dir = base_dir.join("dex");
        create_dir_all(&dex_dir).map_err(|e| PistonError::CreateDirAllError {
            path: dex_dir.clone(), source: e,
        })?;

        let dex_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/android_activity/classes.dex");
        copy(&dex_path, dex_dir.join("classes.dex")).map_err(|e| {
            PistonError::BuildError(format!("Failed to copy activity classes.dex: {}", e))
        })?;

        Ok(())
    }
    */

    fn zip_base(&self, base_dir: &Path) -> Result<(), PistonError> {
        let zip_path = self.build_path.join("base.zip");
        if zip_path.exists() {
            remove_file(&zip_path).map_err(|e| PistonError::RemoveFileError {
                path: zip_path.to_path_buf(),
                source: e,
            })?;
        }
        let zip_command = format!("cd {} && zip -r ../base.zip *", base_dir.display());

        Command::new("bash")
            .arg("-c")
            .arg(&zip_command)
            .current_dir(&self.build_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| PistonError::BuildError(format!("Zip failed: {}", e)))?;

        Ok(())
    }

    fn build_bundle(&self, base_zip: &Path, aab_path: &Path) -> Result<(), PistonError> {
        if aab_path.exists() {
            remove_file(&aab_path).map_err(|e| PistonError::RemoveFileError {
                path: aab_path.to_path_buf(),
                source: e,
            })?;
        }
        let bundle_command = format!(
            "java -jar {} build-bundle --modules={} --output={}",
            self.bundletool_path,
            base_zip.display(),
            aab_path.display()
        );

        Command::new("bash")
            .arg("-c")
            .arg(&bundle_command)
            .current_dir(&self.build_path)
            .env("JAVA_HOME", &self.java_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| PistonError::BuildError(format!("bundletool failed: {}", e)))?;
        Ok(())
    }

    fn create_signing_key(&self) -> Result<(), PistonError> {
        let keystore_path = &self.key_set.key_path;
        let pass = &self.key_set.key_pass;
        let alias = &self.key_set.key_alias;
        let dname = format!(
            "CN={}, OU={}, O={}, L={}, S={}, C={}",
            self.key_set.common_name,
            self.key_set.org_unit,
            self.key_set.org,
            self.key_set.locality,
            self.key_set.state,
            self.key_set.country
        );

        if let Some(parent) = Path::new(&keystore_path).parent() {
            create_dir_all(parent).map_err(|e| PistonError::CreateDirAllError {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        let keytool = format!("{}/bin/keytool", self.java_path);

        let output = Command::new(&keytool)
            .arg("-genkeypair")
            .arg("-v")
            .arg("-keystore")
            .arg(&keystore_path)
            .arg("-storepass")
            .arg(&pass)
            .arg("-keypass")
            .arg(&pass)
            .arg("-alias")
            .arg(&alias)
            .arg("-keyalg")
            .arg("RSA")
            .arg("-keysize")
            .arg("2048")
            .arg("-validity")
            .arg("10000")
            .arg("-dname")
            .arg(&dname)
            .output()
            .map_err(|e| {
                PistonError::KeyToolError(format!(
                    "Failed to generate {} key with keytool: {}",
                    if self.release { "Release" } else { "Debug" },
                    e
                ))
            })?;

        if !output.status.success() {
            return Err(PistonError::KeyToolError(format!(
                "Failed to generate {} key: {}",
                if self.release { "Release" } else { "Debug" },
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    fn verify_key_alias(&self) -> Result<bool, PistonError> {
        let keytool = format!("{}/bin/keytool", self.java_path);
        let output = Command::new(keytool)
            .arg("-list")
            .arg("-v")
            .arg("-keystore")
            .arg(&self.key_set.key_path)
            .arg("-storepass")
            .arg(&self.key_set.key_pass)
            .output()
            .map_err(|e| {
                PistonError::KeyToolError(format!("Failed to list keystore contents: {}", e))
            })?;

        if !output.status.success() { return Ok(false); }

        let stdout = String::from_utf8_lossy(&output.stdout);

        for line in stdout.lines() {
            if let Some(found) = line.strip_prefix("Alias name:") {
                return Ok(found.trim() == self.key_set.key_alias);
            }
        }
        Ok(false)
    }

    fn sign_aab(&self, aab_path: PathBuf) -> Result<(), PistonError> {
        let sdk = PathBuf::from(&self.sdk_path);
        let apksigner_path =
            sdk.join(format!("build-tools/{}/apksigner", self.build_tools_version));
        let api_level = self.manifest.min_sdk_version.to_string();

        let output = Command::new(&apksigner_path)
            .env("JAVA_HOME", &self.java_path)
            .arg("sign")
            .arg("--ks")
            .arg(&self.key_set.key_path)
            .arg("--ks-key-alias")
            .arg(&self.key_set.key_alias)
            .arg("--ks-pass")
            .arg(format!("pass:{}", self.key_set.key_pass))
            .arg("--key-pass")
            .arg(format!("pass:{}", self.key_set.key_pass))
            .arg("--min-sdk-version")
            .arg(&api_level)
            .arg(&aab_path)
            .output()
            .map_err(|e| PistonError::APKSignerError(format!("Error signing AAB: {}", e)))?;

        if !output.status.success() {
            return Err(PistonError::APKSignerError(format!(
                "Error signing AAB: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }
}

impl AndroidBuilder {
    pub fn get_or_err<'a>(map: &'a HashMap<String, String>, key: &str) -> Result<&'a String, PistonError> {
        map.get(key).ok_or_else(|| {
            PistonError::AndroidConfigError(format!("key '{}' not found in .env", key))
        })
    }

    pub fn get_host_platform(ndk_path: &str) -> Result<String, PistonError> {
        let prebuilt_path = PathBuf::from(ndk_path).join("toolchains/llvm/prebuilt");
        let host_dirs = Self::directory_names(&prebuilt_path)?;
        if host_dirs.is_empty() {
            return Err(PistonError::BuildError(
                "No host platform dir found in NDK prebuilt".to_string(),
            ));
        }
        Ok(host_dirs.into_iter().next().unwrap())
    }

    pub fn get_build_tools_version(sdk_path: &str) -> Result<String, PistonError> {
        let prebuilt_path = PathBuf::from(sdk_path).join("build-tools");
        let host_dirs = Self::directory_names(&prebuilt_path)?;
        if host_dirs.is_empty() {
            return Err(PistonError::BuildError(
                "No build tools version found in SDK prebuilt".to_string(),
            ));
        }
        Ok(host_dirs.into_iter().next().unwrap())
    }

    fn directory_names(path: &Path) -> Result<Vec<String>, PistonError> {
        let entries = read_dir(path)
            .map_err(|e| PistonError::BuildError(format!("Failed to read prebuilt dir: {}", e)))?;
        let mut names = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|e| PistonError::BuildError(format!("Dir entry error: {}", e)))?;
            let file_type = entry
                .file_type()
                .map_err(|e| PistonError::BuildError(format!("File type error: {}", e)))?;
            if file_type.is_dir() {
                if let Some(name) = entry.file_name().to_str() { names.push(name.to_string()); }
            }
        }
        Ok(names)
    }
}

pub struct AndroidRunner;

impl AndroidRunner {
    fn deploy_usb(
        device_id: &str, output_path: PathBuf, app_name: String, cwd: PathBuf, package: String,
        env_vars: HashMap<String, String>,
    ) -> Result<(), PistonError> {
        let aab_path = output_path.join(format!("{}.aab", app_name));
        let bundletool_path: &String = AndroidBuilder::get_or_err(&env_vars, "bundletool_path")?;
        let java_path: &String = AndroidBuilder::get_or_err(&env_vars, "java_path")?;
        let sdk_path: &String = AndroidBuilder::get_or_err(&env_vars, "sdk_path")?;
        let adb_path: String = format!("{}/platform-tools/adb", sdk_path);
        let apk_path = cwd.join(format!("{}.apks", app_name));
        let bundle_cmd = format!(
            "java -jar {} build-apks --bundle={} --output={} --connected-device --overwrite --adb {}",
            &bundletool_path,
            &aab_path.display(),
            &apk_path.display(),
            &adb_path
        );

        let output = Command::new("bash")
            .arg("-c")
            .arg(&bundle_cmd)
            .env("JAVA_HOME", &java_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| {
                PistonError::ExtractAPKError(format!("Bundletool failed to extract the APK: {}", e))
            })?;
        if !output.status.success() {
            return Err(PistonError::ExtractAPKError(format!(
                "Bundletool failed to extract APK: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let bundle_cmd = format!(
            "java -jar {} install-apks --apks={} --device-id={} --adb {}",
            &bundletool_path,
            &apk_path.display(),
            device_id,
            &adb_path,
        );

        let output = Command::new("bash")
            .arg("-c")
            .arg(&bundle_cmd)
            .env("JAVA_HOME", &java_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| {
                PistonError::InstallAPKError(format!("Bundletool failed to install the APK: {}", e))
            })?;
        if !output.status.success() {
            return Err(PistonError::InstallAPKError(format!(
                "Bundletool failed to install APK: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let launch = format!("{}/android.app.NativeActivity", package);
        let adb_cmd = format!("{} shell am start -n {}", &adb_path, &launch,);

        let output = Command::new("bash")
            .arg("-c")
            .arg(&adb_cmd)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| PistonError::RunAPKError(format!("ADB failed to run the APK: {}", e)))?;
        if !output.status.success() {
            return Err(PistonError::RunAPKError(format!(
                "ADB failed to run APK: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }
}

//TODO build out intent filters with more robust cargo.toml parameters

#[derive(Deserialize, Default)]
struct AndroidMetadata {
    #[serde(default)]
    package: Option<String>,
    #[serde(default)]
    version_code: Option<u32>,
    #[serde(default)]
    version_name: Option<String>,
    #[serde(default)]
    min_sdk_version: Option<u32>,
    #[serde(default)]
    target_sdk_version: Option<u32>,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Deserialize, Default, Debug)]
struct AndroidManifest {
    package: String,
    version_code: u32,
    version_name: String,
    min_sdk_version: u32,
    target_sdk_version: u32,
    app_label: String,
    app_name: String,
    icon: String,
}

impl AndroidManifest {
    fn build(metadata: &Metadata, app_name: &str, version: &str) -> Result<Self, PistonError> {
        let package = metadata.root_package().ok_or_else(|| {
            PistonError::ParseManifestError("No Root package found in metadata".to_string())
        })?;
        let crate_name = &package.name;
        let android_meta_value: Value =
            package.metadata.get("android").cloned().unwrap_or(Value::Object(Default::default()));
        let android_meta: AndroidMetadata =
            serde_json::from_value(android_meta_value).map_err(|e| {
                PistonError::ParseManifestError(format!(
                    "Failed to deserialize [package.metadata.android: {}]",
                    e
                ))
            })?;
        Ok(Self {
            package: android_meta.package.unwrap_or(format!("com.example.{}", crate_name)),
            version_code: android_meta.version_code.unwrap_or(1),
            version_name: android_meta.version_name.unwrap_or(version.to_string()),
            min_sdk_version: android_meta.min_sdk_version.unwrap_or(21),
            target_sdk_version: android_meta.target_sdk_version.unwrap_or(34),
            app_label: android_meta.label.unwrap_or(format!("{}", crate_name)),
            app_name: app_name.to_string(),
            icon: "@mipmap/ic_launcher".to_string(),
        })
    }

    fn to_xml(&self) -> String {
        let icon_attr = format!(r#" android:icon="{}""#, Self::escape_xml(&self.icon));

        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
            <manifest xmlns:android="http://schemas.android.com/apk/res/android"
                package="{package}"
                android:versionCode="{version_code}"
                android:versionName="{version_name}">

                <uses-sdk android:minSdkVersion="{min_sdk}" android:targetSdkVersion="{target_sdk}" />
                <!-- Disabled until Maverick provides its Android runtime integration.
                <uses-permission android:name="android.permission.CAMERA" />
                <uses-feature android:name="android.hardware.camera" android:required="false" />
                -->

                <application android:label="{label}" android:hasCode="false"{icon_attr}>
                    <activity android:name="android.app.NativeActivity"
                        android:label="{label}"
                        android:exported="true">
                        <meta-data android:name="android.app.lib_name" android:value="{app_name}" />
                        <intent-filter>
                            <action android:name="android.intent.action.MAIN" />
                            <category android:name="android.intent.category.LAUNCHER" />
                        </intent-filter>
                    </activity>
                    <!--
                    <activity android:name="com.maverick.photo.PhotoPickerActivity"
                        android:exported="false" />
                    -->
                </application>
            </manifest>"#,
            package = Self::escape_xml(&self.package),
            version_code = self.version_code,
            version_name = Self::escape_xml(&self.version_name),
            min_sdk = self.min_sdk_version,
            target_sdk = self.target_sdk_version,
            label = Self::escape_xml(&self.app_label),
            app_name = Self::escape_xml(&self.app_name),
            icon_attr = icon_attr,
        )
    }

    fn write_to(&self, dir: &Path) -> Result<(), PistonError> {
        let file = File::create(dir).map_err(|e| {
            PistonError::CreateManifestError(format!("Failed to create manifest file: {}", e))
        })?;

        let mut writer = BufWriter::new(file);
        writer.write_all(self.to_xml().as_bytes()).map_err(|e| {
            PistonError::WriteManifestError(format!("Failed to write manifest file: {}", e))
        })?;

        Ok(())
    }

    fn escape_xml(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
}

struct KeySet {
    key_path: String,
    key_pass: String,
    key_alias: String,
    common_name: String,
    org_unit: String,
    org: String,
    locality: String,
    state: String,
    country: String,
}

impl KeySet {
    fn new(release: bool, env_vars: &HashMap<String, String>) -> Result<KeySet, PistonError> {
        let default_path = Host::android_keystore(release)?;

        let (default_pass, default_alias, default_cn, default_org) = if release {
            ("piston", "release-key", "Unknown", "Unknown")
        } else {
            ("android", "androiddebugkey", "Android Debug", "Android")
        };

        let key_pass = if release {
            env_vars.get("aab_key_pass").cloned().unwrap_or(default_pass.to_string())
        } else {
            default_pass.to_string()
        };

        let key_path = if release {
            env_vars.get("aab_release_key").cloned().unwrap_or(default_path)
        } else {
            default_path
        };

        let key_alias = if release {
            env_vars.get("aab_key_alias").cloned().unwrap_or(default_alias.to_string())
        } else {
            default_alias.to_string()
        };

        let common_name = env_vars.get("common_name").cloned().unwrap_or(default_cn.to_string());

        let org = env_vars.get("org").cloned().unwrap_or(default_org.to_string());

        let org_unit = env_vars.get("org_unit").cloned().unwrap_or("Development".to_string());

        let locality = env_vars.get("locality").cloned().unwrap_or("Unknown".to_string());

        let state = env_vars.get("state").cloned().unwrap_or("Unknown".to_string());

        let country: String = env_vars
            .get("country")
            .filter(|s| s.trim().len() == 2)
            .map(|s| s.trim().to_uppercase())
            .unwrap_or_else(|| "US".to_string());

        Ok(KeySet {
            key_path,
            key_pass,
            key_alias,
            common_name,
            org_unit,
            org,
            locality,
            state,
            country,
        })
    }
}
