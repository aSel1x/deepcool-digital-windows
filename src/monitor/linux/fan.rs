//! Системные вентиляторы (не CPU и не помпа — те живут в `Cpu`).

use std::fs::{read_dir, read_to_string};
use std::path::PathBuf;

pub struct SysFan {
    /// Resolved once at startup: rescanning every hwmon on every frame is both
    /// slow and non-deterministic in ordering.
    inputs: Vec<PathBuf>,
}

impl SysFan {
    pub fn new() -> Self {
        SysFan { inputs: discover() }
    }

    /// Обороты системного вентилятора #N.
    pub fn get_rpm(&self, index: usize) -> Option<u16> {
        let path = self.inputs.get(index)?;
        let rpm = read_to_string(path).ok()?.trim().parse::<u64>().ok()?;
        Some(rpm.min(u16::MAX as u64) as u16)
    }
}

impl Default for SysFan {
    fn default() -> Self {
        Self::new()
    }
}

fn discover() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = read_dir("/sys/class/hwmon") else {
        return out;
    };
    // Sort by hwmon path so fan #0 means the same thing across restarts.
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    dirs.sort();

    for dir in dirs {
        let Ok(name) = read_to_string(dir.join("name")) else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name.contains("cpu") || name.contains("pump") || name.contains("aio") {
            continue;
        }
        for n in 1..=8u8 {
            let label = read_to_string(dir.join(format!("fan{n}_label")))
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if label.contains("cpu") || label.contains("pump") || label.contains("aio") {
                continue;
            }
            let input = dir.join(format!("fan{n}_input"));
            if input.exists() {
                out.push(input);
            }
        }
    }
    out
}
