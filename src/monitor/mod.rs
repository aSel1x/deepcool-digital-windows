//! System monitoring.
//!
//! Linux:   kernel interfaces (sysfs, procfs, NVML)
//! Windows: LibreHardwareMonitor HTTP API
//!
//! Every getter returns `Option`. "Sensor absent" and "value is zero" are
//! different facts, and collapsing them is how a display ends up confidently
//! showing a number that was never measured.

#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

// ── Reexports ──────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
pub use linux::{cpu::Cpu, cpu::EnergySample, fan::SysFan, gpu::Gpu, ram::Ram};
#[cfg(target_os = "windows")]
pub use windows::{cpu::Cpu, cpu::EnergySample, fan::SysFan, gpu::Gpu, ram::Ram};

/// Converts a Celsius reading to the requested unit and clamps it to a byte.
///
/// Shared so the two platforms cannot round or clamp differently.
pub fn scale_temp(celsius: f64, fahrenheit: bool) -> u8 {
    let value = if fahrenheit {
        celsius * 9.0 / 5.0 + 32.0
    } else {
        celsius
    };
    value.round().clamp(0.0, 255.0) as u8
}

/// Everything the display drivers can ask for, sampled once per frame.
///
/// Fetching each metric independently is what made a single LM frame cost
/// eleven HTTP requests on Windows; drivers now take one snapshot and read from
/// it. `None` means the sensor is genuinely unavailable.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sample {
    pub cpu_temp: Option<u8>,
    pub cpu_usage: Option<u8>,
    pub cpu_power: Option<u16>,
    pub cpu_freq: Option<u16>,
    pub cpu_fan_rpm: Option<u16>,
    pub pump_fan_rpm: Option<u16>,
    pub liquid_temp: Option<u8>,
    pub vcore: Option<u16>,
    pub gpu_temp: Option<u8>,
    pub gpu_usage: Option<u8>,
    pub gpu_power: Option<u16>,
    pub gpu_freq: Option<u16>,
    pub vram_usage: Option<u8>,
    pub ram_usage: Option<u8>,
    pub sys_fan_rpm: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::scale_temp;

    #[test]
    fn celsius_rounds_half_away_from_zero() {
        assert_eq!(scale_temp(61.4, false), 61);
        assert_eq!(scale_temp(61.6, false), 62);
    }

    #[test]
    fn fahrenheit_matches_the_documented_alarm_thresholds() {
        assert_eq!(scale_temp(90.0, true), 194);
        assert_eq!(scale_temp(0.0, true), 32);
    }

    #[test]
    fn clamps_instead_of_wrapping() {
        assert_eq!(scale_temp(-40.0, false), 0);
        assert_eq!(scale_temp(1000.0, false), 255);
    }
}
