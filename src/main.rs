mod android;
mod apple;
mod dev;
mod devices;
mod error;
mod project;
use error::PistonError;
mod host;
mod ios;
mod linux;
mod macos;
mod manifest;
mod windows;

mod cli;

pub type Result<T> = std::result::Result<T, PistonError>;

fn main() { env_logger::init(); cli::Cli::run() }
