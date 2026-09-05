//! LM Series LCD Display (320x240 RGB565 graphical screen)

use super::{Ctx, DisplayDriver, Mode};
use crate::config::AppConfig;
use crate::device::{Device, Transport};
use crate::error::{Error, Result};
use crate::monitor::{Cpu, Gpu, Ram, Sample, SysFan};
use crate::utils::crc::compute_crc16;
use crate::utils::lcd_renderer::{clock_string, Fit, Framebuffer, LcdAsset, LcdDeviceSpec, LcdMetrics};
use std::time::Duration;

pub const HEADER: &[u8] = &[
    0xaa, 0x08, 0x00, 0x00, 0x01, 0x00, 0x58, 0x02, 0x00, 0x2c, 0x01, 0xbc, 0x11,
];
const INIT_QUERY: &[u8] = &[0xaa, 0x01, 0x00, 0x09, 0x29, 0x91];

/// ACK timeout used while blanking the panel on the way out.
const SHUTDOWN_ACK_TIMEOUT_MS: u32 = 1_000;

const MODES: &[Mode] = &[Mode::Auto];

/// LM360 orientation code.
fn orientation_from_rotation(deg: u16) -> u8 {
    match deg {
        0 => 0x01,
        90 => 0x02,
        180 => 0x03,
        270 => 0x04,
        other => {
            crate::warning!("lcd.rotation = {other} is not supported (use 0, 90, 180 or 270), using 0");
            0x01
        }
    }
}

pub struct Display {
    cpu: Cpu,
    gpu: Gpu,
    ram: Ram,
    sys_fan: SysFan,
    asset: LcdAsset,
    spec: LcdDeviceSpec,
    brightness: u8,
    orientation: u8,
    /// Scratch framebuffer, reused across frames.
    fb: Framebuffer,
    frame_index: usize,
}

pub fn build(ctx: Ctx) -> Result<Box<dyn DisplayDriver>> {
    let spec = LcdDeviceSpec {
        width: 320,
        height: 240,
        bar_track_width: 296,
    };

    let lcd_cfg = AppConfig::get().lcd.clone().unwrap_or_default();
    let brightness = lcd_cfg.brightness.unwrap_or(100).min(100);
    let rotation = lcd_cfg.rotation.unwrap_or(0);
    let fit = Fit::parse(lcd_cfg.fit.as_deref());

    let source = lcd_cfg.source.as_deref().ok_or_else(|| {
        Error::Config("lcd.source is not set in config.toml — an LCD device requires a template or image".to_string())
    })?;
    let source_path = crate::config::resolve_path(source);
    let asset = LcdAsset::load(&source_path, spec, fit)?;

    Ok(Box::new(Display {
        cpu: ctx.cpu,
        gpu: ctx.gpu,
        ram: ctx.ram,
        sys_fan: ctx.sys_fan,
        asset,
        spec,
        brightness,
        orientation: orientation_from_rotation(rotation),
        fb: Framebuffer::new(spec.width as usize, spec.height as usize),
        frame_index: 0,
    }))
}

impl Display {
    /// Отправляет config-пакет (яркость + ориентация) и читает ответ устройства.
    ///
    /// The ACK is consumed here and *only* here — `init` used to read a second
    /// time after each call, costing two extra 5-second timeouts at startup and
    /// attributing responses to the wrong command.
    fn send_config(device: &mut dyn Device, orientation: u8, brightness: u8) -> Result<()> {
        let mut pkt = [0xAAu8, 0x04, 0x00, 0x06, orientation, brightness, 0x00, 0x00, 0x00];
        let crc = compute_crc16(&pkt[..7]);
        pkt[7] = crc[0];
        pkt[8] = crc[1];

        crate::debug!("Sending LM config packet: {:02X?}", pkt);
        device.write_bulk(&pkt)?;
        Self::drain_ack(device, "config");
        Ok(())
    }

    /// Reads and logs one response packet. Never fatal: some firmware revisions
    /// stay silent for a command they consider a no-op.
    fn drain_ack(device: &mut dyn Device, what: &str) {
        let mut resp = [0u8; 64];
        match device.read(&mut resp) {
            Ok(n) => crate::debug!("LM {what} response ({n} bytes): {:02X?}", &resp[..n]),
            Err(e) => crate::warning!("LM {what}: no response ({e})"),
        }
    }

    fn send_frame(device: &mut dyn Device, fb: &mut Framebuffer) -> Result<()> {
        device.write_bulk(HEADER)?;
        device.write_bulk(fb.as_bytes())?;
        Self::drain_ack(device, "frame");
        Ok(())
    }

    /// Отправляет чёрный кадр, чтобы устройство завершило текущую транзакцию.
    fn send_blank_frame(&mut self, device: &mut dyn Device) {
        self.fb.clear(crate::utils::lcd_renderer::Color::BLACK);
        let _ = device.write_bulk(HEADER);
        let _ = device.write_bulk(self.fb.as_bytes());
        // Как и в обычном кадровом цикле, устройство присылает ответ на транзакцию —
        // не прочитав его перед закрытием хендла, можно оставить прошивку в состоянии
        // ожидания ACK, из-за чего дисплей гаснет и не реагирует до перезагрузки ПК.
        let mut resp = [0u8; 64];
        let _ = device.read(&mut resp);
        std::thread::sleep(Duration::from_millis(100));
    }

    fn collect_metrics(&self) -> LcdMetrics {
        LcdMetrics {
            sample: Sample {
                cpu_temp: self.cpu.get_temp(false),
                cpu_usage: self.cpu.get_usage(),
                cpu_power: None,
                cpu_freq: self.cpu.get_frequency(),
                cpu_fan_rpm: self.cpu.get_cpu_fan_rpm(),
                pump_fan_rpm: self.cpu.get_pump_fan_rpm(),
                liquid_temp: self.cpu.get_liquid_temp(false),
                vcore: self.cpu.get_vcore(),
                gpu_temp: self.gpu.get_temp(false),
                gpu_usage: self.gpu.get_usage(),
                gpu_power: self.gpu.get_power(),
                gpu_freq: self.gpu.get_frequency(),
                vram_usage: self.gpu.get_vram_usage(),
                ram_usage: self.ram.get_usage(),
                sys_fan_rpm: self.sys_fan.get_rpm(0),
            },
        }
    }
}

impl DisplayDriver for Display {
    fn modes(&self) -> &'static [Mode] {
        MODES
    }

    fn warn(&self, _mode: Mode) {
        self.cpu.warn_temp();
        self.ram.warn_missing();
    }

    fn init(&mut self, device: &mut dyn Device) -> Result<()> {
        // The framebuffer stream is an unframed bulk transfer. A HID backend
        // would silently eat the first byte of every 64-byte chunk, so refuse
        // here with an explanation rather than painting garbage.
        if device.transport() != Transport::Bulk {
            return Err(Error::Transfer(
                "the LM-series LCD needs a raw bulk interface (WinUSB); the HID backend cannot \
                 carry its framebuffer protocol"
                    .to_string(),
            ));
        }

        Self::send_config(device, self.orientation, self.brightness)?;

        crate::debug!("Sending LM init query: {:02X?}", INIT_QUERY);
        device.write_bulk(INIT_QUERY)?;
        Self::drain_ack(device, "init query");

        Self::send_config(device, self.orientation, self.brightness)?;
        Ok(())
    }

    fn frame(&mut self, device: &mut dyn Device, _mode: Mode) -> Result<()> {
        match &self.asset {
            LcdAsset::Svg { .. } => {
                let ctx = self.collect_metrics().into_context(&self.spec, clock_string());
                crate::debug!(
                    "LM metrics: cpu_temp={:?} gpu_temp={:?} cpu_usage={:?} ram_usage={:?}",
                    ctx.cpu_temp,
                    ctx.gpu_temp,
                    ctx.cpu_usage,
                    ctx.ram_usage
                );
                self.fb = self.asset.render_svg(&ctx)?;
            }
            LcdAsset::Static { frame } => {
                self.fb = frame.clone();
            }
            LcdAsset::Animated { frames } => {
                let (frame, _) = &frames[self.frame_index % frames.len()];
                self.fb = frame.clone();
                self.frame_index = self.frame_index.wrapping_add(1);
            }
        }
        Self::send_frame(device, &mut self.fb)
    }

    /// GIF sources pace themselves from their own frame delays.
    fn pace(&self) -> Option<Duration> {
        match &self.asset {
            LcdAsset::Animated { frames } if !frames.is_empty() => {
                // frame_index has already advanced past the frame just shown.
                let shown = (self.frame_index + frames.len() - 1) % frames.len();
                Some(frames[shown].1)
            }
            _ => None,
        }
    }

    fn shutdown(&mut self, device: &mut dyn Device) {
        // The console-close handler only gets ~5 s before Windows kills us, and
        // the normal ACK timeout is 5 s on its own.
        device.set_read_timeout(SHUTDOWN_ACK_TIMEOUT_MS);
        self.send_blank_frame(device);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_codes_match_the_protocol() {
        assert_eq!(orientation_from_rotation(0), 0x01);
        assert_eq!(orientation_from_rotation(90), 0x02);
        assert_eq!(orientation_from_rotation(180), 0x03);
        assert_eq!(orientation_from_rotation(270), 0x04);
        // Anything else falls back to 0 degrees with a warning.
        assert_eq!(orientation_from_rotation(45), 0x01);
    }

    #[test]
    fn the_frame_header_carries_a_valid_checksum() {
        // The header's trailing two bytes are its CRC; recomputing the body must
        // reproduce them. This is the regression guard for the CRC seed fix.
        let body = &HEADER[..HEADER.len() - 2];
        assert_eq!(compute_crc16(body), [HEADER[11], HEADER[12]]);
    }

    #[test]
    fn the_init_query_carries_a_valid_checksum() {
        let body = &INIT_QUERY[..INIT_QUERY.len() - 2];
        assert_eq!(compute_crc16(body), [INIT_QUERY[4], INIT_QUERY[5]]);
    }

    #[test]
    fn config_packets_checksum_the_first_seven_bytes() {
        let mut pkt = [0xAAu8, 0x04, 0x00, 0x06, 0x01, 100, 0x00, 0x00, 0x00];
        let crc = compute_crc16(&pkt[..7]);
        pkt[7] = crc[0];
        pkt[8] = crc[1];
        // Brightness 100 at 0 degrees, the documented --send example.
        assert_eq!(&pkt[..7], &[0xAA, 0x04, 0x00, 0x06, 0x01, 0x64, 0x00]);
    }
}
