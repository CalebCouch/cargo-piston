#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Host, LinuxToolchain};
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub use linux::{Host, LinuxToolchain};

pub mod cli;
pub use cli::Cli;
