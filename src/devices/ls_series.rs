//! Display module for:
//! - LS520 SE DIGITAL
//! - LS720 SE DIGITAL

use super::{bar_level, digits3, Ctx, DisplayDriver, Mode};
use crate::device::Device;
use crate::error::Result;
use crate::monitor::{Cpu, EnergySample};

pub const TEMP_LIMIT_C: u8 = 90;
pub const TEMP_LIMIT_F: u8 = 194;

const MODES: &[Mode] = &[Mode::CpuTemperature, Mode::CpuPower, Mode::Auto];

pub struct Display {
    cpu: Cpu,
    fahrenheit: bool,
    alarm: bool,
    /// Energy counter sampled at the end of the previous frame.
    ///
    /// Power is now averaged over the measured gap between two samples instead
    /// of over the nominal update interval — the old code passed `update` while
    /// the real window also contained `get_usage`'s 100 ms sleep, over-reporting
    /// power by 10% at the 1 s default and 2x at the 100 ms minimum.
    last_energy: Option<EnergySample>,
}

pub fn build(ctx: Ctx) -> Result<Box<dyn DisplayDriver>> {
    Ok(Box::new(Display {
        cpu: ctx.cpu,
        fahrenheit: ctx.fahrenheit,
        alarm: ctx.alarm,
        last_energy: None,
    }))
}

impl DisplayDriver for Display {
    fn modes(&self) -> &'static [Mode] {
        MODES
    }

    fn warn(&self, mode: Mode) {
        self.cpu.warn_temp();
        if matches!(mode, Mode::CpuPower | Mode::Auto) {
            self.cpu.warn_rapl();
        }
    }

    fn init(&mut self, device: &mut dyn Device) -> Result<()> {
        let mut init_data = [0u8; 64];
        init_data[0] = 16;
        init_data[1] = 170;
        self.last_energy = self.cpu.read_energy();
        device.write_report(&init_data)
    }

    fn frame(&mut self, device: &mut dyn Device, mode: Mode) -> Result<()> {
        let packet = self.status_message(mode);
        self.last_energy = self.cpu.read_energy();
        device.write_report(&packet)
    }
}

impl Display {
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
            Mode::CpuPower => {
                let power = self.cpu.get_power(self.last_energy).unwrap_or(0);
                data[1] = 76;
                let d = digits3(power);
                data[3] = d[0];
                data[4] = d[1];
                data[5] = d[2];
            }
            _ => (),
        }
        data[2] = bar_level(usage);
        data[6] = (self.alarm && temp >= if self.fahrenheit { TEMP_LIMIT_F } else { TEMP_LIMIT_C }) as u8;

        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_above_999_watts_clamps_instead_of_wrapping() {
        // digits3 is what guards this; assert the contract the packet relies on.
        assert_eq!(super::digits3(1200), [9, 9, 9]);
    }

    #[test]
    fn unit_codes_match_the_protocol_table() {
        let d = Display {
            cpu: Cpu::new(),
            fahrenheit: false,
            alarm: false,
            last_energy: None,
        };
        assert_eq!(d.status_message(Mode::CpuTemperature)[1], 19);
        assert_eq!(d.status_message(Mode::CpuPower)[1], 76);

        let f = Display {
            cpu: Cpu::new(),
            fahrenheit: true,
            alarm: false,
            last_energy: None,
        };
        assert_eq!(f.status_message(Mode::CpuTemperature)[1], 35);
    }
}
