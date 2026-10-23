#[cfg(target_os = "macos")]
#[path = "host/macos.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "host/linux.rs"]
mod platform;

pub use platform::{Credentials, Host, LinuxToolchain};

pub mod cli;
