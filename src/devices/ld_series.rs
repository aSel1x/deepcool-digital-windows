//! Display module for:
//! - LD240
//! - LD360

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

    fn init(&mut self, device: &mut dyn Device) -> Result<()> {
        let mut init_data = [0u8; 64];
        init_data[0] = 16;
        init_data[1] = 104;
        init_data[2] = 1;
        init_data[3] = 1;
        init_data[4] = 2;
        init_data[5] = 3;
        init_data[6] = 1;
        init_data[7] = 112;
        init_data[8] = 22;
        device.write_report(&init_data)?;

        init_data[5] = 2;
        init_data[7] = 111;
        device.write_report(&init_data)?;

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
        data[3] = 1;
        data[4] = 11;
        data[5] = 1;
        data[6] = 2;
        data[7] = 5;

        let power = self.cpu.get_power(self.last_energy).unwrap_or(0).to_be_bytes();
        data[8] = power[0];
        data[9] = power[1];

        let temp = (self.cpu.get_temp(self.fahrenheit).unwrap_or(0) as f32).to_be_bytes();
        data[10] = if self.fahrenheit { 1 } else { 0 };
        data[11] = temp[0];
        data[12] = temp[1];
        data[13] = temp[2];
        data[14] = temp[3];

        data[15] = self.cpu.get_usage().unwrap_or(0);

        data[16] = checksum(&data, 15);
        data[17] = 22;

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
        assert_eq!([pkt[0], pkt[1], pkt[2], pkt[3], pkt[4]], [16, 104, 1, 1, 11]);
        assert_eq!(pkt[16], checksum(&pkt, 15));
        assert_eq!(pkt[17], 22);
    }
}
