use std::io::{self, IsTerminal, Write};
use crate::{PistonError, Result};

pub struct Cli;

impl Cli {
    pub fn interactive() -> bool { io::stdin().is_terminal() && io::stderr().is_terminal() }

    pub fn prompt(label: &str, default: &str) -> Result<String> {
        if !Self::interactive() {
            return Err(PistonError::Generic(format!(
                "{label} needs setup. Rerun this command in an interactive terminal."
            )));
        }
        loop {
            eprint!("{label}");
            if !default.is_empty() { eprint!(" [{default}]"); }
            eprint!(" (q to cancel): ");
            io::stderr().flush().map_err(|error| PistonError::Generic(error.to_string()))?;
            let mut input = String::new();
            let count = io::stdin().read_line(&mut input)
                .map_err(|error| PistonError::Generic(error.to_string()))?;
            let input = input.trim();
            if count == 0 || input.eq_ignore_ascii_case("q") {
                return Err(PistonError::Generic("Setup cancelled.".into()));
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
