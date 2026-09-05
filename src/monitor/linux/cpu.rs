//! CPU monitoring — Linux (hwmon, RAPL, procfs).
//!
//! Every getter returns `Option`: a sensor that is missing or unreadable is
//! reported as absent, never as a plausible-looking zero. Nothing in this module
//! terminates the process — a transient sysfs read failure must not kill a frame
//! loop that still owes the panel a blanking frame.

use crate::monitor::scale_temp;
use std::cell::RefCell;
use std::fs::{read_dir, read_to_string};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Shortest interval that yields a meaningful rate.
///
/// Without this, the first frame — whose baseline was taken microseconds
/// earlier during init — divides an energy delta by ~0 and reports a wildly
/// inflated wattage (clamped to 65535 W).
const MIN_SAMPLE_WINDOW: Duration = Duration::from_millis(50);

/// Snapshot of the RAPL energy counter, paired with when it was taken.
#[derive(Clone, Copy)]
pub struct EnergySample {
    micro_joules: u64,
    at: Instant,
}

#[derive(Clone)]
pub struct Cpu {
    temp_sensor: Option<PathBuf>,
    rapl_energy: Option<PathBuf>,
    rapl_max_uj: u64,
    cpu_fan: Option<PathBuf>,
    pump_fan: Option<PathBuf>,
    /// Previous CPU-time sample, so utilisation is a delta over the real update
    /// interval instead of a 100 ms blocking sleep inside every frame.
    last_instant: RefCell<Option<cpu_monitor::CpuInstant>>,
    /// When that sample was taken, to reject windows too short to be meaningful.
    last_at: std::cell::Cell<Option<Instant>>,
}

impl Cpu {
    pub fn new() -> Self {
        let rapl_energy = rapl_path("energy_uj").filter(|p| read_to_string(p).is_ok());
        Cpu {
            temp_sensor: find_temp_sensor(),
            rapl_max_uj: rapl_path("max_energy_range_uj").and_then(|p| read_u64(&p)).unwrap_or(0),
            rapl_energy,
            cpu_fan: find_fan_input(&["cpu fan", "cpu_fan", "cpufan"]),
            pump_fan: find_fan_input(&["pump", "water pump", "aio pump"]),
            last_instant: RefCell::new(cpu_monitor::CpuInstant::now().ok()),
            last_at: std::cell::Cell::new(Some(Instant::now())),
        }
    }

    pub fn warn_temp(&self) {
        if self.temp_sensor.is_none() {
            crate::warning!("No supported CPU temperature sensor was found");
            eprintln!(" CPU temperature will not be displayed, and alarm will be disabled.");
            eprintln!(" Supported kernel modules are: asusec, coretemp, k10temp, and zenpower.");
        }
    }

    pub fn warn_rapl(&self) {
        if self.rapl_energy.is_none() {
            crate::warning!("RAPL energy counter is not readable");
            // Since CVE-2020-8694 energy_uj is root-only on most kernels, while
            // max_energy_range_uj stays world-readable — so probing the latter
            // alone used to claim RAPL worked and then fail on the first read.
            eprintln!(" CPU power consumption will not be displayed.");
            eprintln!(" /sys/class/powercap/intel-rapl/intel-rapl:0/energy_uj is root-only on most kernels.");
        }
    }

    pub fn get_temp(&self, fahrenheit: bool) -> Option<u8> {
        let milli = read_u64(self.temp_sensor.as_ref()?)?;
        Some(scale_temp(milli as f64 / 1000.0, fahrenheit))
    }

    /// Reads the RAPL energy counter together with the instant it was read at.
    pub fn read_energy(&self) -> Option<EnergySample> {
        let path = self.rapl_energy.as_ref()?;
        Some(EnergySample {
            micro_joules: read_u64(path)?,
            at: Instant::now(),
        })
    }

    /// Average power since `start`, in watts.
    ///
    /// The window is measured, not assumed: the caller used to pass the nominal
    /// update interval while the real gap also included `get_usage`'s sleep,
    /// which over-reported power by 10% at 1 s and 2x at the 100 ms minimum.
    pub fn get_power(&self, start: Option<EnergySample>) -> Option<u16> {
        let start = start?;
        let now = self.read_energy()?;
        let elapsed = now.at.duration_since(start.at);
        if elapsed < MIN_SAMPLE_WINDOW {
            return None;
        }
        let delta_uj = if now.micro_joules >= start.micro_joules {
            now.micro_joules - start.micro_joules
        } else if self.rapl_max_uj > 0 {
            // Counter wrapped at max_energy_range_uj.
            (self.rapl_max_uj - start.micro_joules).checked_add(now.micro_joules)?
        } else {
            return None;
        };
        let watts = delta_uj as f64 / 1_000_000.0 / elapsed.as_secs_f64();
        Some(watts.round().clamp(0.0, u16::MAX as f64) as u16)
    }

    /// CPU utilisation over the interval since the previous call.
    ///
    /// Returns `None` until at least [`MIN_SAMPLE_WINDOW`] has passed since the
    /// baseline, so the first frame reports "not measured yet" rather than a
    /// number derived from a microsecond-long window.
    pub fn get_usage(&self) -> Option<u8> {
        let now = cpu_monitor::CpuInstant::now().ok()?;
        let at = Instant::now();
        if self.last_at.get().map(|t| at.duration_since(t) < MIN_SAMPLE_WINDOW) == Some(true) {
            return None;
        }
        let previous = self.last_instant.replace(Some(now));
        self.last_at.set(Some(at));
        let previous = previous?;
        let delta = now - previous;
        let usage = delta.non_idle() * 100.0;
        if !usage.is_finite() {
            return None;
        }
        Some(usage.round().clamp(0.0, 100.0) as u8)
    }

    pub fn get_frequency(&self) -> Option<u16> {
        let cpuinfo = read_to_string("/proc/cpuinfo").ok()?;
        let highest = cpuinfo
            .lines()
            .filter(|l| l.starts_with("cpu MHz"))
            .filter_map(|l| l.split(':').nth(1)?.trim().parse::<f64>().ok())
            .fold(f64::NAN, f64::max);
        if highest.is_nan() {
            return None;
        }
        Some(highest.round().clamp(0.0, u16::MAX as f64) as u16)
    }

    /// Обороты CPU-вентилятора (если доступен сенсор).
    pub fn get_cpu_fan_rpm(&self) -> Option<u16> {
        read_u64(self.cpu_fan.as_ref()?).map(|v| v.min(u16::MAX as u64) as u16)
    }

    /// Обороты помпы СЖО (если доступен сенсор).
    pub fn get_pump_fan_rpm(&self) -> Option<u16> {
        read_u64(self.pump_fan.as_ref()?).map(|v| v.min(u16::MAX as u64) as u16)
    }

    /// Температура жидкости СЖО — на Linux нет единого sysfs-интерфейса для неё.
    pub fn get_liquid_temp(&self, _fahrenheit: bool) -> Option<u8> {
        None
    }

    /// Напряжение VCore в милливольтах — не реализовано на Linux.
    pub fn get_vcore(&self) -> Option<u16> {
        None
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn read_u64(path: &Path) -> Option<u64> {
    read_to_string(path).ok()?.trim().parse::<u64>().ok()
}

fn rapl_path(leaf: &str) -> Option<PathBuf> {
    let p = PathBuf::from("/sys/class/powercap/intel-rapl/intel-rapl:0").join(leaf);
    p.exists().then_some(p)
}

fn find_temp_sensor() -> Option<PathBuf> {
    for entry in read_dir("/sys/class/hwmon").ok()?.flatten() {
        let path = entry.path();
        let Ok(name) = read_to_string(path.join("name")) else {
            continue;
        };
        if ["asusec", "coretemp", "k10temp", "zenpower"].contains(&name.trim()) {
            let input = path.join("temp1_input");
            if input.exists() {
                return Some(input);
            }
        }
    }
    None
}

/// Finds a `fanN_input` whose `fanN_label` matches one of `keywords`.
///
/// The previous version matched the keywords against the hwmon *driver* name
/// (`nct6798`, `k10temp`, ...), which never contains "cpu fan" or "pump" — so
/// both fan getters returned 0 on essentially every board.
fn find_fan_input(keywords: &[&str]) -> Option<PathBuf> {
    for entry in read_dir("/sys/class/hwmon").ok()?.flatten() {
        let dir = entry.path();
        for n in 1..=8u8 {
            let label_path = dir.join(format!("fan{n}_label"));
            let input_path = dir.join(format!("fan{n}_input"));
            if !input_path.exists() {
                continue;
            }
            let Ok(label) = read_to_string(&label_path) else {
                continue;
            };
            let label = label.trim().to_ascii_lowercase();
            if keywords.iter().any(|k| label.contains(k)) {
                return Some(input_path);
            }
        }
    }
    // Secondary heuristic: some drivers expose no labels at all, but name the
    // chip after the pump (e.g. the `kraken`/`aquacomputer` hwmon drivers).
    for entry in read_dir("/sys/class/hwmon").ok()?.flatten() {
        let dir = entry.path();
        let Ok(name) = read_to_string(dir.join("name")) else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if keywords.iter().any(|k| name.contains(k)) {
            let input = dir.join("fan1_input");
            if input.exists() {
                return Some(input);
            }
        }
    }
    None
}
