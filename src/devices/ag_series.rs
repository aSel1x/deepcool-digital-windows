//! Display module for:
//! - AG300 DIGITAL
//! - AG400 DIGITAL
//! - AG500 DIGITAL
//! - AG620 DIGITAL
//!
//! # Celsius only
//!
//! The AG protocol has one temperature unit code — 19 (°C). There is no
//! Fahrenheit counterpart in `device-list/tables/ag-series.md`, and the panel is
//! two digits wide. `Caps::fahrenheit` is false for this device, so `main`
//! warns and forces Celsius instead of sending an F value under the C code —
//! which used to freeze the display at "99" from about 38 °C upwards.

use super::{digits2, Ctx, DisplayDriver, Mode};
use crate::device::Device;
use crate::error::Result;
use crate::monitor::Cpu;

pub const TEMP_LIMIT_C: u8 = 90;

const MODES: &[Mode] = &[Mode::CpuTemperature, Mode::CpuUsage, Mode::Auto];

pub struct Display {
    cpu: Cpu,
    alarm: bool,
}

pub fn build(ctx: Ctx) -> Result<Box<dyn DisplayDriver>> {
    Ok(Box::new(Display {
        cpu: ctx.cpu,
        alarm: ctx.alarm,
    }))
}

impl DisplayDriver for Display {
    fn modes(&self) -> &'static [Mode] {
        MODES
    }

    fn warn(&self, _mode: Mode) {
        self.cpu.warn_temp();
    }

    fn frame(&mut self, device: &mut dyn Device, mode: Mode) -> Result<()> {
        device.write_report(&self.status_message(mode))
    }
}

impl Display {
    fn status_message(&self, mode: Mode) -> [u8; 64] {
        let mut data = [0u8; 64];
        data[0] = 16;

        if mode == Mode::CpuUsage {
            let usage = self.cpu.get_usage().unwrap_or(0);
            data[1] = 76;
            let d = digits2(usage as u16);
            data[3] = d[0];
            data[4] = d[1];
        }

        // Always Celsius: see the module docs.
        let temp = self.cpu.get_temp(false).unwrap_or(0);

        if mode == Mode::CpuTemperature {
            data[1] = 19;
            let d = digits2(temp as u16);
            data[3] = d[0];
            data[4] = d[1];
        }

        data[5] = (self.alarm && temp >= TEMP_LIMIT_C) as u8;

        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_reports_celsius() {
        let d = Display {
            cpu: Cpu::new(),
            alarm: false,
        };
        // There is no fahrenheit field to set: the unit code is unconditional.
        assert_eq!(d.status_message(Mode::CpuTemperature)[1], 19);
    }

    #[test]
    fn usage_mode_uses_two_digits() {
        let d = Display {
            cpu: Cpu::new(),
            alarm: false,
        };
        let pkt = d.status_message(Mode::CpuUsage);
        assert_eq!(pkt[1], 76);
        assert!(pkt[3] <= 9 && pkt[4] <= 9, "digits must stay in 0..=9");
    }
}
