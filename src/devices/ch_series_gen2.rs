//! Display module for:
//! - CH170 DIGITAL
//! - CH270 DIGITAL
//! - CH690 DIGITAL

use super::{checksum, Ctx, DisplayDriver, Mode};
use crate::device::Device;
use crate::error::Result;
use crate::monitor::{Cpu, EnergySample, Gpu};
use crate::warning;

// cpu_fan and psu set their protocol selector byte but carry no data yet. They
// stay accepted — rejecting them would turn a previously working config into a
// startup failure — and warn instead. Auto only cycles the implemented ones.
const MODES: &[Mode] = &[Mode::CpuFrequency, Mode::Gpu, Mode::Auto, Mode::CpuFan, Mode::Psu];
const AUTO_CYCLE: &[Mode] = &[Mode::CpuFrequency, Mode::Gpu];

#[cfg(test)]
pub(super) const MODES_FOR_TEST: &[Mode] = MODES;
#[cfg(test)]
pub(super) const AUTO_CYCLE_FOR_TEST: &[Mode] = AUTO_CYCLE;

pub struct Display {
    cpu: Cpu,
    gpu: Gpu,
    fahrenheit: bool,
    last_energy: Option<EnergySample>,
}

pub fn build(ctx: Ctx) -> Result<Box<dyn DisplayDriver>> {
    Ok(Box::new(Display {
        cpu: ctx.cpu,
        gpu: ctx.gpu,
        fahrenheit: ctx.fahrenheit,
        last_energy: None,
    }))
}

impl DisplayDriver for Display {
    fn modes(&self) -> &'static [Mode] {
        MODES
    }

    fn auto_cycle(&self) -> &'static [Mode] {
        AUTO_CYCLE
    }

    fn warn(&self, mode: Mode) {
        match mode {
            Mode::CpuFan => warning!("CPU fan speed monitoring is not yet supported"),
            Mode::Psu => warning!("PSU monitoring is not yet supported"),
            Mode::Auto => warning!("Display mode \"auto\" only cycles between fully supported modes"),
            _ => (),
        }
        if matches!(mode, Mode::CpuFrequency | Mode::CpuFan | Mode::Auto) {
            self.cpu.warn_temp();
            self.cpu.warn_rapl();
        }
        if matches!(mode, Mode::Gpu | Mode::Auto) {
            self.gpu.warn_missing();
        }
    }

    fn init(&mut self, _device: &mut dyn Device) -> Result<()> {
        self.last_energy = self.cpu.read_energy();
        Ok(())
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
        data[1] = 104;
        data[2] = 1;
        data[3] = 6;
        data[4] = 35;
        data[5] = 1;
        data[9] = if self.fahrenheit { 1 } else { 0 };

        data[6] = match mode {
            Mode::CpuFrequency => 2,
            Mode::CpuFan => 3,
            Mode::Gpu => 4,
            Mode::Psu => 5,
            _ => 0,
        };

        match mode {
            Mode::CpuFrequency | Mode::CpuFan => {
                let power = self.cpu.get_power(self.last_energy).unwrap_or(0).to_be_bytes();
                data[7] = power[0];
                data[8] = power[1];

                let temp = (self.cpu.get_temp(self.fahrenheit).unwrap_or(0) as f32).to_be_bytes();
                data[10] = temp[0];
                data[11] = temp[1];
                data[12] = temp[2];
                data[13] = temp[3];

                data[14] = self.cpu.get_usage().unwrap_or(0);

                if mode == Mode::CpuFrequency {
                    let frequency = self.cpu.get_frequency().unwrap_or(0).to_be_bytes();
                    data[15] = frequency[0];
                    data[16] = frequency[1];
                }
            }
            Mode::Gpu => {
                let power = self.gpu.get_power().unwrap_or(0).to_be_bytes();
                data[19] = power[0];
                data[20] = power[1];

                let temp = (self.gpu.get_temp(self.fahrenheit).unwrap_or(0) as f32).to_be_bytes();
                data[21] = temp[0];
                data[22] = temp[1];
                data[23] = temp[2];
                data[24] = temp[3];

                data[25] = self.gpu.get_usage().unwrap_or(0);

                let frequency = self.gpu.get_frequency().unwrap_or(0).to_be_bytes();
                data[26] = frequency[0];
                data[27] = frequency[1];
            }
            _ => (),
        }

        data[40] = checksum(&data, 39);
        data[41] = 22;

        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display() -> Display {
        Display {
            cpu: Cpu::new(),
            gpu: Gpu::absent(),
            fahrenheit: false,
            last_energy: None,
        }
    }

    #[test]
    fn mode_selector_byte_matches_the_protocol_table() {
        assert_eq!(display().status_message(Mode::CpuFrequency)[6], 2);
        assert_eq!(display().status_message(Mode::Gpu)[6], 4);
    }

    #[test]
    fn checksum_covers_bytes_one_through_thirtynine() {
        let pkt = display().status_message(Mode::CpuFrequency);
        let manual = (pkt[1..=39].iter().map(|&b| b as u16).sum::<u16>() % 256) as u8;
        assert_eq!(pkt[40], manual);
        assert_eq!(pkt[41], 22);
    }

    #[test]
    fn not_yet_implemented_modes_are_accepted_but_never_auto_cycled() {
        let d = display();
        // Accepted: rejecting them would break configs that worked before.
        assert!(d.modes().contains(&Mode::CpuFan));
        assert!(d.modes().contains(&Mode::Psu));
        // But `auto` only rotates through the modes that actually draw data.
        assert_eq!(d.auto_cycle(), &[Mode::CpuFrequency, Mode::Gpu]);
    }
}
