//! Display module for:
//! - AK620 G2 DIGITAL NYX
//! - AK700 DIGITAL NYX
//! - ASSASSIN IV VC VISION
//! - LQ240
//! - LQ360

use super::{checksum, Ctx, DisplayDriver, Mode};
use crate::device::Device;
use crate::error::Result;
use crate::monitor::{Cpu, EnergySample};

const MODES: &[Mode] = &[Mode::Auto];

pub struct Display {
    cpu: Cpu,
    fahrenheit: bool,
    last_energy: Option<EnergySample>,
}

pub fn build(ctx: Ctx) -> Result<Box<dyn DisplayDriver>> {
    Ok(Box::new(Display {
        cpu: ctx.cpu,
        fahrenheit: ctx.fahrenheit,
        last_energy: None,
    }))
}

impl DisplayDriver for Display {
    fn modes(&self) -> &'static [Mode] {
        MODES
    }

    fn warn(&self, _mode: Mode) {
        self.cpu.warn_temp();
        self.cpu.warn_rapl();
    }

    fn init(&mut self, _device: &mut dyn Device) -> Result<()> {
        self.last_energy = self.cpu.read_energy();
        Ok(())
    }

    fn frame(&mut self, device: &mut dyn Device, _mode: Mode) -> Result<()> {
        let packet = self.status_message();
        self.last_energy = self.cpu.read_energy();
        device.write_report(&packet)
    }
}

impl Display {
    fn status_message(&self) -> [u8; 64] {
        let mut data = [0u8; 64];
        data[0] = 16;
        data[1] = 104;
        data[2] = 1;
        data[3] = 8;
        data[4] = 12;
        data[5] = 1;
        data[6] = 2;

        let power = self.cpu.get_power(self.last_energy).unwrap_or(0).to_be_bytes();
        data[7] = power[0];
        data[8] = power[1];

        let temp = (self.cpu.get_temp(self.fahrenheit).unwrap_or(0) as f32).to_be_bytes();
        data[9] = if self.fahrenheit { 1 } else { 0 };
        data[10] = temp[0];
        data[11] = temp[1];
        data[12] = temp[2];
        data[13] = temp[3];

        data[14] = self.cpu.get_usage().unwrap_or(0);

        let frequency = self.cpu.get_frequency().unwrap_or(0).to_be_bytes();
        data[15] = frequency[0];
        data[16] = frequency[1];

        data[17] = checksum(&data, 16);
        data[18] = 22;

        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_header_and_terminator_match_the_protocol_table() {
        let d = Display {
            cpu: Cpu::new(),
            fahrenheit: false,
            last_energy: None,
        };
        let pkt = d.status_message();
        assert_eq!([pkt[1], pkt[2], pkt[3], pkt[4], pkt[5], pkt[6]], [104, 1, 8, 12, 1, 2]);
        assert_eq!(pkt[17], checksum(&pkt, 16));
        assert_eq!(pkt[18], 22);
    }

    #[test]
    fn temperature_is_encoded_as_a_big_endian_f32() {
        let d = Display {
            cpu: Cpu::new(),
            fahrenheit: false,
            last_energy: None,
        };
        let pkt = d.status_message();
        let bytes = [pkt[10], pkt[11], pkt[12], pkt[13]];
        let decoded = f32::from_be_bytes(bytes);
        assert!(decoded.is_finite() && (0.0..=255.0).contains(&decoded), "{decoded}");
    }
}
