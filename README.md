`cargo build` gives you a binary. Cargo Piston gives you a signed, installable app for every platform -- from one `Cargo.toml`.

# How to Use Cargo Piston

Cargo Piston builds and runs Rust applications on macOS and Linux hosts. The `dev`
command selects the host by default. On macOS it also supports iOS devices and
Simulator, Android devices and emulators, and a Linux desktop VM. On Linux it runs
native Linux applications and Android development with an installed SDK. Apple
development requires macOS. The older `build` and `run` commands retain their
separate packaging configuration, including Windows GNU builds from macOS.

Setup uses short steps with descriptive prompts and labeled links whose full URLs
remain visible. Open those links when needed; Piston does not automatically open
web pages or account/settings windows. Links are clickable in supported interactive
terminals and appear as plain text when output is redirected, `TERM=dumb`, or
`NO_COLOR` is set.

## Code organization

Destination modules (`ios.rs`, `macos.rs`, `android.rs`, `linux.rs`, and `windows.rs`)
own app packaging and deployment for that destination. `host/macos.rs` and
`host/linux.rs` expose matching host and toolchain APIs, selected at their module
declarations with `cfg`. These select the machine running Piston, not the app being
built, so a macOS host can still build Android and Windows GNU outputs.

Shared Apple signing operations live in `apple.rs`. App Store Connect
authentication and API requests live separately in `apple/app_store.rs`, used by
Apple signing and deployment. App Store screenshot preparation and upload live in
`apple/app_store/artwork.rs`
and `apple/app_store/release.rs`. Shared iOS app building lives in `ios/application.rs`;
`ios/development.rs` owns connected-device development and `ios/distribution.rs` owns
Xcode App Store archives, exports, and binary uploads. Rust modules use named files
such as `host.rs`, with child modules in the matching directory.

Start tracing a command in `src/cli.rs`. CLI files own argument parsing, prompts,
terminal messages, setup and recovery links, and the sequence of operations. Service
structures in the neighboring files perform builds, signing, device operations,
network requests, and persistence. Flow-specific `Error` types and their user-facing
wording live in the owning CLI files. Shared command, file, and project errors live in the root
`cli.rs`; Apple signing and App Store Connect have their own shared CLI errors.
Services propagate these concrete errors through the shared boxed `Result` without
merging them into a global error enum. Recovery inspects the relevant local error
type, and setup links remain available through nested error sources. Services do not prompt
or call back into CLI flows. Native tool and app
output can still stream directly to the terminal. Every CLI structure is named
`Cli` and is referenced through its owner, such as `ios::Cli`,
`ios::deployment::Cli`, or `host::macos::Cli`.
The shared `host::Cli::link` method formats terminal links using OSC 8 when enabled.

| Flow | CLI entry points under `src/` |
| --- | --- |
| Command dispatch and shared arguments | `cli.rs` |
| Developer identity creation, selection, backup, and restoration | `developer/cli.rs` |
| Development platform selection | `dev/cli.rs`, then `host/cli.rs` for the default host |
| macOS, iOS, Android development and older build/run commands | `macos/cli.rs`, `ios/cli.rs`, `android/cli.rs` |
| Linux development and packaging | `linux/cli.rs`, then `linux/macos/cli.rs` for the VM or `host/linux/cli.rs` for native runs |
| Windows GNU packaging | `windows/cli.rs` |
| Device listing | `devices/cli.rs` |
| App Store and TestFlight deployment | `ios/deployment/cli.rs` |
| Compile, archive, export, and upload an iOS store build | `ios/distribution/cli.rs` |
| App Store account setup and screenshots | `apple/app_store/setup/cli.rs`, `apple/app_store/release/cli.rs` |
| Shared Apple signing and App Store Connect errors | `apple/cli.rs`, `apple/app_store/cli.rs` |
| TestFlight status monitoring and compliance actions | `apple/app_store/testflight/cli.rs` |
| Signing-team selection and notification setup | `apple/xcode/cli.rs`, `apple/notifications/cli.rs` |
| Xcode selection, download, and component setup on macOS | `host/macos/apple/cli.rs` |

For example, `cargo piston deploy testflight` follows `cli.rs` →
`ios/deployment/cli.rs` → account setup → `ios/distribution/cli.rs` →
`apple/app_store/testflight/cli.rs`. A pending upload resumes monitoring directly.
The distribution CLI shows team selection, notification setup, compilation,
archiving, export, and upload in execution order.

`project.rs` handles Cargo project selection and builds. `macos/development.rs`
prepares local app bundles, `android/development.rs` builds and installs Maverick
APKs, and `ios/simulator.rs` manages Simulator runs. Linux VM operations live in
`linux/macos.rs`; the Swift viewer's window lifetime controls the VM.
`apple/xcode.rs` generates signing projects, `apple/notifications.rs` registers
notification credentials, and `apple/push.rs` prepares and verifies entitlements.
`developer.rs` owns the encrypted AIR developer contract, credential synchronization,
and local developer identity storage. `developer/cli.rs` owns its setup and backup flow.

Piston generates platform metadata, including `Info.plist`, into the app's build
output using project settings. Application runtime behavior belongs in Maverick.
The old bundled Android photo picker and its camera declarations remain commented
out. The Android development path packages Maverick's Gradle bridge instead.

## Installing Piston
Cargo piston is a global cargo crate. There are two ways you can install it. Choose option one unless you have a reason not to.

1. Globally install the crate directly with
`cargo install cargo-piston --version 0.5.8 --locked`

2. Or clone the source with `git clone https://github.com/CalebCouch/cargo-piston`
and install it with `cargo install --locked --path </path/to/cargo-piston>`.

The iOS development command below does not require a `.env` file. The older platform
build/run commands use the configuration described later in this document.

## Development

Select one platform per command. Omitting it runs the app on the host:

```sh
cargo piston dev                 # macOS on a Mac, Linux on Linux
cargo piston dev macos
cargo piston dev ios             # connected iPhone/iPad, otherwise iOS Simulator
cargo piston dev android         # connected Android device, otherwise emulator
cargo piston dev linux           # Linux desktop VM on a Mac, native on Linux
```

The command builds, installs where needed, and streams application output in the
terminal. `--simulator` explicitly selects an iOS Simulator or Android emulator;
`--device` selects a particular phone, simulator, or AVD. A running simulator or
emulator is reused when it matches the request. Only the selected platform starts.
An Android emulator started by the command closes when the session ends, fails, or
is interrupted with Ctrl-C. A previously running emulator stays open.

When multiple iOS devices match, an interactive terminal shows a numbered list of
names and UDIDs. Enter a number to select the device, or `q` to cancel. Scripts
should use `--device <name-or-UDID>` to select one without a prompt.

### macOS

`cargo piston dev` builds for the Mac's native architecture, creates a `.app`
bundle, and runs its executable with terminal output. Use
`--target x86_64-apple-darwin` or `--target aarch64-apple-darwin` to select an architecture.
Development builds use ad-hoc signing unless push notifications require an Apple
team and provisioning. Universal distribution builds remain a separate build path.

The root `icon.png`, `icon.jpg`, or `icon.jpeg` becomes the app icon. Optional
`bundle_id`, `info_plist`, `entitlements`, and `resources` settings belong under
`[package.metadata.piston.macos]`; paths are relative to the application package.

### Android

Android development requires a `cdylib` application and a Maverick dependency
containing its `android/` Gradle bridge. Use `maverick_os::start!` or `ramp::run!`
for the application entry point so the library exports Android's entry function.
Piston generates the app manifest and
Gradle packaging project in `target/piston/android`; the activity, permission
handling, and other runtime code come from Maverick. Piston selects the device's
supported Rust ABI, builds the library with cargo-ndk, signs a debug APK, installs
it, launches it, and streams its logcat output.

```sh
cargo piston dev android
cargo piston dev android --simulator
cargo piston dev android --device emulator-5554 --target aarch64-linux-android
```

Piston reuses Android Studio's SDK and Java installation, installs missing SDK
components and Rust tooling, and creates an AVD if none exists. SDK license prompts
remain interactive. On macOS, missing command-line tooling can be installed using
Homebrew. Set `ANDROID_HOME` and `JAVA_HOME`, or optional `sdk`, `java`, and `ndk`
paths in `[package.metadata.piston.android]`, to use other installations. An optional
`bundle_id` in that table selects the Android application ID. Connected phones need
USB debugging enabled and this computer approved on the device.

Android push-account setup and store distribution are not part of this development
path. Apple push setup below applies to macOS and physical iOS devices; the iOS
Simulator path builds without configuring remote push notifications.

### Linux

On macOS, `cargo piston dev linux` uses Lima and a dedicated desktop window. Piston
installs Lima through Homebrew when needed, creates an Ubuntu VM, installs the
guest build and desktop dependencies, copies the project and local dependencies,
and builds inside Linux. The default VM uses two CPUs, 4 GiB RAM, and a sparse
64 GiB disk. This is a capacity limit; the disk consumes host space as data is written.
Setup provides 2 GiB of disk-backed swap for build memory peaks.
Rust `target` directories are excluded from source copies at every depth.
Linux Cargo artifacts are stored on the host under
`target/piston/linux/<package>/target`, respecting Cargo's configured target directory
and `--target-dir`. Piston mounts only this cache directory writable at
`/mnt/piston-target`; compilation still runs in Linux. Keeping it separate avoids
mixing Linux and macOS build scripts and proc macros. After stopping development,
run `cargo clean` in the host project to remove the Linux artifacts too.

Existing VMs keep their configured disk capacity. If a build reports `No space left
on device`, check both the Mac's free space and the VM's disk capacity. With the VM
stopped, grow its disk using the saved `lima` executable and `vm` name:

```sh
limactl edit --tty=false --disk=64 <vm-name>
```

Set `LIMA_HOME` to the project's saved `lima_home` value if it points outside the
default `~/.lima`. Choose a larger size if the VM already has a 64 GiB disk. Growing the disk
preserves its build cache and application data; rerun `cargo piston dev linux`
afterward.

Closing the window, stopping with Ctrl-C, or a build/run failure shuts down the
VM started by that command. Shutdown removes the transferred sources, transfer
archives, and desktop-session files, then trims the guest root filesystem to return
freed disk blocks to the Mac. The host build cache and application data are preserved.
Guest cleanup failures are reported, and Piston still attempts to stop the VM.

Only one Piston Linux VM session may run on a Mac, including across projects,
login accounts, `--vm` selections, and `--lima-home` directories. A second command
asks you to stop the active session. Piston also refuses to start while another
Lima VM is running, including one left running after a previous Piston process
was killed. It does not take over or stop that VM. The lock at
`/Users/Shared/.cargo-piston-linux-vm.lock` is released when the process exits;
do not delete its file while a command is running.
An existing stopped Lima VM can be selected explicitly:

```sh
cargo piston dev linux --vm my-linux-vm
cargo piston dev linux --lima /path/to/limactl --lima-home /path/to/lima-state --vm my-linux-vm
```

The selected `vm`, `lima`, and `lima_home` are saved under
`[package.metadata.piston.linux]` in Cargo.toml. VM builds use the guest's native
Rust target; an explicit `--target` must match it. Guest
setup currently targets Ubuntu/Debian package tooling. The desktop uses software
rendering, with no GPU passthrough. On a Linux host, the command builds and runs
locally instead of starting a VM.

### Push notification setup

Run the ordinary development, deployment, or macOS build command:

```sh
cargo piston dev
cargo piston dev ios
cargo piston deploy testflight
cargo piston build --target macos
```

During setup, Piston asks **Does this app use push notifications?** if the app has
no valid saved notification choice. Both yes and no are saved in
Cargo.toml, so later commands do not ask again. Answering no continues the normal
build or deployment without notification setup. Answering yes starts the setup
below, then continues the original command. A missing or invalid choice asks again while preserving any existing notification registration.
There is no `--notifications` flag.

For apps that use notifications, Piston asks for an **APNs** key ID or downloaded
`.p8` path and shows a link to Apple's
[Keys page](https://developer.apple.com/account/resources/authkeys/list).
Choose Apple Push Notification service when creating the key, and configure its
topic and environment scope for the app and selected Apple team. App Store Connect
API keys do not grant APNs access. A filename such as `AuthKey_ABCDEFGHIJ.p8`
supplies the key ID; entering an ID searches Spotlight for that exact filename.
The selected key must remain outside the project and repository. Apple allows
downloading each private key only once.

After the initial import, ordinary builds retrieve the APNs credentials from the
team's encrypted AIR developer contract; the source `.p8` file is no longer needed.
To change whether the app uses notifications, edit `enabled` in
`[package.metadata.piston.notifications]`. Unattended builds require a saved choice,
the team's local identity, and an existing AIR configuration. The old `key_path` and
`key_id` manifest fields are not used to configure or rotate APNs credentials.
Development and production have separate settings; configuring development does
not assume that its APNs key also permits production. iPhone development uses the
sandbox even with `--release`; TestFlight and App Store exports use production.
For macOS, development builds use the sandbox and distribution exports use production.
Piston checks each imported or saved APNs key against the environment being used before
accepting it. Deployment requires production access; sandbox-only keys are valid only
for development. A key supporting both environments can be used for either. Rejected
keys reopen interactive setup, while unattended commands stop before building or uploading.
App Store Connect credentials are separately checked against its production API.

To import replacement APNs credentials for an enabled app, use:

```sh
cargo piston dev ios --setup
cargo piston dev macos --setup
cargo piston deploy testflight --setup
```

Development setup replaces the development APNs credentials; iOS requires a physical
device. Deployment setup also replaces App Store Connect credentials and replaces
enabled production APNs credentials before preparing the application build.
`--prepare-only` also checks and configures enabled production notifications. Resuming
TestFlight checks the saved production key again; `--screenshots-only` does not use APNs.

Piston stores APNs keys and notification registration state in the selected team's
encrypted AIR developer contract. Each app, platform, and environment has its own
credential entry; registration snapshots preserve pending requests and AIR Inbox
sharing progress across interrupted runs. The developer contract also stores the
team's App Store Connect key and its IDs. These credentials are retrieved from AIR
when needed, rather than cached in Piston's own Keychain item.

### Developer identities

Piston selects the project's developer identity before starting build, development,
or deployment tools, including Android/Linux development and iOS Simulator runs.
If the project has no selection, a menu lists saved Team IDs and **Create a team identity**.
Piston saves the chosen team and public identity in the selected package's Cargo.toml
for later runs. When creating an identity needs an Apple Team ID, Piston shows a link to your
[Apple Developer account](https://developer.apple.com/account#MembershipDetailsCard)
and asks you to copy and paste the ID. Open the link, sign in, and select your team,
then click **Membership details** near the top of the page. Apple may reset the
section link while loading. Creating an identity does not create an Apple account
or signing certificate. Selecting an identity is local; AIR access starts when
credentials are needed. Help, version, and device-listing commands skip identity setup.

Plain `cargo piston` resolves the current Cargo project and uses its saved selection
or opens the menu. `cargo piston setup` always opens that menu so the current project
can select another identity or create one. Restore the existing identity when
joining a team: restoration is an explicit command, not a paste prompt in the menu.
If a project's saved identity is missing locally, Piston reports the restore command.

```sh
cargo piston                                    # use or select this project's identity
cargo piston setup                              # reselect or create this project's identity
cargo piston backup                             # choose an identity and copy its backup line
cargo piston restore "piston-identity:<data>"     # restore the complete quoted backup line
```

`backup` and `restore` also work outside a Cargo project. Restore only adds the
identity to local storage; choose it for the project at the next startup or through
`cargo piston setup`.

Missing or invalid Piston metadata reopens only the affected setup step. Valid values
and unrelated Cargo settings remain intact. Identity selection returns to the menu;
notification choices are asked again; saved app settings and paths can be corrected
or explicitly removed. Notification references are rebuilt from AIR, and unusable
TestFlight receipts trigger a fresh lookup. An unreadable TOML document or invalid
Cargo package definition must be corrected before Cargo can load the project.

Local identity secrets are stored in `~/.cargo/piston/identities.json`, or
`$CARGO_HOME/piston/identities.json` when `CARGO_HOME` is set. The directory has
owner-only permissions (`0700`) and the identity file has owner-only read/write
permissions (`0600`). This local file is not encrypted; it is the private identity
needed to open the encrypted developer contract. A file lock prevents simultaneous
local setup commands from overwriting each other's identity changes.

The backup menu lists Apple Team IDs. Selecting one prints
a single `piston-identity:` line containing that team's private identity. Keep a
private backup and share it only with trusted teammates: it grants full authority
over every app and stored credential using that identity. It is not
permission-limited delegation. Other teams' identities are not included. Restoring
the identity lets another computer retrieve the team's credentials from AIR;
Xcode's Apple Account access and signing certificates remain configured separately.

Restore validates the identity and its team before saving it in the local identity
file. Restoring the same identity again is safe; a different identity for an existing
team is rejected. A missing or invalid project selection returns to the identity menu.
AIR contract locations are derived from the identity and their initial settings, so
restoring the same identity accesses the same developer and notification contracts
on another computer.

Piston does not read or automatically migrate its previous Keychain storage. Use
`cargo piston restore "piston-identity:<data>"` with an explicit backup to retain an
existing identity. The backup is a required command argument; there is no paste
prompt. Apple credentials that have not been imported into the developer
contract are requested by their setup flow. Xcode still manages its signing
certificates and may show its own Keychain access prompts.

Cargo.toml stores the selected developer identity and public notification references:

```toml
[package.metadata.piston.developer]
team = "ABCDEFGHIJ"
identity = "<public AIR owner name>"

[package.metadata.piston.notifications]
enabled = true

[package.metadata.piston.notifications.ios.development]
team = "ABCDEFGHIJ"
topic = "org.example.app"
identity = "<public AIR owner name>"
application = "<public AIR application ID>"
```

Production settings use `notifications.ios.production`; macOS uses
`notifications.macos.development` and `notifications.macos.production`.
macOS signing saves its team under `package.metadata.piston.macos.team` and accepts
`bundle_id` and an optional `entitlements` path in that table. When no macOS bundle
ID is specified, an existing `piston.ios.bundle_id` is reused before the older
macOS builder's default identifier.

Piston creates or opens an ordinary `air_notifications::NotificationApplication`
contract, waits for confirmed credential acceptance in AIR, and shares its private
location with the notification service using AIR Inbox. Interrupted setup resumes
the same application. Credential preflight authenticates an HTTP/2 request with the
selected key and app topic against the required APNs endpoint, using a nonexistent
device token. It accepts only Apple's `BadDeviceToken` response, following Apple's
[documented authentication-before-device-lookup flow](https://developer.apple.com/documentation/usernotifications/sending-notification-requests-to-apns).
Authentication, environment, topic, connection, and unexpected response failures block
setup. Failed imports do not replace saved credentials. This checks current access;
it does not verify delivery to a real device or protect against later key revocation.

Only the public ID is supplied as `RAMP_NOTIFICATION_APPLICATION` to the actual
Cargo compilation, including both architectures in universal macOS builds.
RAMP reads it with `option_env!`, so changing the ID invalidates the relevant Cargo
build cache. Piston depends on AIR and `air_notifications`, not the UI stack.

Notification-enabled builds request Push Notifications in the generated Xcode
project and preserve custom entitlements. Piston verifies the final signature,
embedded profile, team, topic, expiry, and APNs environment before installing or
returning the app. iOS distribution checks the exported IPA. macOS notifications
use Xcode automatic provisioning even when the older `.env` signing fields are
absent; distribution exports retain an archive and installer in `target/piston/macos`.
Mac App Store exports enable App Sandbox and outgoing network access. Developer ID
exports retain the existing notarization step and its `DeveloperID-Notary` profile.

This setup requires an Apple team with Push Notifications access. App runtime
permission requests and per-installation push destinations remain in Maverick/RAMP.

### iPhone and iPad

From an application crate with a Rust binary target, run:

```sh
cargo piston dev ios
```

Piston selects the connected device, installs missing Rust target libraries, builds the
application, prepares an app bundle, asks Xcode to sign it for development, installs it,
and launches it with its standard output and standard error attached to your terminal.
Ctrl-C stops the console session and is forwarded to the app. Repeating the command
installs an update without uninstalling the app or erasing its container.
If the phone locks before launch, Piston offers a numbered menu: `1` retries after
unlocking without rebuilding or reinstalling, `2` cancels, and `3` switches to the
iOS Simulator. Switching rebuilds for the simulator and keeps the phone app installed.
Press Ctrl-C to stop at any time. Commands without an interactive terminal report
the locked device and exit instead of waiting for input.
Each run cleans Xcode's generated signing products before importing the Rust app,
so Xcode signs the copied executable again. Cargo's compilation cache is preserved.

This initial development path requires macOS, Xcode 15 or newer, and iOS/iPadOS 17 or
newer. Add your Apple Account in Xcode Settings > Accounts once, connect and unlock
your device and enable Developer Mode on the device. Piston starts pairing for an
unpaired USB phone; accept the Trust This Computer prompt on the phone. It verifies
the live development connection before building.
Piston handles Xcode first-launch setup, automatic provisioning, and device
registration using that account. Apple authentication, device trust, Developer Mode,
and any macOS administrator approval remain interactive Apple requirements. Full
Xcode installation requires Apple's installer; command-line tools alone are insufficient.

With one configured signing team and one connected device, no other configuration is
required. When several signing teams are available, Piston displays a numbered menu
with each team's name and ID. Enter its number to continue the same build, or `q` to
cancel. Piston saves the chosen team in `package.metadata.piston.ios.team` for later runs.
An explicit `--team` changes the saved team and skips the menu; scripts without
an interactive terminal must choose a team explicitly. Wired devices take priority
over wireless devices. If several devices remain, select one with `--device`:

```sh
cargo piston dev ios --device "My iPhone" --team ABCDE12345
cargo piston dev ios --target aarch64-apple-ios --release
cargo piston dev ios --manifest-path apps/example/Cargo.toml --bin example --features camera
cargo piston dev ios -- --example-app-argument
cargo piston dev ios --simulator
```

Physical devices use `aarch64-apple-ios`. If no device is connected, Piston opens an
iOS Simulator and uses `aarch64-apple-ios-sim` on Apple Silicon or `x86_64-apple-ios`
on Intel. Missing simulator runtimes are installed through Xcode. An explicit
simulator target also selects the simulator path. Catalyst and non-iOS targets are
rejected. Package, binary, features, profile, manifest, target directory, `--locked`,
and `--offline` options are passed through to Cargo. Piston reads the executable
path from Cargo's build output.

The default development bundle identifier is `dev.piston.<team>.<package>` so a
development installation can coexist with the store app. Use `--bundle-id` or optional
Cargo metadata to select another identifier and configure application capabilities:

```toml
[package.metadata.piston.ios]
# team = "ABCDE12345"
# bundle_id = "com.example.myapp.dev"
minimum_os = "17.0"
info_plist = "ios/Info.plist"
# entitlements = "ios/App.entitlements"
# resources = ["ios/LaunchScreen.storyboard", "ios/wordmark.png"]
```

`minimum_os` defaults to `17.0` and sets `IPHONEOS_DEPLOYMENT_TARGET` for compilation
as well as the bundle's minimum iOS version. Device, simulator, and store builds
link the executable with `/usr/lib/swift` and `@executable_path/Frameworks` runtime
search paths. Existing project Rust flags are preserved.

Paths in this metadata are relative to the selected package. Supply your app's privacy
descriptions, such as camera access, in the optional Info.plist. Identity, executable,
version, and minimum OS fields are resolved by Piston. Storyboards listed as resources
are compiled; PNG/JPEG resources also become named image assets in the same catalog as the app icon,
so launch storyboards can resolve them before the app runs. Other resources are copied into the bundle.
The resources list is optional:
fonts, images, and other data embedded by Maverick, Ramp, PTSD, or the application are
already in the executable and must not be listed again. Piston generates the OS app icon
separately from the root icon file. Embedded dynamic frameworks currently need their own
signing integration.

The iOS app version uses numeric `major.minor.patch` from `package.version`; Cargo
prerelease labels and build metadata are omitted. Store deployments automatically
choose an integer build number higher than the app's existing iOS builds and the
saved `package.metadata.piston.ios.build_number`. Piston saves it as a string in
`Cargo.toml` before building. Device development uses that saved build number, or
the numeric Cargo version before the first store deployment. iOS bundles support both
iPhone and iPad, and default to all interface orientations. An app can restrict orientations through
its supplied Info.plist.

Place `icon.png`, `icon.jpg`, or `icon.jpeg` beside the selected package's
`Cargo.toml`. Piston uses the first existing file in that order and compiles the
iPhone and iPad app icons into the generated bundle. No icon-path configuration
is needed. Without an icon file, the development command preserves any supplied icon
settings; otherwise the system uses its default app icon.

### Setup and recovery

Piston installs missing Rust target libraries, prepares Xcode components, starts USB
pairing, and requests automatic development signing and device registration. Apple
Account sign-in, verification codes, Trust prompts, and enabling Developer Mode need
your interaction. Piston reports the specific failure and the next steps in readable
terminal output. When no signing account is configured or Xcode rejects its login,
Piston displays a link to Xcode's Accounts settings and the manual navigation steps.
Open the link or go to Xcode > Settings > Accounts, complete sign-in, then rerun the
same command.

- **Apple Account rejected or expired:** open Xcode > Settings > Accounts, select the
  account for the listed team, and complete sign-in and verification. Confirm your team
  appears. If Xcode continues rejecting that account, remove and add the account again
  in Xcode. Credentials stay in Xcode; Piston does not collect them. A missing-profile
  message alongside rejected login is a consequence of the login failure.
- **Developer Mode is disabled or hidden:** after pairing, reopen Settings on the
  phone and scroll to the bottom of Privacy & Security. Enable Developer Mode, accept
  the restart, unlock the phone, and confirm Turn On. If the setting remains hidden,
  select the phone in Xcode's device manager and let Xcode prepare it.
- **Project folder access denied:** allow the terminal app access to the project in
  macOS System Settings > Privacy & Security > Files and Folders, or move the project
  outside a protected Documents/Desktop/Downloads folder.

Authentication is verified by Xcode when it requests signing assets. A team cached
in Xcode settings does not prove the account's login is still valid. Once sign-in is
working, Piston asks Xcode to create the development profile; manual profile setup
is normally unnecessary. Cargo reuses completed compilation on the next attempt.

Generated bundles, device diagnostics, and the signing project are placed under
`<cargo-target-dir>/piston/ios/<package>/`. Xcode runs in the background; signing
failures include recovery steps, relevant diagnostics, and the full log path. This
command performs a single build/install/launch cycle; automatic reload and store
submission are separate future workflows.

## Deploy iOS to App Store Connect

Deployment checks production credentials independently of whether the app uses push
notifications. Every exported IPA is checked for Apple distribution signing, the
selected team and bundle ID, a valid App Store profile, and a signing certificate
authorized by that profile. Development/ad-hoc/enterprise profiles and debugging
entitlements are rejected. APNs entitlements must be production when present;
iCloud containers must use Production when enabled. Xcode and App Store Connect
continue to validate signing and account permissions during export and upload.

`cargo piston dev` keeps development signing and sandbox notifications, including
when using `--release` for an optimized development build. `--prepare-only` checks
available account/APNs credentials; final certificate and profile checks require
an exported app. Resuming TestFlight rechecks production APNs access without
rebuilding the app Apple already received.

Run `cargo piston deploy` from the app crate. It validates the screenshot folders,
checks App Store access, prepares images, builds a release ARM64 iOS app, archives
and signs it with Xcode, and uploads its screenshots and binary to App Store Connect.
No connected phone is used. Xcode may require an existing development provisioning
profile to create the initial archive; a team with no registered devices receives
instructions to register a development iPhone once. Export then signs for App Store
distribution. Uploading does not submit the app for review or release it.

Keep the app icon next to `Cargo.toml` and screenshots in these conventional folders:

```text
Cargo.toml
icon.png
store/
  apple/
    iphone/
      01.png
      02.png
    ipad/
      01.png
```

PNG, JPG, and JPEG inputs are accepted. Both families are required because Piston's
iOS apps support iPhone and iPad. Put one to ten screenshots in each folder, using
zero-padded filenames to select display order. Use screenshots from one supported display
size per folder so each family forms a consistent gallery. This first implementation updates
the app's primary App Store language; other localizations are preserved.

Piston reads Apple's current screenshot specifications on each deployment. It keeps
accepted dimensions or downsizes to an accepted size with exactly the same aspect
ratio. It does not stretch, crop, or upscale the app's UI to invent another device
layout. Capture the actual iPhone and iPad layouts. Invalid images produce a message
with the source filename and accepted dimensions. Opaque alpha channels are removed;
images containing transparent pixels need an opaque source. Generated RGB PNGs go
under `<cargo-target-dir>/piston/ios/<package>/store/screenshots/`; originals stay intact.
The root icon must be square, opaque, and at least 1024 pixels on each side for deployment.

Set the store bundle identifier once in the app's Cargo metadata:

```toml
[package.metadata.piston.ios]
bundle_id = "com.example.myapp"
# team = "ABCDE12345"
```

It must match an app record in App Store Connect. The default development identifier
is never used for a store upload. Select the signing team from the existing menu,
set it in metadata, or pass `--team`. Use the team belonging to the API key below.

App Store Connect API access is separate from Xcode's saved Apple Account. Run your
deploy command in a terminal. When credentials are missing or invalid, Piston shows
a link to Apple's API-key page and guides you through setup. Enter the Issuer ID first,
then either the Team API Key ID or the downloaded `.p8` file's path. Piston finds files
by Key ID or infers the Key ID from an `AuthKey_<KEY_ID>.p8` filename. Renamed files
need the Key ID entered manually.
The Issuer ID comes from Apple's Team Keys page: it cannot be derived from the private
key. Piston checks API access and stores the key, Key ID, and Issuer ID in the selected
team's encrypted AIR developer contract for later runs.
Enter `q` to cancel. For automatic signing and uploads, create a
Team Key with Admin access; the Account Holder may first need to enable API access.
Store distribution requires Apple Developer Program membership.

On that same **Team Keys** page, click **Download API Key** beside the key you created.
The downloaded file is normally named `AuthKey_<KEY_ID>.p8`; check your browser's
downloads for its location and enter that file's path at Piston's prompt, for example
`~/Downloads/AuthKey_<KEY_ID>.p8`. Apple permits only one download. If the link is
missing, locate the previously downloaded file. If it is lost, create a new Team Key
and restart Piston setup with its new Key ID and file.

On macOS, Piston searches Spotlight by the exact key filename, including indexed
locations outside the usual folders. It also checks `private_keys/` beside the selected
package, `~/.appstoreconnect/private_keys/`, `~/.private_keys/`, `~/private_keys/`, and
`~/Downloads/` directly, so a recent download does not have to wait for indexing.
Duplicate results for the same canonical file are combined. Multiple different matches
produce a selection menu; scripts must set `ASC_KEY_PATH` to choose one. Discovery
uses filenames, and the selected file and IDs are verified against Apple before saving.
Spotlight cannot find excluded, unindexed or inaccessible files; entering a path still
works for an accessible file in an unindexed folder. Piston does not alter search privacy
settings. After import, later deployments retrieve the key from AIR without searching
for the original file.

Piston writes public project references to the selected package's `Cargo.toml`:

```toml
[package.metadata.piston.ios]
team = "ABCDE12345"

[package.metadata.piston.developer]
team = "ABCDE12345"
identity = "<public AIR owner name>"
```

The key's PEM contents and Apple API identifiers are stored in AIR under the
selected team's developer identity. No source `.p8` path is saved in the manifest.
Keep downloaded private keys outside the repository. Use `--setup` on the deploy
command to import replacement App Store Connect credentials for the selected team.
Existing AIR credentials take precedence over import environment variables on normal
runs. Xcode receives a temporary `.p8` file for archive, export, and upload; Piston
removes that temporary file when the deployment operation releases it, including
normal error returns. It does not delete the file originally downloaded from Apple.

Signing-team selections are saved as `package.metadata.piston.ios.team`.
An explicit `--team` changes that saved choice after selection. Successful TestFlight
uploads save `app_id`, `version`, `build_number`, and `state` in
`package.metadata.piston.testflight`, so deployment can resume even after
`cargo clean`. The legacy Apple signing path separately saves its `team` and
`identity` in `package.metadata.piston.apple.signing`.

Old account settings and receipt files are not read or migrated by the modern
deployment flow. Run deployment in an interactive terminal to import credentials
again when no AIR entry exists. Updates preserve unrelated manifest settings,
comments, and formatting, and write the selected package's manifest even when
using `--package` from a workspace. Build products, downloaded tools, diagnostics,
and provisioning files remain build artifacts. Xcode's certificates and Apple
Account credentials stay managed by Xcode. Xcode is discovered on each run.

For the initial credential import in CI, first provision the team's local developer
identity, then supply `ASC_ISSUER_ID` and either `ASC_KEY_ID` or `ASC_KEY_PATH` through
the environment. A standard filename supplies the missing Key ID; a unique search
match supplies the missing file path. Supplying all three remains supported.
Relative input paths resolve against the selected package, and an explicitly
supplied path takes precedence over search results. Piston validates and saves
these imported credentials in AIR; subsequent runs need the local identity and AIR
access, not those environment variables. The modern deployment flow does not read
`.env` for App Store Connect setup. Noninteractive runs report missing configuration
and exit instead of prompting.

Create the iOS app record in [App Store Connect → Apps](https://appstoreconnect.apple.com/apps)
with the same bundle identifier. Piston uses its primary language and the numeric
Cargo version. App Store deployment can create the draft version when necessary.

```sh
cargo piston deploy --manifest-path path/to/Cargo.toml
cargo piston deploy --prepare-only
cargo piston deploy --screenshots-only
```

`--prepare-only` performs read-only account/specification checks and prepares local
screenshots, without building, signing, or uploading. It still requires API access.
`--screenshots-only` updates the screenshot gallery for the Cargo version without
uploading a binary. The normal command uploads both. Cargo package, binary, feature,
manifest, target-directory, `--locked`, and `--offline` options are supported; offline
applies to Cargo dependency resolution, while App Store operations need network access.

Images already uploaded by Piston are reused by content hash. Piston waits for image
processing before changing gallery placements and verifies the resulting display order.
The local folders define the screenshot galleries selected for this deployment;
other languages, device groups, and app-preview videos remain untouched. Library
images are retained so existing placements can be restored if a replacement fails.

The App Store version must be editable. If it is approved or under review, create a
new version by increasing `package.version`, or resolve the pending review in App Store
Connect. An unreleased app version can have multiple uploaded builds; Piston assigns
a new build number without changing `package.version`. After uploading, complete the listing,
privacy and review information in App Store Connect, then select the processed build
and submit when ready. Piston never fabricates that information or submits automatically.

Before compiling a new upload, Piston checks Apple's iOS release and build history.
The Cargo version may equal an existing unreleased version. Piston stops if it is
equal to or below a previously released version, because Apple closes released
versions to new uploads. For an eligible version, Piston automatically increments
the build number, including past expired builds. Versions are compared numerically
(`1.10.0` is newer than `1.9.0`).
Monitoring an existing TestFlight upload still resumes normally, and screenshot-only
updates do not require a new binary version.

## Deploy to TestFlight

```sh
cargo piston deploy testflight
```

This iOS path builds a release app, archives and signs it for distribution, uploads
it to App Store Connect, and waits for Apple's TestFlight processing result. It uses
the same bundle identifier, team, root icon, resources, and API-key configuration
as App Store deployment. App Store screenshots and an editable store listing are
not required or changed. One command targets iOS; it does not upload a macOS build.
`cargo piston deploy` and `cargo piston deploy app-store` retain the App Store workflow.

Uploads require Xcode 26 or later with the iOS 26 SDK or later under Apple's current
[upload requirements](https://developer.apple.com/news/upcoming-requirements/?id=04282026a).
Piston checks the toolchain and API-key configuration before compiling. Apple builds
automatically use the newest compatible installed Xcode, searching `/Applications`,
`~/Applications`, the current `xcode-select` location, and Spotlight. Selection checks
the app's declared macOS requirements and whether its tools run. `DEVELOPER_DIR`
remains an explicit override and may point to an `.app` or its `Contents/Developer`
directory. Invalid or too-old overrides produce an error rather than being ignored.
Piston passes the chosen path to its build commands without changing `xcode-select`.
Installed versions, including prereleases, can be selected; downloadable versions
are stable releases only.

If no suitable Xcode is installed, Piston checks the community-maintained
[Xcode Releases catalog](https://xcodereleases.com/) for the newest stable version
whose macOS, CPU, and SDK requirements fit. It prefers the Apple silicon installer
on Apple silicon and a Universal installer on Intel. Download URLs must point to
Apple's official developer downloads. There is no fixed fallback version. Catalog
access is needed only for Xcode setup; using an installed Xcode does not require
that catalog. AIR-backed credentials and Apple deployment services still require
network access.

In an interactive terminal, Piston displays a link to the exact `.xip` download.
Open the link and download the file. Piston waits for the completed file in Downloads
or Spotlight, extracts it with Apple's `xip`, verifies Apple's app signature and the
expected version/build, and installs it alongside
existing versions in `~/Applications/Xcode-<version>-<build>.app`. It then selects
that Xcode for the current command, prepares its first-launch components, and
continues the build. It does not overwrite existing applications or remove the
download. Extraction and verification logs remain in the temporary installation
directory; component logs are under `~/Library/Logs/Piston/Xcode`.

Apple sign-in may still be required in the browser. If license acceptance or
administrator approval is needed, Piston asks through Apple's command-line license
viewer or `sudo`, then continues. It never accepts license terms for you. Physical
device builds do not download optional simulator runtimes. Press Ctrl-C to cancel
while waiting or installing.

For a custom download location or noninteractive setup, set
`PISTON_XCODE_ARCHIVE=/absolute/path/to/Xcode.xip`. The archive must match the version
and build selected from the catalog. Noninteractive runs do not wait for a download
and report any license or administrator setup still required.
If this Mac cannot run a suitable release, Piston links to Software Update and explains
the minimum macOS version. See [Apple's compatibility table](https://developer.apple.com/xcode/system-requirements/).

```sh
cargo piston deploy testflight --prepare-only
cargo piston deploy testflight --manifest-path path/to/Cargo.toml --team ABCDE12345
cargo piston deploy testflight --setup
```

`--prepare-only` prepares account access and the toolchain without building or uploading.
The normal deploy command stays running until Apple marks the build ready for internal
TestFlight testing, or reports a failure. It prints each build/upload stage once,
then checks Apple's processing status every 15 seconds in the background. A new
status line, with elapsed time, appears only when that status changes. Xcode setup
also reports stage changes without repeated waiting messages. There is no processing
time limit and no separate status command to run.

After Ctrl-C or an interruption, rerun the same deploy command. Piston resumes the
pending upload saved in `Cargo.toml`, including the delay before Apple lists it.
The saved app version and build number identify the exact upload. Resuming it does
not require a current Xcode installation. Expired or invalid uploads are replaced
with a new build number on the next deployment.

After deployment completes, running the command again builds and uploads fresh code
with the same Cargo version and a new build number. To replace a pending upload
instead of resuming it, run `cargo piston deploy testflight --new-build`.
`--prepare-only` displays the selected version and build number without reserving it.

A processed build may still need export-compliance answers. Piston displays a link
to that exact build's TestFlight page once when manual answers are required. Open the
link and click **Provide Export Compliance Information** to answer Apple's questionnaire.
Piston keeps monitoring until Apple clears the build, including during Apple's
compliance review. Groups configured for automatic internal distribution receive
eligible builds automatically; otherwise select an Internal Testing group in App
Store Connect and add the build. External testers require beta information and
Apple's beta review. This command does not create testers, invite people, submit
an external beta review, or submit the app for public App Store review.

## Legacy platform build/run setup

The remaining sections document the older `build` and `run` paths and their
manual tool/signing configuration. They do not describe credential storage for
`cargo piston dev` or `cargo piston deploy`; those flows use the setup above.

Configure your `.env` and project `Cargo.toml` as shown below and install the desired
build targets via rustup.

For example if you want to build for MacOS devices you would want to add the following build targets:

`rustup target add aarch64-apple-darwin`

and

`rustup target add x86_64-apple-darwin`

## Example commands for using piston

Build an app bundle for a target architecture. This command will build a MacOS binary within a dynamically created app bundle derived from the information contained within your `Cargo.toml` and `.env`. This includes ordinarily tedious minutia such as an Info.plist and app icon configuration.

`cargo piston build --target aarch64-apple-darwin`

Optionally, users can specify a release flag for the build.

`cargo piston b --target aarch64-apple-darwin --release`

Run an App locally on the host machine

`cargo piston run`

List viable USB tethered mobile devices (iOS & Android)

`cargo piston list-devices`

Install and run an app over USB tether on the specified target device

`cargo piston run --device <deviceID>`

Cargo piston also supports generic targets which are dynamically determined based on the devices connected via usb and host system architecture. Cargo piston will make an automatic determination for the build target and device target with a generic argument.

For example...

`cargo piston build --target ios`

The above command will guess the appropriate ios build target based on host architecture. A full list of generic build targets includes...

```
ios
android
windows
linux
macos
```

`cargo piston run --device ios`

The above command will guess the appropriate ios device based on the reachable connected devices.

## Tested & Supported Build Targets

In theory this tool should support build targets for all of the supported operating systems, but they will only be added explicitly after being tested. If you test any of the unsupported targets in main.rs please open an Issue or Pull Request on the github repository.

### Windows

`x86_64-pc-windows-gnu`

### Android

`aarch64-linux-android`

`x86_64-linux-android`

### MacOS

`aarch64-apple-darwin`

`x86_64-apple-darwin`

### IOS

`aarch64-apple-ios`

`x86_64-apple-ios`

### Linux

`aarch64-unknown-linux-gnu`

`x86_64-unknown-linux-gnu`

# General Configuration

## Legacy .ENV configuration

The older `build` and `run` commands read their configuration from a `.env` file in
your project root — the directory you run `cargo piston` from.

Piston utilizes `cargo`, so it needs to be able to locate it. `cargo`'s file path needs to 
match the file path in the .env. If `cargo` is on your PATH, Piston can usually find it
automatically. Check with: 

`which cargo`

If that prints a path, cargo is on your PATH. If it prints nothing, set
`cargo_path` in the .env so piston knows where to look:

Replace `<username>` with your actual username (run `whoami` if you're unsure).

`cargo_path=/Users/<username>/.cargo/bin/cargo`

To verify, run that exact path with `--version`:

`/Users/<username>/.cargo/bin/cargo --version`

A printed cargo version means it's correct. "No such file or directory" means the
path is wrong — compare it against the output of `which cargo`.

## General Cargo.toml configuration 

```
name = "appname"
version = "0.0.1"
```

### App Icon example

Place `icon.png`, `icon.jpg`, or `icon.jpeg` in the package root beside `Cargo.toml`.
PNG takes precedence, followed by JPG and JPEG. The `icon_path` metadata setting
is no longer used. The older platform builders retain their placeholder when no
icon file is supplied.

### Including assets

```
[package.metadata]
assets_path = absolute/path/to/assets/directory
```

# Windows Output Configuration

## Install mingw-w64 via homebrew (required on MACOS only)

`brew install mingw-w64`

After installing mingw-w64, add the path to the linker to your global `~/.cargo/config.toml`.
Create the config.toml if it doesn't already exist, then write the following:

```
[target.x86_64-pc-windows-gnu]
linker = path/to/homebrew/bin/x86_64-w64-mingw32-gcc
```

## App Icon
You must have embed-resource in your `Cargo.toml` as a `[build dependency]`
```
[build dependency]
embed-resource = "3.0.2"
```

## Automated Signing

Not yet implemented for windows outputs

<!-- 
TODO can probably remove this, not sure if needed for linux host?
You should have your desired output filename designated in your Cargo.toml as 
```
[package.metadata.winres]
OriginalFilename = "<appname>.exe"
``` -->

# Linux Output Configuration

## Configure paths in .env (MACOS HOST ONLY)

`zigbuild_path=/Users/<username>/.cargo/bin/cargo-zigbuild`

`homebrew_path=/opt/homebrew/bin`

## Install Zig via homebrew (MACOS HOST ONLY)

Provide a path to your homebrew binaries (somewhere like `/opt/homebrew/bin`) in your `.env`

`homebrew_path=/opt/homebrew/bin`

## Install zigbuild (MACOS HOST ONLY)

`cargo install cargo-zigbuild`

## Automated Signing

Note: if you do not designate a signing key ID and password for your chosen output in the `.env`, automated signing will be skipped. See details in your output specific section.

Install GPG with brew (MacOS Host Only)

`brew install gnupg`

Install GPG with apt (Linux Host Only)

`sudo apt install gnupg2`

Configure the path to your gpg installation in your `.env` (this is an example, use your actual path)

`gpg_path=/opt/homebrew/bin/gpg`

Add the following line to your `~/.gnupg/gpg-agent.conf`

`allow-loopback-pinentry`

You can create this file and add the config option with a single terminal command as follows (ensure the correct path to your .gnupg is supplied)

`echo allow-loopback-pinentry > /Users/$USER/.gnupg/gpg-agent.conf`

Generate a keypair if you need one (Choose RSA [option 1], 2048 bits, key does not expire [option 0], provide and email and passphrase)

`gpg --full-generate-key`

A standard gpg brew installation will store public keys within a keybox file at `~/.gnupg/pubring.kbx` and private keys at `~/.gnupg/private-keys-v1.d/` within individual files for each key, these keys are encrypted with an optional passphrase. 

To find a key id from your gpg keyring run the following command
`gpg --list-secret-keys`

Configure your `.env` with your gpg key and passphrase

`linux_gpg_key_id=<key_id>`

`linux_gpg_key_pass=<passphrase>`

## Linux Release Options

Linux has a variety of options for building outputs depending on how you intend to release the software and what linux distributions you wish to support.

### AppImage

To build a linux appimage, which is the most versatile way to release linux software, you must call the `--release-appimage` flag on your piston build

Example

`cargo piston build --target linux --release-appimage`

Running this command requires you first obtain the proper runtime for your target architecture.

Please download the following runtimes. It is reccomended that you place these runtimes in $HOME/Linux-Runtimes. You must specify the full path to your runtimes directory by setting `linux_runtime_path=full/path/to/runtimes` in your `.ENV`.

Note: it is reccomended that you also download the `.asc` signature for each runtime and verify them with GPG. 

X86_64 (x86 64 bit)

`https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64`

aarch64 (ARM 64 bit)

`https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-aarch64`

<!-- armhf (ARM 32 Bit)

`https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-armhf`

i686 (x86 32 bit)

`https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-i686`

ppc64le

`https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-ppc64le`

riscv64

`https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-riscv64` -->

### Static Binary tarball

Not yet implemented

### snap bundle

Not yet implemented

### flatpak bundle

Not yet implemented

### native .deb or .rpm

Not yet implemented


# MacOS & IOS Output Configuration (MACOS HOST ONLY)

## Xcode setup

Piston discovers and uses the newest compatible installed Xcode for Apple builds.
When none is suitable, it downloads and prepares a compatible release as described
in [Deploy to TestFlight](#deploy-to-testflight). Use `DEVELOPER_DIR` to override its
selection. You do not need to change the system-wide `xcode-select` setting. License
acceptance and administrator approval, when required, are handled interactively.

## Legacy automated signing and device provisioning

The older Apple `build` and `run` paths can sign through the configuration below.
Modern `dev ios` and `deploy` use Xcode automatic provisioning and the earlier
setup instructions instead.

First, create an App Store Connect API key through your apple developer portal at the following URL. **This MUST be an admin key**. A developer key will not have sufficient privileges to access the device provisioning endpoints.

`appstoreconnect.apple.com/access/integrations/api`

After creating and downloading the API key, add your `asc_key_path` to the `.env`

When downloaded from apple developer portal, the key file will look like this:

`/Users/<user>/Downloads/AuthKey_1AB23CDEFG.p8`

`asc_key_path=path/to/authkey`

Also add the `asc_key_id` and the `asc_issuer_id` to the `.env`, these items are obtainable from `appstoreconnect.apple.com/access/integrations/api`. The `asc_issuer_id` can be found at the top of the key list, and the  The `asc_key_id` can be found in the row corresponding to the key you've chosen. This `asc_key_id` should match the filename of the key you've selected in the `asc_key_path`.

Lastly provide the full path to your MacOS keychain directory in the `.env`:

`keystore_path=/Users/<username>/Library/Keychains`

Eventually the rest of this should be automated, but for now...

Download the Apple Worldwide Developer Relations cert:

`curl -o /Users/<username>/Library/Keychains/AppleWWDRCA.cer https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer`

Add `AppleWWDRCA.cer` to the security chain:

`security import /Users/<username>/Library/Keychains/AppleWWDRCA.cer -k /Users/<username>/Library/Keychains/login.keychain-db`

Download the Apple Root CA cert:

`curl -o /Users/<username>/Library/Keychains/AppleRootCA.cer https://www.apple.com/certificateauthority/AppleRootCA-G3.cer`

Add `AppleRootCA.cer` to the security chain:

`security import /Users/<username>/Library/Keychains/AppleRootCA.cer -k /Users/<username>/Library/Keychains/login.keychain-db`

Download the Developer ID Authority Certificate:

`curl -o /Users/<username>/Library/Keychains/AppleDevIDCA.cer https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer`

Add `AppleDevIDCA.cer` to the security chain:

`security import /Users/<username>/Library/Keychains/AppleDevIDCA.cer -k /Users/<username>/Library/Keychains/login.keychain-db`

## Universal binary

Universal binaries are automatically created via lipo when you pass in the `--release` flag to a macos target build command. For example running the following command will automatically create a universal binary in the output bundle that will run on either arm64 or x86_64 architecture. The Apple App store requires MacOS binaries to be universal.

`cargo piston build --target aarch64-apple-darwin --release`

## External Release (Optional)

Distributing MacOS apps outside of the Apple App Store follows a different release process. This process cannot be fully automated via the ASC API key. 

In order to sign apps for external release you must use a special macos specific flag called `--release-external`. Invoking this flag will sign a MacOS app bundle for external distribution, provided external signing is properly configured.

To obtain the proper security certificate, you must access the AppStore Connect security certificate wizard via your organization's account holder. This process will not work with a child account, even with administrative priveleges. 

First create a new keypair with openssl:

`openssl genrsa -out ~/<key_name>.key> 2048`

Next, generate a Certificate Signing Request (CSR) for your keypair

```
openssl req -new \ 
-key ~/<key_name>.key> \
-out ~/<key_name>.csr \
-subj "/CN=<your_full_legal_name> /O=<your_organization_name> /emailAddress=<your_developer_email>"
```

Navigate to the Apple Developer Portal and login to your apple developer account

`https://developer.apple.com/account/resources/certificates/list`
 
Press the `+` button to create a new certificate.

For the certificate category select `Developer ID Application` and press the `continue` button in the top right hand corner of the screen.

Upload your `<key_name>.csr` file created previously to the page and click next.

Click the download button. 

Obtain the `developerID_application.cer` from your downloads folder.

Convert this `.cer` file to a `.pem` file

`openssl x509 -inform der -in ~/Downloads/developerID_application.cer -out ~/developerID_application.pem`

Bundle the key and the converted security certificate into a .p12, remember the password, you'll need it the next step

```
openssl pkcs12 -export \
-out ~/<key_name>.p12 \
-inkey ~/<key_name>.key \
-in ~/cert.pem \
-name "Developer ID Application"
```

Import the paired .p12 into your keychain

```
security import ~/key.p12 \
-k $HOME/Library/Keychains/login.keychain-db \
$HOME/Library/Keychains \
-P "<password>" \
-T /usr/bin/codesign \
-T /usr/bin/productsign
```

Next, navigate to `https:://appleid.apple.com` and sign in with your apple developer account. You must create an app specific password for notarytool. To do this, go to `Sign-in and Security` and then click on `App-Specific Passwords`.

Give your app specific password a name like "Notarytool" or "Piston" and then copy it to your clipboard.

Enter the following command in your terminal (Note you can obtain your team id from logging in at `https://developer.apple.com/account` and clicking on "membership details" at the top or you can view your ID in parantheses next to your security profile/certificate). Developer email should match your security profile/developer account.

```
xcrun notarytool store-credentials "DeveloperID-Notary" \
    --apple-id <your developer email> \
    --team-id <your 10 character team ID> \
    --password <your app specific password> 
```

Find the full name of your desired security certificate in your keychain with the following command.

`security find-identity -v -p codesigning | grep "Developer ID"`

Provide the full name of the certificate returned by the above command (including the team ID in parantheses) in your `.env`

Example:

`external_cert=Developer ID Application: Organiation Name (A8BCD8EFGH)`

You can now build & sign MacOS apps for external release.


## IOS Output Configuration (MACOS HOST ONLY & after completing the MacOS setup above)

External release setup (above) is not required for iOS because iOS apps cannot be distributed outside of the App store.

### Prepare Xcode

Piston selects and prepares the full Xcode installation automatically. Physical iOS
device builds use its included iPhoneOS SDK and do not need simulator downloads.
See [Deploy to TestFlight](#deploy-to-testflight) for setup and explicit overrides.

Install libimobile device via homebrew (if you wish to deploy directly to iOS devices via USB cable)

`brew install libimobiledevice`

Set the path to libimobile device's `ideviceprovision` bin in your `.env` (if you wish to deploy directly to iOS devices via USB cable). 

Note: this can be found in your homebrew bin dir typically `/opt/homebrew/bin` on silicon architecture or `/usr/local/bin` on intel architecture.

`idp_path=/path/to/ideviceprovision`

### Configure IOS Cargo.toml parameters (optional)

```
[package.metadata.ios]
bundle_id=com.<organization>.<appname>
min_os_version=17.5
```

if you do not set a bundle_id in your `Cargo.toml`, the bundle ID will default to

`com.piston.<appname>`

if you do not set a min_os_version in your `Cargo.toml`, the mininimumOSVersion will default to 17.5

## Streaming Install to USB connected iPhone

To stream install an app to a USB tethered iOS device, ensure you first enable developer mode. To do this, go to `Settings > Privacy & Security > Developer Mode` and toggle it on.

Also ensure you "trust" the device after connecting it to your Mac via the pop up dialogue.

Your device MUST remain unlocked while working with cargo-piston. It is highly suggested to increase the screen lock time in your device settings.

Piston should take care of the rest. 

If Developer Mode does not appear as an option under `Settings > Privacy & Security > Developer Mode`, run these two commands sequentially with your device connected via USB:

`xcrun devicectl list devices`

`xcrun xctrace list devices`

After running these two commands you should see the toggle for Developer Mode in `Settings > Privacy & Security > Developer Mode`.

# Android Output Configuration

## Install Java

Install the Java JDK and provide the path to the installation in your `.env` file.
<!--
One option is to download the Java installer

`https://www.oracle.com/in/java/technologies/downloads/#jdk25-mac`
-->

The recommended way to get the Java JDK is to install OpenJDK 17 via Homebrew (Macos). Alternative versions, such as Oracle's JDK 25 are experimental:

`brew install openjdk@17`

Homebrew is the Apple community's de facto package manager. If you do not yet have Homebrew installed, follow the instructions here:

<https://brew.sh>

Set the path to the binary in your `.env`:

Example `.env` entries (Macos)

macos arm64 installer
`/usr/bin/java`

arm64 (homebrew)
`java_path=/opt/homebrew/opt/openjdk@17`

Intel chipset (homebrew)
`java_path=/usr/local/opt/openjdk@17`

Example install command (Linux)

`sudo apt update`

`sudo apt install -y openjdk-17-jdk`

Example `.env` entry (Linux)

`java_path=/usr/lib/jvm/java-17-openjdk-amd64`


## Install Android Command-line tools

Install the android NDK & SDK and provide the paths to the installation in your `.env` file.

Example install commands

Download & Install command line tools

SDK url Repository (MacOS)

`https://dl.google.com/android/repository/commandlinetools-mac-11076708_latest.zip`

SDK url Repository (Linux)

`https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip`

Download the file

`curl -o </path/to/downloads> <sdk_url_from_above>`

Create an install dir and unzip the file (replace $HOME with your absolute path)

`mkdir -p <$HOME>/Android/sdk`

`unzip -o </path/to/downloads>/cmdline-tools.zip -d <$HOME>/Android/sdk`

The cmdline-tools zip unpacks to a flat layout (`cmdline-tools/bin`, `cmdline-tools/lib`, ...), but sdkmanager requires the tools to live under a versioned `latest` directory. Restructure it:

`cd <$HOME>/Android/sdk/cmdline-tools`

`mkdir latest`

`mv bin lib NOTICE.txt source.properties latest/`

Accept android SDK licenses

`yes | JAVA_HOME=<PATH/TO/JAVA> <$HOME>/Android/sdk/cmdline-tools/latest/bin/sdkmanager --licenses --sdk_root=<$HOME>/Android/sdk || echo "Failed to accept the license"`

Note: if you installed java manually instead of using the installer you may need to set the JAVA_HOME var in your PATH or pass in the environment variable as shown above and below.

## Install Android SDK & NDK

Install platform-tools

`JAVA_HOME="</path/to/java>" sudo </path/to/sdkmanager> "platform-tools" --sdk_root=</path/to/sdk>`

Install build-tools;34.0.0

`JAVA_HOME="</path/to/java>" sudo </path/to/sdkmanager> "build-tools;34.0.0" --sdk_root=</path/to/sdk>`

Install platforms;android-34

`JAVA_HOME="</path/to/java>" sudo </path/to/sdkmanager> "platforms;android-34" --sdk_root=</path/to/sdk>`

Install ndk;25.1.8937393

`JAVA_HOME="</path/to/java>" sudo </path/to/sdkmanager> "ndk;25.1.8937393" --sdk_root=</path/to/sdk>`


Set the paths to the binaries in your `.env` (replace $HOME with your absolute path)

Examples (MacOS)

`sdk_path=<$HOME>/Android/sdk`

`ndk_path=<$HOME>/Android/sdk/ndk/26.1.10909125`

## Install Android Bundle tool

Download Android bundletool at:

`https://github.com/google/bundletool/releases`

Under Assets, download the `bundletool-all-1.18.3.jar` or whatever version is current at the time of your download.

<!--
or install with brew on macos

`brew install bundletool`
-->
Move your bundletool.jar to your Android directory:

`/Users/<user>/Android/sdk/bundle-tools/bundletool-all-1.18.3.jar`

Set the path to your bundle tool `.jar` in your `.env`:

`bundletool_path=/Users/<user>/Android/sdk/bundle-tools/bundletool-all-1.18.3.jar`

Note that if you upgrade `bundletool` in the future, you will need to update the .env to match your current version.

<!--
or if installed via brew

`bundletool_path=/opt/homebrew/Cellar/bundletool/1.18.3/libexec/bundletool-all.jar`

Make sure to replace 1.18.3 with whatever version is current when you download.
-->
## Android Cargo.Toml configuration

Add the following dependencies

```
[dependencies]

[target.'cfg(target_os = "android")'.dependencies]
android-activity = { version = "0.5", features = ["native-activity"] }
log = "0.4"
```

Your `Cargo.toml` must have the following library designation 

```
[lib]
name="<app_name>"
crate-type=["cdylib"]
```

Optionally, you can configure the following metadata parameters inside of your `Cargo.toml` within the `[package.metadata.android]` subheading. 

version_name is drawn automatically from your `Cargo.toml` within the `[package]` subheading. 

target_sdk_version must be installed in your `~/Android/sdk/platforms` path.

```
[package.metadata.android]
package="<com.example.appname>"
target_sdk_version=34
min_sdk_version=<21>
version_code=<1>
app_label="<app_name>"

```
Use the default values for `min_sdk_version` and `version_code` unless you have reason not to. For example, feel free to increment `version_code`
upon releasing version 2 of your program if need be. Technical minutia point: strictly speaking, only `package` is needed in the section `[package.metadata.android]`. 
If you were to forego populating `target_sdk_version` through `app_label` the default values listed would be populated by Piston automatically.
The Maintainers have decided to give the Users of Piston the option to populate their own values in the interest of Developer control.

## Create a Lib.rs in ~/src

Unlike other outputs, android apps require first building a cdylib, we've already designated those settings in the `Cargo.toml`, however, your project must also contain a `~/src/lib.rs` file with a main activity. It is important that if you are maintaing a cross compiled code base for multiple output types, that you wrap android specific logic in `#[cfg(target_os = "android")]` flags as shown below. 

Example `lib.rs`

```
#[cfg(target_os = "android")]
use android_activity::AndroidApp;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn android_main(app: AndroidApp) {
    loop {
        log::info!("Hello from Rust on Android!");
    }
}
```

## Automated Signing

Users can manually designate key for signing their Android App Bundle for upload to the Google Play Store. To designate a release key for Android App Bundles add the following values to your .env...

```
aab_keystore=/path/to/<release_key_name>.keystore
aab_key_pass=<keystore password>
aab_key_alias=<key alias>
```


Users are not required to manually specify a release key as shown above, if one is not provided a release key will be created at `~/.android/release.keystore` with the default keystore & key password `piston` and the key alias `release-key`. These defaults will be used to sign Android App Bundles unless otherwise specified. Cargo Piston assumes the Keystore password will match the key file password.

Users should also specicfy metadata for the signature on their android release key. These parameters can be set with the following `.env` values...

```
common_name=<developer_name>
org_unit=<department or team>
org=<company>
locality=<city or town>
state=<state or province>
country=<Two letter country code, example: 'US' for United States>
```

If any of these metadata values are not provided they are be left as "Unknown" in the dname subject field of the release key's signature.

## Streaming install to USB connected Android Phone

To stream install an app to a USB tethered Android device, ensure you first enable developer mode. To do this, go to `Settings` and then `About Phone`. You will then need to quickly tap `Build Number` 7 times in short succession. After that `Developer Options` will appear in your settings menu.

Also ensure you "trust" the device after connecting it to your PC/Mac via the dialogue popup.

Your device MUST remain unlocked while working with cargo-piston. It is highly suggested to increase the screen lock time in your device settings.

Piston should take care of the rest. 
