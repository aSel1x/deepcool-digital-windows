//! Display module for:
//! - CH360 DIGITAL
//! - CH560 DIGITAL
//! - MORPHEUS

use super::{bar_level, digits3, Ctx, DisplayDriver, Mode};
use crate::device::Device;
use crate::error::Result;
use crate::monitor::{Cpu, Gpu};

const MODES: &[Mode] = &[Mode::CpuTemperature, Mode::CpuUsage, Mode::Auto];
/// What the GPU half accepts. The old constructor rejected anything else with
/// `support_error_secondary`; without this the value silently blanked it.
const SECONDARY_MODES: &[Mode] = &[Mode::GpuTemperature, Mode::GpuUsage];

pub struct Display {
    cpu: Cpu,
    gpu: Gpu,
    secondary: Mode,
    fahrenheit: bool,
}

pub fn build(ctx: Ctx) -> Result<Box<dyn DisplayDriver>> {
    // Default means "mirror the primary metric on the GPU half".
    let secondary = match ctx.secondary {
        Mode::Default => Mode::Auto,
        other => other,
    };
    Ok(Box::new(Display {
        cpu: ctx.cpu,
        gpu: ctx.gpu,
        secondary,
        fahrenheit: ctx.fahrenheit,
    }))
}

impl DisplayDriver for Display {
    fn modes(&self) -> &'static [Mode] {
        MODES
    }

    fn secondary_modes(&self) -> &'static [Mode] {
        SECONDARY_MODES
    }

    fn warn(&self, mode: Mode) {
        if matches!(mode, Mode::CpuTemperature | Mode::Auto) {
            self.cpu.warn_temp();
        }
        self.gpu.warn_missing();
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
    fn status_message(&self, mode: Mode) -> [u8; 64] {
        let mut data = [0u8; 64];
        data[0] = 16;

        let cpu_usage = self.cpu.get_usage().unwrap_or(0);
        let gpu_usage = self.gpu.get_usage().unwrap_or(0);
        let unit = if self.fahrenheit { 35 } else { 19 };

        match mode {
            Mode::CpuTemperature => {
                let cpu_temp = digits3(self.cpu.get_temp(self.fahrenheit).unwrap_or(0) as u16);
                data[1] = unit;
                data[3] = cpu_temp[0];
                data[4] = cpu_temp[1];
                data[5] = cpu_temp[2];
                if self.secondary == Mode::Auto {
                    let gpu_temp = digits3(self.gpu.get_temp(self.fahrenheit).unwrap_or(0) as u16);
                    data[6] = unit;
                    data[8] = gpu_temp[0];
                    data[9] = gpu_temp[1];
                    data[10] = gpu_temp[2];
                }
            }
            Mode::CpuUsage => {
                let d = digits3(cpu_usage as u16);
                data[1] = 76;
                data[3] = d[0];
                data[4] = d[1];
                data[5] = d[2];
                if self.secondary == Mode::Auto {
                    let g = digits3(gpu_usage as u16);
                    data[6] = 76;
                    data[8] = g[0];
                    data[9] = g[1];
                    data[10] = g[2];
                }
            }
            _ => (),
        }

        if data[6] == 0 {
            match self.secondary {
                Mode::GpuTemperature => {
                    let g = digits3(self.gpu.get_temp(self.fahrenheit).unwrap_or(0) as u16);
                    data[6] = unit;
                    data[8] = g[0];
                    data[9] = g[1];
                    data[10] = g[2];
                }
                Mode::GpuUsage => {
                    let g = digits3(gpu_usage as u16);
                    data[6] = 76;
                    data[8] = g[0];
                    data[9] = g[1];
                    data[10] = g[2];
                }
                _ => (),
            }
        }

        data[2] = bar_level(cpu_usage);
        data[7] = bar_level(gpu_usage);

        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(secondary: Mode, fahrenheit: bool) -> Display {
        Display {
            cpu: Cpu::new(),
            gpu: Gpu::absent(),
            secondary,
            fahrenheit,
        }
    }

    #[test]
    fn mirrors_the_primary_metric_when_secondary_is_auto() {
        let pkt = display(Mode::Auto, false).status_message(Mode::CpuTemperature);
        assert_eq!(pkt[1], 19, "primary unit");
        assert_eq!(pkt[6], 19, "secondary mirrors the unit");
    }

    #[test]
    fn an_explicit_secondary_overrides_the_mirror() {
        let pkt = display(Mode::GpuUsage, false).status_message(Mode::CpuTemperature);
        assert_eq!(pkt[1], 19);
        assert_eq!(pkt[6], 76, "secondary shows usage");
    }

    #[test]
    fn status_bars_stay_within_one_to_ten() {
        let pkt = display(Mode::Auto, false).status_message(Mode::CpuUsage);
        assert!((1..=10).contains(&pkt[2]), "cpu bar {}", pkt[2]);
        assert!((1..=10).contains(&pkt[7]), "gpu bar {}", pkt[7]);
    }
}
