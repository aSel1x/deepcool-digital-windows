//! Unified display module for AK PRO series:
//! - AK400 DIGITAL PRO
//! - AK500 DIGITAL PRO
//! - AK620 DIGITAL PRO

use super::{checksum, Ctx, DisplayDriver, Mode};
use crate::device::Device;
use crate::error::Result;
use crate::monitor::{Cpu, EnergySample};

const MODES: &[Mode] = &[Mode::Auto];

/// Per-model bytes of the otherwise identical AK PRO packet.
pub struct ProConfig {
    pub init_byte_3: u8,
    pub init_byte_4: u8,
    pub init_byte_7: u8,
    pub has_frequency: bool,
}

pub const AK400_PRO_CONFIG: ProConfig = ProConfig {
    init_byte_3: 2,
    init_byte_4: 11,
    init_byte_7: 5,
    has_frequency: false,
};

pub const AK620_PRO_CONFIG: ProConfig = ProConfig {
    init_byte_3: 4,
    init_byte_4: 13,
    init_byte_7: 8,
    has_frequency: true,
};

pub struct Display {
    cpu: Cpu,
    fahrenheit: bool,
    config: &'static ProConfig,
    last_energy: Option<EnergySample>,
}

pub fn build_ak400(ctx: Ctx) -> Result<Box<dyn DisplayDriver>> {
    Ok(Box::new(Display::new(ctx, &AK400_PRO_CONFIG)))
}

pub fn build_ak620(ctx: Ctx) -> Result<Box<dyn DisplayDriver>> {
    Ok(Box::new(Display::new(ctx, &AK620_PRO_CONFIG)))
}

impl Display {
    fn new(ctx: Ctx, config: &'static ProConfig) -> Self {
        Display {
            cpu: ctx.cpu,
            fahrenheit: ctx.fahrenheit,
            config,
            last_energy: None,
        }
    }

    fn status_message(&self) -> [u8; 64] {
        let mut data = [0u8; 64];
        data[0] = 16;
        data[1] = 104;
        data[2] = 1;
        data[3] = self.config.init_byte_3;
        data[4] = self.config.init_byte_4;
        data[5] = 1;
        data[6] = 2;
        data[7] = self.config.init_byte_7;

        // Power consumption
        let power = self.cpu.get_power(self.last_energy).unwrap_or(0).to_be_bytes();
        data[8] = power[0];
        data[9] = power[1];

        // Temperature
        let temp = (self.cpu.get_temp(self.fahrenheit).unwrap_or(0) as f32).to_be_bytes();
        data[10] = if self.fahrenheit { 1 } else { 0 };
        data[11] = temp[0];
        data[12] = temp[1];
        data[13] = temp[2];
        data[14] = temp[3];

        // Utilization
        data[15] = self.cpu.get_usage().unwrap_or(0);

        if self.config.has_frequency {
            let frequency = self.cpu.get_frequency().unwrap_or(0).to_be_bytes();
            data[16] = frequency[0];
            data[17] = frequency[1];
            data[18] = checksum(&data, 17);
            data[19] = 22;
        } else {
            data[16] = checksum(&data, 15);
            data[17] = 22;
        }

        data
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn display(config: &'static ProConfig) -> Display {
        Display {
            cpu: Cpu::new(),
            fahrenheit: false,
            config,
            last_energy: None,
        }
    }

    #[test]
    fn ak400_packet_layout_matches_the_protocol_table() {
        let pkt = display(&AK400_PRO_CONFIG).status_message();
        assert_eq!([pkt[0], pkt[1], pkt[2], pkt[3], pkt[4]], [16, 104, 1, 2, 11]);
        assert_eq!(pkt[7], 5);
        // Checksum in [16], terminator in [17]; frequency bytes unused.
        assert_eq!(pkt[16], checksum(&pkt, 15));
        assert_eq!(pkt[17], 22);
    }

    #[test]
    fn ak620_packet_carries_frequency_and_shifts_the_checksum() {
        let pkt = display(&AK620_PRO_CONFIG).status_message();
        assert_eq!([pkt[3], pkt[4], pkt[7]], [4, 13, 8]);
        assert_eq!(pkt[18], checksum(&pkt, 17));
        assert_eq!(pkt[19], 22);
    }

    #[test]
    fn checksum_is_a_byte_sum_modulo_256() {
        let pkt = display(&AK620_PRO_CONFIG).status_message();
        let manual = (pkt[1..=17].iter().map(|&b| b as u16).sum::<u16>() % 256) as u8;
        assert_eq!(pkt[18], manual);
    }
}
