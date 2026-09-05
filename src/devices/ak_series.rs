//! Display module for:
//! - AK400 DIGITAL
//! - AK500 DIGITAL
//! - AK500S DIGITAL
//! - AK620 DIGITAL

use super::{bar_level, digits3, Ctx, DisplayDriver, Mode};
use crate::device::Device;
use crate::error::Result;
use crate::monitor::Cpu;

pub const TEMP_LIMIT_C: u8 = 90;
pub const TEMP_LIMIT_F: u8 = 194;

const MODES: &[Mode] = &[Mode::CpuTemperature, Mode::CpuUsage, Mode::Auto];

pub struct Display {
    cpu: Cpu,
    fahrenheit: bool,
    alarm: bool,
}

pub fn build(ctx: Ctx) -> Result<Box<dyn DisplayDriver>> {
    Ok(Box::new(Display {
        cpu: ctx.cpu,
        fahrenheit: ctx.fahrenheit,
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

    fn init(&mut self, device: &mut dyn Device) -> Result<()> {
        let mut init_data = [0u8; 64];
        init_data[0] = 16;
        init_data[1] = 170;
        device.write_report(&init_data)
    }

    fn frame(&mut self, device: &mut dyn Device, mode: Mode) -> Result<()> {
        device.write_report(&self.status_message(mode))
    }
}

impl Display {
    /// Reads the CPU status information and returns the data packet.
    fn status_message(&self, mode: Mode) -> [u8; 64] {
        let mut data = [0u8; 64];
        data[0] = 16;

        let usage = self.cpu.get_usage().unwrap_or(0);
        let temp = self.cpu.get_temp(self.fahrenheit).unwrap_or(0);

        match mode {
            Mode::CpuTemperature => {
                data[1] = if self.fahrenheit { 35 } else { 19 };
                let d = digits3(temp as u16);
                data[3] = d[0];
                data[4] = d[1];
                data[5] = d[2];
            }
            Mode::CpuUsage => {
                data[1] = 76;
                let d = digits3(usage as u16);
                data[3] = d[0];
                data[4] = d[1];
                data[5] = d[2];
            }
            _ => (),
        }
        // Status bar
        data[2] = bar_level(usage);
        // Alarm
        data[6] = (self.alarm && temp >= if self.fahrenheit { TEMP_LIMIT_F } else { TEMP_LIMIT_C }) as u8;

        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(fahrenheit: bool, alarm: bool) -> Display {
        Display {
            cpu: Cpu::new(),
            fahrenheit,
            alarm,
        }
    }

    #[test]
    fn temperature_mode_uses_the_celsius_unit_code() {
        let d = display(false, false);
        let pkt = d.status_message(Mode::CpuTemperature);
        assert_eq!(pkt[0], 16, "report id");
        assert_eq!(pkt[1], 19, "unit code for Celsius");
    }

    #[test]
    fn temperature_mode_uses_the_fahrenheit_unit_code() {
        let d = display(true, false);
        assert_eq!(d.status_message(Mode::CpuTemperature)[1], 35);
    }

    #[test]
    fn usage_mode_uses_the_usage_code() {
        let d = display(false, false);
        assert_eq!(d.status_message(Mode::CpuUsage)[1], 76);
    }

    #[test]
    fn the_alarm_byte_is_zero_when_the_alarm_is_disabled() {
        let d = display(false, false);
        assert_eq!(d.status_message(Mode::CpuTemperature)[6], 0);
    }
}
