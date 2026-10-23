use std::collections::HashMap;
use crate::PistonError;
use super::Devices;

pub struct DevicesCli;

impl DevicesCli {
    pub fn list(env_vars: HashMap<String, String>, silent: bool) -> Result<Devices, PistonError> {
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
