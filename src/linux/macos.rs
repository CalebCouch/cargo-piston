use cargo_metadata::Message;
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item, Value};
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::{Duration, Instant};
use crate::Result;
use crate::cli::Error;
use crate::dev::DevelopmentArgs;
use crate::host::Host;
use crate::manifest::Manifest;
use crate::project::Project;

pub mod cli;

pub struct LinuxDevelopment {
    project: Project,
    lima: PathBuf,
    home: PathBuf,
    name: String,
    identity: String,
    guest: PathBuf,
    app_args: Vec<String>,
    interrupted: Arc<AtomicBool>,
    owned: bool,
    desktop: Option<Child>,
    viewer: Option<Child>,
    _lock: File,
}

impl LinuxDevelopment {
    pub fn new(args: DevelopmentArgs) -> Result<Self> {
        let project = Project::new(args.application, "linux", false)?;
        let manifest = Manifest::new(project.package.manifest_path.as_std_path())?;
        let settings = manifest.settings(&["linux"])?;
        let identity =
            format!("{:x}", Sha256::digest(project.root().as_os_str().as_encoded_bytes()))[..16]
                .to_owned();
        let name =
            args.vm.or(settings.get("vm").cloned()).unwrap_or_else(|| format!("piston-{identity}"));
        Self::validate_name(&name)?;
        if args.lima_home.is_none() {
            if let Some(value) = manifest.setting(&["linux", "lima_home"])? {
                let path = value.as_str().ok_or_else(|| cli::Error::ManifestStringRequired {
                    path: manifest.path().to_owned(), key: "linux.lima_home".into(),
                })?;
                Self::validate_home(Path::new(path))?;
            }
        }
        let lima = match args.lima {
            Some(path) => {
                Project::output(Command::new(&path).arg("--version"))?;
                path
            }
            None => match settings.get("lima").map(PathBuf::from).filter(|path| {
                Command::new(path).arg("--version").stdout(Stdio::null()).stderr(Stdio::null())
                    .status().is_ok_and(|status| status.success())
            }) {
                Some(path) => path,
                None => Host::linux_lima()?,
            },
        };
        let home = args
            .lima_home
            .or_else(|| settings.get("lima_home").map(PathBuf::from))
            .or_else(|| env::var_os("LIMA_HOME").map(PathBuf::from))
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".lima")))
            .ok_or_else(|| cli::Error::LinuxVmHomeMissing)?;
        let home = env::current_dir()?.join(home);
        fs::create_dir_all(&home)
            .map_err(|source| cli::Error::LinuxVmHomeAccess { path: home.clone(), source })?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(home.join(format!(".piston-{name}.lock")))
            .map_err(|source| cli::Error::LinuxVmHomeAccess { path: home.clone(), source })?;
        lock.try_lock().map_err(|source| cli::Error::LinuxVmLocked {
            name: name.clone(), source: Box::new(source),
        })?;
        let interrupted = Arc::new(AtomicBool::new(false));
        let cancelled = interrupted.clone();
        ctrlc::set_handler(move || cancelled.store(true, Ordering::SeqCst))
            .map_err(cli::Error::LinuxVmSignalHandler)?;
        let development = Self {
            project,
            lima,
            home,
            name,
            identity,
            guest: PathBuf::new(),
            app_args: args.app_args,
            interrupted,
            owned: false,
            desktop: None,
            viewer: None,
            _lock: lock,
        };
        manifest.save(
            &["linux"],
            &[
                ("vm", development.name.clone()),
                ("lima", development.lima.to_string_lossy().into_owned()),
                ("lima_home", development.home.to_string_lossy().into_owned()),
            ],
        )?;
        Ok(development)
    }

    pub fn validate_name(name: &str) -> std::result::Result<(), cli::Error> {
        if !name.starts_with(|c: char| c.is_ascii_alphanumeric())
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        { return Err(cli::Error::LinuxVmNameInvalid { name: name.into() }); }
        Ok(())
    }

    pub fn validate_home(path: &Path) -> std::result::Result<(), cli::Error> {
        if !path.is_dir() { return Err(cli::Error::LinuxVmHomeInvalid { path: path.into() }); }
        tempfile::tempfile_in(path).map(|_| ())
            .map_err(|source| cli::Error::LinuxVmHomeAccess { path: path.into(), source })
    }

    pub fn name(&self) -> &str { &self.name }
    pub fn owns_vm(&self) -> bool { self.owned }

    pub fn viewer_needs_build(&self) -> bool {
        fs::read_to_string(self.project.work.join("viewer.swift")).ok().as_deref()
            != Some(include_str!("viewer.swift"))
            || !self.viewer_binary().is_file()
    }

    pub fn compile_viewer(&self, developer: &Path) -> Result<()> {
        let source = self.project.work.join("viewer.swift");
        let binary = self.viewer_binary();
        let pending = self.project.work.join("viewer-pending.swift");
        let pending_binary = binary.with_extension("pending");
        Project::write(&pending, include_str!("viewer.swift"))?;
        fs::create_dir_all(binary.parent().unwrap())
            .map_err(|error| Error::WriteFileError(error.to_string()))?;
        self.run(
            Command::new("xcrun")
                .env("DEVELOPER_DIR", developer)
                .args(["swiftc", "-O", "-framework", "AppKit", "-framework", "WebKit"])
                .arg(&pending)
                .arg("-o")
                .arg(&pending_binary),
        )?;
        fs::rename(&pending_binary, &binary)
            .map_err(|error| Error::WriteFileError(error.to_string()))?;
        fs::rename(&pending, &source)
            .map_err(|error| Error::WriteFileError(error.to_string()))?;
        Ok(())
    }

    pub fn prepare_viewer(&self, name: &str) -> Result<()> {
        let network = plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("NSAllowsLocalNetworking", plist::Value::Boolean(true)),
        ]));
        let info = plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("CFBundleExecutable", plist::Value::String("PistonLinux".into())),
            ("CFBundleIdentifier", plist::Value::String("dev.piston.linux-viewer".into())),
            ("CFBundleName", plist::Value::String(name.into())),
            ("CFBundlePackageType", plist::Value::String("APPL".into())),
            ("NSAppTransportSecurity", network),
        ]));
        let mut contents = Vec::new();
        info.to_writer_xml(&mut contents)
            .map_err(|error| cli::Error::WritePlUtilError(error.to_string()))?;
        Project::write(&self.project.work.join("Piston Linux.app/Contents/Info.plist"), contents)
    }

    pub fn start_vm(&mut self) -> Result<()> {
        let output = Project::output(self.lima().args(["list", "--json"]))?;
        let instances = String::from_utf8_lossy(&output)
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if let Some(instance) =
            instances.iter().find(|instance| instance["name"].as_str() == Some(&self.name))
        {
            if instance["status"].as_str() != Some("Stopped") {
                return Err(cli::Error::LinuxVmUnavailable {
                    name: self.name.clone(), state: instance["status"].as_str().map(str::to_owned),
                }.into());
            }
        } else {
            self.run(
                self.lima().args([
                    "create",
                    "--tty=false",
                    "--name",
                    &self.name,
                    "--vm-type=vz",
                    "--cpus=2",
                    "--memory=4",
                    "--disk=24",
                    "--mount-none",
                    "--containerd=none",
                    "--image-variant=minimal",
                    "template:ubuntu-24.04",
                ]),
            )?;
        }
        self.owned = true;
        self.run(self.lima().args(["start", "--tty=false", &self.name]))
    }

    pub fn prepare_guest(&mut self) -> Result<()> {
        let output = Project::output(&mut self.shell("printf '%s' \"$HOME\""))?;
        let home = String::from_utf8(output)?;
        self.guest = PathBuf::from(home).join(".local/share/piston").join(&self.identity);
        self.run(&mut self.shell(&format!(
            "mkdir -p {} \"$HOME/.cache/piston\"", Self::quote(&self.guest)
        )))?;
        let setup = self.project.work.join("setup.sh");
        Project::write(&setup, include_str!("setup.sh"))?;
        self.copy(&setup, &self.guest.join("setup.sh"))?;
        self.run(&mut self.shell(&format!("bash {}", Self::quote(&self.guest.join("setup.sh")))))
    }

    pub fn sync(&self) -> Result<()> {
        let metadata = self.project.dependencies()?;
        let mut roots = BTreeSet::from([self.project.workspace.clone()]);
        let mut manifests = BTreeSet::from([self.project.workspace.join("Cargo.toml")]);
        for package in metadata.packages.iter().filter(|package| package.source.is_none()) {
            let manifest = package.manifest_path.clone().into_std_path_buf();
            roots.insert(manifest.parent().unwrap().to_owned());
            manifests.insert(manifest);
        }
        let roots: Vec<_> = roots
            .iter()
            .filter(|root| !roots.iter().any(|parent| parent != *root && root.starts_with(parent)))
            .cloned()
            .collect();
        let archive = self.project.work.join("source.tar");
        let mut tar = Command::new("tar");
        tar.env("COPYFILE_DISABLE", "1")
            .args(["--no-xattrs", "-cf"])
            .arg(&archive)
            .args([
                "--exclude=.git", "--exclude=node_modules", "--exclude=.DS_Store", "--exclude=target",
            ])
            .arg(format!(
                "--exclude={}",
                self.project.target_dir.strip_prefix("/").unwrap().display()
            ))
            .args(["-C", "/"]);
        for root in &roots { tar.arg(root.strip_prefix("/").unwrap()); }
        self.run(&mut tar)?;
        let source = self.guest.join("source");
        let guest_archive = self.guest.join("source.tar");
        self.run(&mut self.shell(&format!(
            "rm -rf {0} && rm -f {1} && mkdir -p {0}",
            Self::quote(&source), Self::quote(&guest_archive),
        )))?;
        self.copy(&archive, &guest_archive)?;
        self.run(
            &mut self.shell(&format!(
                "tar -xf {0} -C {1} && rm -f {0}",
                Self::quote(&guest_archive), Self::quote(&source),
            )),
        )?;
        fs::remove_file(&archive)
            .map_err(|source| Error::RemoveFileError { path: archive, source })?;
        let config_roots: BTreeSet<_> =
            manifests.iter().map(|path| path.parent().unwrap().to_owned()).collect();
        for root in config_roots {
            for config in [root.join(".cargo/config.toml"), root.join(".cargo/config")] {
                if config.is_file() { manifests.insert(config); }
            }
        }
        for manifest in manifests {
            let original = fs::read_to_string(&manifest)?;
            let mut document = original.parse::<DocumentMut>()?;
            Self::relocate(document.as_item_mut(), &source);
            let rewritten = document.to_string();
            if rewritten != original {
                let overlay = self.project.work.join("manifest-overlay.toml");
                Project::write(&overlay, rewritten)?;
                self.copy(&overlay, &self.guest_path(&manifest))?;
            }
        }
        Ok(())
    }

    pub fn compile<F: FnMut(&str)>(&mut self, mut report: F) -> Result<PathBuf> {
        let version = Project::output(&mut self.shell(". \"$HOME/.cargo/env\"; rustc -vV"))?;
        let target = String::from_utf8_lossy(&version)
            .lines()
            .find_map(|line| line.strip_prefix("host: "))
            .map(str::to_owned)
            .ok_or(cli::Error::LinuxVmTargetMissing)?;
        if self.project.args.cargo.target.as_ref().is_some_and(|requested| requested != &target) {
            return Err(cli::Error::LinuxVmTargetMismatch {
                actual: target, requested: self.project.args.cargo.target.clone().unwrap(),
            }.into());
        }
        let mut args = self.project.args.cargo.clone();
        args.manifest_path =
            Some(self.guest_path(self.project.package.manifest_path.as_std_path()));
        args.target_dir = Some(self.guest.join("target"));
        args.target = Some(target.clone());
        let mut cargo = Command::new("cargo");
        cargo.args(["build", "--message-format=json-render-diagnostics"]);
        args.apply(&mut cargo);
        cargo.args(["--jobs", "2"]);
        if self.project.args.locked { cargo.arg("--locked"); }
        if self.project.args.offline { cargo.arg("--offline"); }
        let command = std::iter::once(cargo.get_program())
            .chain(cargo.get_args())
            .map(|arg| Self::quote(Path::new(arg)))
            .collect::<Vec<_>>()
            .join(" ");
        let mut command = self.shell(&format!(
            ". \"$HOME/.cargo/env\"; cd {}; unset RAMP_NOTIFICATION_APPLICATION; exec {command}",
            Self::quote(&self.guest_path(self.project.root()))
        ));
        let log = self.project.work.join("linux-build.jsonl");
        command.stdout(File::create(&log)?);
        let result = self.run(&mut command);
        let mut executable = None;
        for message in Message::parse_stream(BufReader::new(File::open(&log)?)) {
            match message {
                Ok(Message::CompilerArtifact(artifact))
                    if artifact.target.name == self.project.args.cargo.bin[0] =>
                {
                    executable =
                        artifact.executable.map(|path| path.into_std_path_buf()).or(executable);
                }
                Ok(Message::CompilerMessage(message)) => {
                    if let Some(rendered) = message.message.rendered { report(&rendered); }
                }
                Ok(Message::TextLine(line)) => report(&format!("{line}\n")),
                _ => {}
            }
        }
        result?;
        executable.ok_or_else(|| Error::ArtifactNotProduced {
            package: self.project.package.name.to_string(), target: Some(target),
        }.into())
    }

    pub fn show(&mut self, executable: &Path, title_prefix: &str, close_label: &str, quit_label: &str) -> Result<()> {
        let port = TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())?.port();
        let mut random = [0_u8; 20];
        File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut random))?;
        let token = random[..16].iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        let password = random[16..].iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        let session = self.guest.join(format!("session-{token}"));
        self.run(&mut self.shell(&format!("mkdir -p {}", Self::quote(&session))))?;
        let script = self.project.work.join("desktop.sh");
        Project::write(&script, include_str!("desktop.sh"))?;
        self.copy(&script, &session.join("desktop.sh"))?;
        self.run(
            &mut self.shell(&format!(
                "umask 077; printf '%s\\n' '{}' > {}; printf '%s\\n' '{}' > {}",
                token,
                Self::quote(&session.join("token")),
                password,
                Self::quote(&session.join("password"))
            )),
        )?;
        let arguments = self
            .app_args
            .iter()
            .map(|arg| Self::quote(Path::new(arg)))
            .collect::<Vec<_>>()
            .join(" ");
        let command = format!(
            "cd {}; exec dbus-run-session -- bash {} {} {port} {} {arguments}",
            Self::quote(&self.guest_path(self.project.root())),
            Self::quote(&session.join("desktop.sh")),
            Self::quote(&session),
            Self::quote(executable)
        );
        self.desktop = Some(
            self.shell(&command)
                .spawn()
                .map_err(|source| Error::CommandSpawn {
                    program: self.lima.clone().into(), source,
                })?,
        );
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(1)).build();
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            self.check_interrupt()?;
            if let Some(status) = self
                .desktop
                .as_mut()
                .unwrap()
                .try_wait()
                .map_err(|source| Error::CommandWait {
                    program: self.lima.clone().into(), source,
                })?
            { return Self::status(status, self.lima.as_os_str()); }
            if agent
                .get(&format!("http://127.0.0.1:{port}/piston-session"))
                .call()
                .ok()
                .and_then(|response| response.into_string().ok())
                .is_some_and(|value| value.trim() == token)
            { break; }
            if Instant::now() > deadline {
                return Err(cli::Error::LinuxVmDesktopTimeout.into());
            }
            thread::sleep(Duration::from_millis(200));
        }
        self.viewer = Some(Command::new(self.viewer_binary())
            .env("PISTON_VIEWER_TITLE", format!("{title_prefix}{}", self.project.package.name))
            .env("PISTON_VIEWER_CLOSE", close_label)
            .env("PISTON_VIEWER_QUIT", quit_label)
            .env("PISTON_VIEWER_URL", format!(
                "http://127.0.0.1:{port}/vnc.html?autoconnect=true&resize=scale&password={password}"
            ))
            .stdin(Stdio::null()).spawn().map_err(|source| Error::CommandSpawn {
                program: self.viewer_binary().into(), source,
            })?);
        loop {
            self.check_interrupt()?;
            if let Some(status) = self
                .viewer
                .as_mut()
                .unwrap()
                .try_wait()
                .map_err(|source| Error::CommandWait {
                    program: self.viewer_binary().into(), source,
                })?
            { return Self::status(status, self.viewer_binary().as_os_str()); }
            if let Some(status) = self
                .desktop
                .as_mut()
                .unwrap()
                .try_wait()
                .map_err(|source| Error::CommandWait {
                    program: self.lima.clone().into(), source,
                })?
            { return Self::status(status, self.lima.as_os_str()); }
            thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn stop(&mut self) -> Result<()> {
        for child in [&mut self.viewer, &mut self.desktop].into_iter().flatten() {
            if !child.try_wait().ok().flatten().is_some() { let _ = child.kill(); }
            let _ = child.wait();
        }
        if !self.owned { return Ok(()); }
        for force in [false, true] {
            let mut command = self.lima();
            command.args(["stop", "--tty=false"]);
            if force { command.arg("--force"); }
            command.arg(&self.name);
            let mut child = command
                .spawn()
                .map_err(|source| Error::CommandSpawn {
                    program: self.lima.clone().into(), source,
                })?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(status) =
                    child.try_wait().map_err(|source| Error::CommandWait {
                        program: self.lima.clone().into(), source,
                    })?
                { if status.success() { self.owned = false; return Ok(()); } break; }
                if Instant::now() > deadline { let _ = child.kill(); let _ = child.wait(); break; }
                thread::sleep(Duration::from_millis(100));
            }
        }
        Err(cli::Error::LinuxVmStopFailed { name: self.name.clone(), home: self.home.clone() }.into())
    }

    fn lima(&self) -> Command {
        let mut command = Command::new(&self.lima);
        command.env("LIMA_HOME", &self.home);
        command
    }

    fn shell(&self, script: &str) -> Command {
        let mut command = self.lima();
        command.args(["shell", "--workdir=/tmp", &self.name, "bash", "-lc", script]);
        command
    }

    fn copy(&self, source: &Path, destination: &Path) -> Result<()> {
        self.run(self.lima().arg("copy").arg(source)
            .arg(format!("{}:{}", self.name, destination.display())))
    }

    fn run(&self, command: &mut Command) -> Result<()> {
        self.check_interrupt()?;
        let program = command.get_program().to_owned();
        let mut child = command.spawn()
            .map_err(|source| Error::CommandSpawn { program: program.clone(), source })?;
        loop {
            if self.interrupted.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return self.check_interrupt();
            }
            if let Some(status) = child.try_wait()
                .map_err(|source| Error::CommandWait { program: program.clone(), source })?
            { return Self::status(status, &program); }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn check_interrupt(&self) -> Result<()> {
        if self.interrupted.load(Ordering::SeqCst) {
            Err(cli::Error::LinuxVmInterrupted.into())
        } else {
            Ok(())
        }
    }

    fn status(status: ExitStatus, program: &OsStr) -> Result<()> {
        if status.success() { Ok(()) } else {
            Err(Error::CommandStatus { program: program.into(), status }.into())
        }
    }

    fn quote(path: &Path) -> String {
        format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
    }
    fn guest_path(&self, path: &Path) -> PathBuf {
        self.guest.join("source").join(path.strip_prefix("/").unwrap())
    }
    fn viewer_binary(&self) -> PathBuf {
        self.project.work.join("Piston Linux.app/Contents/MacOS/PistonLinux")
    }

    fn relocate(item: &mut Item, root: &Path) {
        if let Some(table) = item.as_table_like_mut() {
            for (key, value) in table.iter_mut() {
                if matches!(key.get(), "path" | "target-dir") {
                    if let Some(path) = value.as_str().filter(|path| Path::new(path).is_absolute())
                    {
                        *value = toml_edit::value(
                            root.join(path.trim_start_matches('/')).to_string_lossy().into_owned(),
                        );
                    }
                } else {
                    Self::relocate(value, root);
                }
            }
        } else if let Item::Value(Value::Array(array)) = item {
            for value in array.iter_mut() {
                if let Some(table) = value.as_inline_table_mut() {
                    for (key, value) in table.iter_mut() {
                        if key.get() == "path" {
                            if let Some(path) =
                                value.as_str().filter(|path| Path::new(path).is_absolute())
                            {
                                *value = Value::from(
                                    root.join(path.trim_start_matches('/'))
                                        .to_string_lossy()
                                        .into_owned(),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Drop for LinuxDevelopment {
    fn drop(&mut self) { if self.owned { let _ = self.stop(); } }
}
