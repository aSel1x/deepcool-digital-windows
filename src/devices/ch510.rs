//! Display module for:
//! - CH510 MESH DIGITAL

use super::{Ctx, DisplayDriver, Mode};
use crate::device::Device;
use crate::error::Result;
use crate::monitor::{Cpu, Gpu};

const MODES: &[Mode] = &[Mode::Cpu, Mode::Gpu];

pub struct Display {
    cpu: Cpu,
    gpu: Gpu,
    fahrenheit: bool,
}

pub fn build(ctx: Ctx) -> Result<Box<dyn DisplayDriver>> {
    Ok(Box::new(Display {
        cpu: ctx.cpu,
        gpu: ctx.gpu,
        fahrenheit: ctx.fahrenheit,
    }))
}

impl DisplayDriver for Display {
    fn modes(&self) -> &'static [Mode] {
        MODES
    }

    fn warn(&self, mode: Mode) {
        match mode {
            Mode::Gpu => self.gpu.warn_missing(),
            _ => self.cpu.warn_temp(),
        }
    }

    /// The CH510 protocol is a variable-length ASCII string, and the released
    /// version sends exactly those bytes — not a 64-byte zero-padded report.
    fn frame(&mut self, device: &mut dyn Device, mode: Mode) -> Result<()> {
        let message = self.status_message(mode);
        let bytes = message.as_bytes();
        device.write_report_exact(&bytes[..bytes.len().min(64)])
    }
}

impl Display {
    fn status_message(&self, mode: Mode) -> String {
        let unit = if self.fahrenheit { "F" } else { "C" };
        match mode {
            Mode::Cpu => format!(
                "HLXDATA({},{},0,0,{})\r\n",
                self.cpu.get_usage().unwrap_or(0),
                self.cpu.get_temp(self.fahrenheit).unwrap_or(0),
                unit,
            ),
            Mode::Gpu => format!(
                "HLXDATA({},{},0,0,{})\r\n",
                self.gpu.get_usage().unwrap_or(0),
                self.gpu.get_temp(self.fahrenheit).unwrap_or(0),
                unit,
            ),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(fahrenheit: bool) -> Display {
        Display {
            cpu: Cpu::new(),
            gpu: Gpu::absent(),
            fahrenheit,
        }
    }

    #[test]
    fn message_uses_the_hlxdata_framing() {
        let m = display(false).status_message(Mode::Cpu);
        assert!(m.starts_with("HLXDATA("), "{m}");
        assert!(m.ends_with(",C)\r\n"), "{m}");
        assert_eq!(m.matches(',').count(), 4, "{m}");
    }

    #[test]
    fn fahrenheit_switches_the_unit_letter() {
        assert!(display(true).status_message(Mode::Gpu).ends_with(",F)\r\n"));
    }

    #[test]
    fn message_always_fits_a_single_report() {
        assert!(display(false).status_message(Mode::Cpu).len() <= 64);
    }

    #[test]
    fn the_message_is_sent_unpadded() {
        // The released version writes exactly the ASCII bytes; padding a text
        // protocol out to 64 bytes changes the transfer length on the wire.
        struct Capture(Vec<u8>);
        impl crate::device::Device for Capture {
            fn write_report(&mut self, d: &[u8; 64]) -> crate::error::Result<()> {
                self.0 = d.to_vec();
                Ok(())
            }
            fn write_report_exact(&mut self, d: &[u8]) -> crate::error::Result<()> {
                self.0 = d.to_vec();
                Ok(())
            }
            fn write_bulk(&mut self, _d: &[u8]) -> crate::error::Result<()> {
                Ok(())
            }
            fn read(&mut self, _b: &mut [u8]) -> crate::error::Result<usize> {
                Ok(0)
            }
            fn transport(&self) -> crate::device::Transport {
                crate::device::Transport::Hid
            }
        }

        let mut dev = Capture(Vec::new());
        let mut d = display(false);
        d.frame(&mut dev, Mode::Cpu).unwrap();

        let expected = d.status_message(Mode::Cpu);
        assert_eq!(dev.0.len(), expected.len(), "packet must not be zero-padded");
        assert_eq!(dev.0, expected.as_bytes());
        assert!(dev.0.ends_with(b")\r\n"));
    }
}
