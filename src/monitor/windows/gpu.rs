//! Windows GPU monitoring via LibreHardwareMonitor.

use super::{read, to_percent, to_u16};
use crate::monitor::scale_temp;

pub struct Gpu;

impl Gpu {
    pub fn new() -> Self {
        Gpu
    }

    pub fn warn_missing(&self) {
        if self.get_temp(false).is_none() && self.get_usage().is_none() {
            crate::warning!("No supported GPU was found");
            eprintln!("         LibreHardwareMonitor HTTP did not expose GPU sensors.");
        }
    }

    pub fn get_temp(&self, fahrenheit: bool) -> Option<u8> {
        let c = read(&["gpu core"], 'c')?;
        Some(scale_temp(c.clamp(-50.0, 150.0), fahrenheit))
    }

    pub fn get_usage(&self) -> Option<u8> {
        read(&["gpu core"], '%').map(to_percent)
    }

    pub fn get_power(&self) -> Option<u16> {
        read(&["gpu package", "gpu power", "gpu core", "gpu soc"], 'w').map(to_u16)
    }

    pub fn get_vram_usage(&self) -> Option<u8> {
        read(&["gpu memory", "gpu memory controller", "d3d dedicated memory"], '%').map(to_percent)
    }

    pub fn get_frequency(&self) -> Option<u16> {
        read(&["gpu core"], 'h').map(to_u16)
    }
}

impl Default for Gpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Gpu {
    /// A GPU handle that reports every metric as absent. Test-only; on Windows
    /// there is no LHM server during tests, so `new()` already behaves this way.
    #[cfg(test)]
    pub fn absent() -> Self {
        Gpu
    }
}
