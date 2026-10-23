use cargo_metadata::TargetKind;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use crate::dev::DevelopmentArgs;
use crate::host::Host;
use crate::manifest::Manifest;
use crate::project::Project;
use crate::{PistonError, Result};

pub struct AndroidDevelopment {
    project: Project,
    sdk: PathBuf,
    java: PathBuf,
    bridge: PathBuf,
    package_id: String,
    serial: String,
    abi: String,
    ndk: PathBuf,
    emulator: Option<Child>,
    interrupted: std::sync::mpsc::Receiver<()>,
}

impl AndroidDevelopment {
    pub fn start(args: DevelopmentArgs) -> Result<()> {
        let mut session = AndroidDevelopment::new(&args)?;
        println!("Preparing Android tools");
        session.prepare()?;
        println!("Selecting an Android device or emulator");
        session.serial = if let Some(serial) = session.select_device(&args)? {
            serial
        } else {
            println!("Opening an Android emulator");
            session.start_emulator(args.device.as_deref())?
        };
        println!("Connecting to {}", session.serial);
        session.wait_for_boot()?;
        session.select_target()?;
        println!(
            "Building {} for {} ({})",
            session.project.package.name,
            session.serial,
            session.project.args.cargo.target.as_ref().unwrap()
        );
        let library = session.compile()?;
        println!("Packaging Android app with Maverick's activity");
        let apk = session.package(&library)?;
        session.install(&apk)?;
        println!("Launching {}. App output follows; press Ctrl-C to stop.", session.package_id);
        session.launch()
    }

    fn new(args: &DevelopmentArgs) -> Result<Self> {
        if !args.app_args.is_empty() {
            return Err(PistonError::Generic(
                "Android NativeActivity does not accept desktop command-line arguments.".into(),
            ));
        }
        let project = Project::new(args.application.clone(), "android", true)?;
        let metadata = project.dependencies()?;
        let bridge = metadata.packages.iter().find(|package| package.name.as_str() == "maverick_os")
            .map(|package| package.manifest_path.parent().unwrap().as_std_path().join("android"))
            .filter(|path| path.join("bridge/build.gradle").is_file() && path.join("gradlew").is_file())
            .ok_or_else(|| PistonError::Generic(
                "The selected Maverick dependency needs its android/ Gradle bridge. \
                 Use maverick_os 3.0.3 or later.".into(),
            ))?;
        let settings =
            Manifest::new(project.package.manifest_path.as_std_path())?.settings(&["android"])?;
        let sdk = match settings.get("sdk") {
            Some(path) => PathBuf::from(path),
            None => Host::android_sdk()?,
        };
        let java = match settings.get("java") {
            Some(path) => PathBuf::from(path),
            None => Host::android_java()?,
        };
        let ndk =
            settings.get("ndk").map(PathBuf::from).unwrap_or_else(|| sdk.join("ndk/29.0.14033849"));
        let package_id =
            project.args.bundle_id.clone().or(settings.get("bundle_id").cloned()).unwrap_or_else(
                || format!("dev.piston.{}", project.package.name.replace('-', "_")),
            );
        if !package_id.contains('.')
            || package_id.split('.').any(|part| {
                part.is_empty()
                    || !part.chars().next().unwrap().is_ascii_alphabetic()
                    || !part
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '_')
            })
        {
            return Err(PistonError::Generic(format!(
                "Invalid Android application ID: {package_id}"
            )));
        }
        let (cancel, interrupted) = std::sync::mpsc::sync_channel(1);
        ctrlc::set_handler(move || { let _ = cancel.try_send(()); })
        .map_err(|error| PistonError::Generic(format!("Cannot watch Android shutdown: {error}")))?;
        Ok(Self {
            project,
            sdk,
            java,
            bridge,
            package_id,
            serial: String::new(),
            abi: String::new(),
            ndk,
            emulator: None,
            interrupted,
        })
    }

    fn prepare(&self) -> Result<()> {
        let mut packages = Vec::new();
        for (relative, package) in [
            ("platform-tools/adb", "platform-tools"),
            ("platforms/android-36/android.jar", "platforms;android-36"),
            ("build-tools/36.0.0/apksigner", "build-tools;36.0.0"),
        ] { if !self.sdk.join(relative).is_file() { packages.push(package); } }
        if !self.ndk.is_dir() {
            if self.ndk != self.sdk.join("ndk/29.0.14033849") {
                return Err(PistonError::Generic(format!(
                    "Configured Android NDK does not exist: {}",
                    self.ndk.display()
                )));
            }
            packages.push("ndk;29.0.14033849");
        }
        if !packages.is_empty() {
            Project::run(self.sdkmanager()?.args(packages), "Installing Android SDK components")?;
        }
        if !Command::new("cargo")
            .args(["ndk", "--version"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            Project::run(
                Command::new("cargo").args(["install", "cargo-ndk", "--locked"]),
                "Installing Android Rust tooling",
            )?;
        }
        Ok(())
    }

    fn select_device(&self, args: &DevelopmentArgs) -> Result<Option<String>> {
        let bytes = Project::output(self.adb().args(["devices", "-l"]), "Finding Android devices")?;
        let listing = String::from_utf8_lossy(&bytes);
        let mut devices = Vec::new();
        for line in listing.lines().skip(1) {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 2 { continue; }
            let serial = fields[0];
            if args.simulator && !serial.starts_with("emulator-") { continue; }
            if let Some(selector) = &args.device {
                if selector != serial {
                    if !serial.starts_with("emulator-") || fields[1] != "device" { continue; }
                    let output = self
                        .adb()
                        .args(["-s", serial, "shell", "getprop", "ro.boot.qemu.avd_name"])
                        .output()
                        .map_err(|error| PistonError::Generic(error.to_string()))?;
                    if !output.status.success()
                        || String::from_utf8_lossy(&output.stdout).lines().next()
                            != Some(selector.as_str())
                    { continue; }
                }
            }
            if fields[1] != "device" {
                return Err(PistonError::Generic(format!(
                    "Android device {serial} is {}. Unlock it, enable USB debugging and accept this computer's debugging prompt.",
                    fields[1]
                )));
            }
            devices.push(serial.to_owned());
        }
        if !args.simulator && devices.iter().any(|serial| !serial.starts_with("emulator-")) {
            devices.retain(|serial| !serial.starts_with("emulator-"));
        }
        if devices.len() > 1 {
            return Err(PistonError::Generic(format!(
                "Multiple Android devices: {}. Select one with --device <serial>.",
                devices.join(", ")
            )));
        }
        Ok(devices.pop())
    }

    fn select_target(&mut self) -> Result<()> {
        let bytes = Project::output(
            self.device_command().args(["shell", "getprop", "ro.product.cpu.abilist"]),
            "Reading Android architecture",
        )?;
        let supported = String::from_utf8_lossy(&bytes);
        let supported: Vec<_> = supported.trim().split(',').collect();
        let candidates = [
            ("aarch64-linux-android", "arm64-v8a"),
            ("x86_64-linux-android", "x86_64"),
            ("armv7-linux-androideabi", "armeabi-v7a"),
            ("i686-linux-android", "x86"),
        ];
        let selected = candidates
            .iter()
            .find(|(target, abi)| {
                supported.contains(abi)
                    && self
                        .project
                        .args
                        .cargo
                        .target
                        .as_deref()
                        .is_none_or(|requested| requested == *target)
            })
            .ok_or_else(|| {
                PistonError::UnsupportedTargetError(format!(
                    "Requested Rust target cannot run on {}. Device ABIs: {}.",
                    self.serial,
                    supported.join(", ")
                ))
            })?;
        self.project.args.cargo.target = Some(selected.0.into());
        self.abi = selected.1.into();
        Project::rust_target(selected.0)
    }

    fn compile(&self) -> Result<PathBuf> {
        let output = self.project.work.join("native");
        let mut command = Command::new("cargo");
        command.args(["ndk", "-t", &self.abi, "--platform", "30", "-o"]).arg(&output).arg("rustc");
        self.project.build_options(&mut command);
        command
            .env("ANDROID_HOME", &self.sdk)
            .env("ANDROID_NDK_HOME", &self.ndk)
            .env("JAVA_HOME", &self.java);
        command.args(["--", "-C", "link-arg=-Wl,-z,max-page-size=16384"]);
        Project::run(&mut command, "Building Android library")?;
        let target = self
            .project
            .package
            .targets
            .iter()
            .find(|target| target.kind.contains(&TargetKind::CDyLib))
            .unwrap();
        let library = output.join(&self.abi).join(format!("lib{}.so", target.name));
        if !library.is_file() {
            return Err(PistonError::BuildError(format!(
                "cargo-ndk produced no library at {}",
                library.display()
            )));
        }
        Ok(library)
    }

    fn package(&self, library: &Path) -> Result<PathBuf> {
        let gradle = self.project.work.join("gradle");
        let source = gradle.join("app/src/main");
        let libraries = source.join("jniLibs").join(&self.abi);
        fs::create_dir_all(&libraries)
            .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        fs::copy(library, libraries.join(library.file_name().unwrap()))
            .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        let prebuilt = fs::read_dir(self.ndk.join("toolchains/llvm/prebuilt"))
            .map_err(|error| PistonError::Generic(error.to_string()))?
            .filter_map(|entry| entry.ok())
            .find(|entry| entry.path().join("bin/llvm-strip").is_file())
            .ok_or_else(|| PistonError::Generic("The NDK contains no host toolchain.".into()))?
            .path();
        Project::run(
            Command::new(prebuilt.join("bin/llvm-strip"))
                .arg("--strip-debug")
                .arg(libraries.join(library.file_name().unwrap())),
            "Stripping packaged Android debug symbols",
        )?;
        let target = self.project.args.cargo.target.as_ref().unwrap();
        let cxx_target =
            if target == "armv7-linux-androideabi" { "arm-linux-androideabi" } else { target };
        fs::copy(
            prebuilt.join("sysroot/usr/lib").join(cxx_target).join("libc++_shared.so"),
            libraries.join("libc++_shared.so"),
        )
        .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        let version = self.project.package.version.to_string();
        let lib = library.file_stem().unwrap().to_string_lossy();
        let lib = lib.strip_prefix("lib").unwrap_or(&lib);
        let label = self
            .project
            .package
            .name
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;");
        Project::write(
            &source.join("AndroidManifest.xml"),
            format!(
                r#"<manifest xmlns:android="http://schemas.android.com/apk/res/android">
  <application android:label="{label}" android:allowBackup="false" android:theme="@android:style/Theme.Material.NoActionBar">
    <activity android:name="me.ramp.maverick.MaverickActivity" android:exported="true"
      android:configChanges="keyboardHidden|orientation|screenSize|screenLayout|smallestScreenSize|density|uiMode">
      <meta-data android:name="android.app.lib_name" android:value="{lib}" />
      <intent-filter><action android:name="android.intent.action.MAIN" /><category android:name="android.intent.category.LAUNCHER" /></intent-filter>
    </activity>
  </application>
</manifest>"#
            ),
        )?;
        if let Some(icon) = crate::project::Project::find_icon(self.project.root()) {
            let image = image::open(&icon)
                .map_err(|source| PistonError::OpenImageError { path: icon, source })?;
            let path = source.join("res/drawable/icon.png");
            fs::create_dir_all(path.parent().unwrap())
                .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
            image
                .resize_exact(192, 192, image::imageops::FilterType::Lanczos3)
                .save(&path)
                .map_err(PistonError::WriteImageError)?;
            let path = source.join("AndroidManifest.xml");
            let contents = fs::read_to_string(&path)
                .map_err(|error| PistonError::Generic(error.to_string()))?;
            Project::write(
                &path,
                contents.replace("<application ", "<application android:icon=\"@drawable/icon\" "),
            )?;
        }
        Project::write(
            &gradle.join("settings.gradle"),
            format!(
                r#"pluginManagement {{ repositories {{ google(); mavenCentral(); gradlePluginPortal() }} }}
dependencyResolutionManagement {{ repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS); repositories {{ google(); mavenCentral() }} }}
rootProject.name = 'PistonApp'
include ':app', ':bridge'
project(':bridge').projectDir = file({})
"#,
                json!(self.bridge.join("bridge"))
            ),
        )?;
        let mut root_gradle = fs::read_to_string(self.bridge.join("build.gradle"))
            .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        root_gradle.push_str(&format!(
            "\nproject(':bridge') {{ layout.buildDirectory.set(file({})) }}\n",
            json!(gradle.join("bridge-build"))
        ));
        Project::write(&gradle.join("build.gradle"), root_gradle)?;
        Project::write(
            &gradle.join("gradle.properties"),
            "android.useAndroidX=true\norg.gradle.jvmargs=-Xmx2g\norg.gradle.workers.max=2\n",
        )?;
        Project::write(
            &gradle.join("app/build.gradle"),
            format!(
                r#"plugins {{ id 'com.android.application' }}
android {{
  namespace {}
  compileSdk 36
  buildToolsVersion '36.0.0'
  defaultConfig {{ applicationId {}; minSdk 30; targetSdk 36; versionCode 1; versionName {}; ndk {{ abiFilters {} }} }}
  packaging.jniLibs.useLegacyPackaging = false
}}
dependencies {{ implementation project(':bridge') }}
"#,
                json!(self.package_id),
                json!(self.package_id),
                json!(version),
                json!(self.abi)
            ),
        )?;
        Project::run(
            Command::new("sh")
                .arg(self.bridge.join("gradlew"))
                .arg("-p")
                .arg(&gradle)
                .args(["--no-daemon", "--console=plain", ":app:assembleDebug"])
                .env("ANDROID_HOME", &self.sdk)
                .env("JAVA_HOME", &self.java),
            "Packaging Android app",
        )?;
        let apk = gradle.join("app/build/outputs/apk/debug/app-debug.apk");
        Project::run(
            Command::new(self.sdk.join("build-tools/36.0.0/apksigner"))
                .arg("verify")
                .arg(&apk)
                .env("JAVA_HOME", &self.java),
            "Verifying Android app",
        )?;
        Ok(apk)
    }

    fn install(&self, apk: &Path) -> Result<()> {
        Project::run(
            self.device_command().args(["install", "-r", "-t"]).arg(apk),
            "Installing Android app",
        )
    }

    fn launch(&mut self) -> Result<()> {
        Project::output(
            self.device_command().args(["shell", "am", "force-stop", &self.package_id]),
            "Stopping previous Android app",
        )?;
        let output = Project::output(
            self.device_command().args([
                "shell",
                "am",
                "start",
                "-W",
                "-n",
                &format!("{}/me.ramp.maverick.MaverickActivity", self.package_id),
            ]),
            "Launching Android app",
        )?;
        if String::from_utf8_lossy(&output).contains("Error:") {
            return Err(PistonError::Generic(String::from_utf8_lossy(&output).into_owned()));
        }
        let mut pid = String::new();
        for _ in 0..20 {
            let output = self
                .device_command()
                .args(["shell", "pidof", "-s", &self.package_id])
                .output()
                .map_err(|error| PistonError::Generic(error.to_string()))?;
            pid = String::from_utf8_lossy(&output.stdout).trim().into();
            if !pid.is_empty() { break; }
            std::thread::sleep(Duration::from_millis(250));
        }
        if pid.is_empty() {
            Project::run(
                self.device_command().args(["logcat", "-b", "crash", "-d", "-t", "100"]),
                "Reading Android crash log",
            )?;
            return Err(PistonError::Generic(
                "Android app exited before log streaming began.".into(),
            ));
        }
        let mut logs = self
            .device_command()
            .args(["logcat", "--pid", &pid, "-v", "brief"])
            .spawn()
            .map_err(|error| PistonError::Generic(error.to_string()))?;
        let result = (|| {
            loop {
                if self.interrupted.try_recv().is_ok() { return Ok(()); }
                std::thread::sleep(Duration::from_secs(1));
                if let Some(status) =
                    logs.try_wait().map_err(|error| PistonError::Generic(error.to_string()))?
                {
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(PistonError::Generic("Android log connection ended.".into()))
                    };
                }
                let output = self
                    .device_command()
                    .args(["shell", "pidof", "-s", &self.package_id])
                    .output()
                    .map_err(|error| PistonError::Generic(error.to_string()))?;
                if String::from_utf8_lossy(&output.stdout).trim() != pid { return Ok(()); }
            }
        })();
        let _ = logs.kill();
        let _ = logs.wait();
        result
    }

    fn adb(&self) -> Command { Command::new(self.sdk.join("platform-tools/adb")) }
    fn device_command(&self) -> Command {
        let mut command = self.adb();
        command.args(["-s", &self.serial]);
        command
    }

    fn sdkmanager(&self) -> Result<Command> {
        let root = self.sdk.join("cmdline-tools");
        let tool = fs::read_dir(root)
            .map_err(|error| {
                PistonError::Generic(format!("Android command-line tools are missing: {error}"))
            })?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path().join("bin/sdkmanager"))
            .find(|path| path.is_file())
            .ok_or_else(|| {
                PistonError::Generic(
                    "Install Android SDK Command-line Tools in Android Studio's SDK Manager."
                        .into(),
                )
            })?;
        let mut command = Command::new(tool);
        command.env("JAVA_HOME", &self.java).arg(format!("--sdk_root={}", self.sdk.display()));
        Ok(command)
    }

    fn start_emulator(&mut self, selector: Option<&str>) -> Result<String> {
        let emulator = self.sdk.join("emulator/emulator");
        if !emulator.is_file() {
            Project::run(self.sdkmanager()?.arg("emulator"), "Installing Android Emulator")?;
        }
        let output = Project::output(
            Command::new(&emulator).arg("-list-avds"),
            "Listing Android emulators",
        )?;
        let names: Vec<_> = String::from_utf8_lossy(&output)
            .lines()
            .map(str::to_owned)
            .filter(|line| !line.is_empty())
            .collect();
        let name = match selector {
            Some(name) if names.iter().any(|value| value == name) => name.to_owned(),
            Some(name) => {
                return Err(PistonError::Generic(format!(
                    "No Android device or AVD matches {name}. Available AVDs: {}",
                    names.join(", ")
                )));
            }
            None if !names.is_empty() => names[0].clone(),
            None => {
                let abi = if Project::host_target()?.starts_with("aarch64") {
                    "arm64-v8a"
                } else {
                    "x86_64"
                };
                let package = format!("system-images;android-36;google_apis;{abi}");
                Project::run(
                    self.sdkmanager()?.arg(&package),
                    "Installing Android emulator image",
                )?;
                let sdkmanager = self.sdkmanager()?;
                let avdmanager = Path::new(sdkmanager.get_program()).with_file_name("avdmanager");
                let mut child = Command::new(avdmanager)
                    .args(["create", "avd", "--name", "piston", "--package", &package])
                    .env("JAVA_HOME", &self.java)
                    .env("ANDROID_HOME", &self.sdk)
                    .stdin(Stdio::piped())
                    .spawn()
                    .map_err(|error| PistonError::Generic(error.to_string()))?;
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(b"no\n")
                    .map_err(|error| PistonError::Generic(error.to_string()))?;
                if !child.wait().map_err(|error| PistonError::Generic(error.to_string()))?.success()
                { return Err(PistonError::Generic("Cannot create the Android emulator.".into())); }
                "piston".into()
            }
        };
        let port = (5554..5680)
            .step_by(2)
            .find(|port| {
                std::net::TcpListener::bind(("127.0.0.1", *port)).is_ok()
                    && std::net::TcpListener::bind(("127.0.0.1", *port + 1)).is_ok()
            })
            .ok_or_else(|| {
                PistonError::Generic("No free Android emulator console ports.".into())
            })?;
        let log = fs::File::create(self.project.work.join("emulator.log"))
            .map_err(|error| PistonError::WriteFileError(error.to_string()))?;
        self.emulator = Some(
            Command::new(emulator)
                .args(["-avd", &name, "-port", &port.to_string()])
                .stdout(log.try_clone().map_err(|error| PistonError::Generic(error.to_string()))?)
                .stderr(log)
                .spawn()
                .map_err(|error| PistonError::Generic(error.to_string()))?,
        );
        Ok(format!("emulator-{port}"))
    }

    fn wait_for_boot(&mut self) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(180) {
            if self.interrupted.try_recv().is_ok() {
                return Err(PistonError::Generic("Android development interrupted.".into()));
            }
            if let Some(child) = &mut self.emulator {
                if child
                    .try_wait()
                    .map_err(|error| PistonError::Generic(error.to_string()))?
                    .is_some()
                {
                    return Err(PistonError::Generic(format!(
                        "Android emulator closed before booting. See {}",
                        self.project.work.join("emulator.log").display()
                    )));
                }
            }
            let output = self
                .device_command()
                .args(["shell", "getprop", "sys.boot_completed"])
                .output()
                .map_err(|error| PistonError::Generic(error.to_string()))?;
            if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "1" {
                return Ok(());
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        Err(PistonError::Generic("Android did not finish booting within three minutes.".into()))
    }
}

impl Drop for AndroidDevelopment {
    fn drop(&mut self) {
        if let Some(emulator) = &mut self.emulator {
            if emulator.try_wait().ok().flatten().is_none() {
                if let Err(error) = emulator.kill() {
                    eprintln!("Cannot stop Android emulator: {error}");
                }
            }
            if let Err(error) = emulator.wait() {
                eprintln!("Cannot wait for Android emulator: {error}");
            }
        }
    }
}
