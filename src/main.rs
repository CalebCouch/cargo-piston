mod android;
mod apple;
mod dev;
mod developer;
mod devices;
mod error;
use error::PistonError;
mod host;
mod ios;
mod linux;
mod macos;
mod manifest;
mod project;
mod windows;

mod cli;

pub type Result<T> = std::result::Result<T, PistonError>;

fn main() { env_logger::init(); cli::Cli::run() }
