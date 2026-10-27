use std::fmt;
use std::io::{self, IsTerminal, Write};
use crate::Result;

#[derive(Debug)]
pub enum Error {
    RustHostMissing,
    HostArchitectureUnsupported { target: String },
    HostUsernameMissing,
    SetupRequired { label: String },
    SetupCancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RustHostMissing => write!(f, "rustc did not report its host target."),
            Self::HostArchitectureUnsupported { target } => write!(f,
                "Cannot select a target for host {target}. Specify an explicit --target."),
            Self::HostUsernameMissing => write!(f, "whoami returned no user name."),
            Self::SetupRequired { label } => write!(f,
                "{label} needs setup. Rerun this command in an interactive terminal."),
            Self::SetupCancelled => write!(f, "Setup cancelled."),
        }
    }
}

impl std::error::Error for Error {}

pub struct Cli;

impl Cli {
    pub fn interactive() -> bool { io::stdin().is_terminal() && io::stderr().is_terminal() }

    pub fn prompt(label: &str, default: &str) -> Result<String> {
        if !Self::interactive() {
            return Err(Error::SetupRequired { label: label.into() }.into());
        }
        loop {
            eprint!("{label}");
            if !default.is_empty() { eprint!(" [{default}]"); }
            eprint!(" (q to cancel): ");
            io::stderr().flush()?;
            let mut input = String::new();
            let count = io::stdin().read_line(&mut input)?;
            let input = input.trim();
            if count == 0 || input.eq_ignore_ascii_case("q") {
                return Err(Error::SetupCancelled.into());
            }
            if !input.is_empty() { return Ok(input.into()); }
            if !default.is_empty() { return Ok(default.into()); }
            eprintln!("A value is required.");
        }
    }

    pub fn link(label: &str, url: &str) -> String {
        let label: String = label.chars().filter(|character| !character.is_control()).collect();
        let url: String = url.chars().filter(|character| !character.is_control()).collect();
        if io::stdout().is_terminal() && io::stderr().is_terminal()
            && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
            && std::env::var_os("NO_COLOR").is_none()
        {
            format!("{label}: \x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\")
        } else { format!("{label}: {url}") }
    }
}
