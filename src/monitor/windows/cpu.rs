//! Windows CPU monitoring via LibreHardwareMonitor.

use super::{read, read_all, to_percent, to_u16};
use crate::monitor::scale_temp;

/// Opaque energy sample: RAPL is a Linux concept, so on Windows LHM reports
/// power directly and this carries nothing.
#[derive(Clone, Copy, Default)]
pub struct EnergySample;

#[derive(Clone, Copy, Default)]
pub struct Cpu;

impl Cpu {
    pub fn new() -> Self {
        Cpu
    }

    pub fn warn_temp(&self) {
        if self.get_temp(false).is_none() {
            crate::warning!("CPU temperature is unavailable");
            eprintln!("         Enable LibreHardwareMonitor's 'Run Remote Web Server'.");
        }
    }

    pub fn warn_rapl(&self) {
        if self.get_power(None).is_none() {
            crate::warning!("CPU power is unavailable");
        }
    }

    pub fn get_temp(&self, fahrenheit: bool) -> Option<u8> {
        let c = read(&["core (tctl/tdie)", "core (tdie)", "package", "cpu core"], 'c')?;
        Some(scale_temp(c.clamp(-50.0, 150.0), fahrenheit))
    }

    pub fn read_energy(&self) -> Option<EnergySample> {
        Some(EnergySample)
    }

    pub fn get_power(&self, _start: Option<EnergySample>) -> Option<u16> {
        read(&["cpu package", "package"], 'w').map(to_u16)
    }

    pub fn get_usage(&self) -> Option<u8> {
        read(&["cpu total"], '%').map(to_percent)
    }

    /// Core clock.
    ///
    /// "Bus Speed" is deliberately *not* in the keyword list: the exact-match
    /// pass runs over every keyword before the substring pass, so on LHM's Intel
    /// path — which exposes "Bus Speed" but no "Cores (Average)" — it silently
    /// won and reported the ~100 MHz reference clock as the core frequency.
    pub fn get_frequency(&self) -> Option<u16> {
        if let Some(avg) = read(&["cores (average)"], 'h') {
            return Some(to_u16(avg));
        }
        // Fall back to the fastest individual core, ignoring "(effective)" ones.
        let cores = read_all("cpu core #", 'h');
        cores
            .into_iter()
            .fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |a| a.max(v))))
            .map(to_u16)
    }

    pub fn get_cpu_fan_rpm(&self) -> Option<u16> {
        read(&["cpu fan"], 'r').map(to_u16)
    }

    pub fn get_pump_fan_rpm(&self) -> Option<u16> {
        read(&["pump fan #1", "pump fan", "water pump"], 'r').map(to_u16)
    }

    pub fn get_liquid_temp(&self, fahrenheit: bool) -> Option<u8> {
        let c = read(&["liquid", "coolant"], 'c')?;
        Some(scale_temp(c.clamp(-50.0, 150.0), fahrenheit))
    }

    /// VCore в милливольтах.
    pub fn get_vcore(&self) -> Option<u16> {
        read(&["vcore"], 'v').map(|volts| to_u16(volts.clamp(0.0, 2.0) * 1000.0))
    }
}
