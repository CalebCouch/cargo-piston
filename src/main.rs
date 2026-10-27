mod android;
mod apple;
mod dev;
mod developer;
mod devices;
mod host;
mod ios;
mod linux;
mod macos;
mod manifest;
mod project;
mod windows;

mod cli;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() { env_logger::init(); cli::Cli::run() }
