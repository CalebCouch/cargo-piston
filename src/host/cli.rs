use std::io::{self, IsTerminal};
use std::path::Path;
use super::Host;

pub struct HostCli;

impl HostCli {
    pub fn interactive() -> bool { io::stdin().is_terminal() && io::stderr().is_terminal() }

    pub fn open_url(url: &str) {
        if Self::interactive() {
            if let Err(error) = Host::open_url(url) {
                eprintln!("{error}. Open this address: {url}");
            }
        }
    }

    pub fn open_accounts(developer: &Path) {
        if Self::interactive() {
            eprintln!("Apple Account sign-in needs attention. Opening Xcode's Accounts settings.");
            if let Err(error) = Host::open_accounts(developer) { eprintln!("{error}"); }
        }
    }
}
