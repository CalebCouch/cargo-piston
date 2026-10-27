use std::collections::HashMap;
use std::fmt;
use crate::Result;
use super::Devices;

#[derive(Debug)]
pub enum Error {
    ADBDevicesError(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ADBDevicesError(err) => {
                write!(f, "Error Running 'ADB Devices', check installation and .env path: {}", err)
            }
        }
    }
}

impl std::error::Error for Error {}

pub struct Cli;

impl Cli {
    pub fn list(env_vars: HashMap<String, String>, silent: bool) -> Result<Devices> {
        let devices = Devices::discover(env_vars)?;
        if !devices.android_available { println!("Android installation not found"); }
        if !silent { Self::print_devices(&devices); }
        Ok(devices)
    }
    fn print_devices(devices: &Devices) {
        println!();
        println!("Cargo Piston Device List:");
        println!();
        if devices.ios.is_empty() && devices.android.is_empty() {
            println!("No devices connected");
            return;
        }
        if !devices.android.is_empty() {
            println!("Android:");
            for (index, device) in devices.android.iter().enumerate() {
                println!();
                println!("Device {}:", index + 1);
                println!("id: {}", device.id);
            }
            if !devices.ios.is_empty() { println!(); }
        }
        if !devices.ios.is_empty() {
            println!("iOS:");
            for (index, device) in devices.ios.iter().enumerate() {
                println!();
                println!("Device {}:", index + 1);
                println!("Model: {}", device.model);
                println!("id: {}", device.id);
            }
        }
    }
}
