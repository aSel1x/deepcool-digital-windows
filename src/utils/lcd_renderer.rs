//! Базовый LCD-рендерер.
//!
//! Поддерживаемые форматы: SVG, PNG, JPG, BMP, GIF.
//! SVG — рендеринг с подстановкой метрик.
//! PNG/JPG/BMP — статичное изображение (ресайз до spec).
//! GIF — анимация (кадры с задержками).
//!
//! # Что делается один раз, а что — каждый кадр
//!
//! Всё дорогое здесь стартовое: шрифтовая база, `usvg::Options`, разбор
//! шаблона, ресайз растровых кадров. Раньше первые три пересобирались на
//! каждом кадре — измеренные 7.3 мс из 13.6 мс кадра уходили только на
//! `load_system_fonts()`. Теперь per-frame остаются подстановка значений,
//! разбор получившегося SVG и собственно растеризация.

use crate::config::AppConfig;
use crate::error::{Error, Result};
use crate::monitor::Sample;
use image::RgbaImage;
use resvg::{tiny_skia, usvg};
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Спецификация LCD-устройства.
#[derive(Debug, Clone, Copy)]
pub struct LcdDeviceSpec {
    pub width: u32,
    pub height: u32,
    pub bar_track_width: u32,
}

/// How a raster source is mapped onto the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Fill the panel, distorting the aspect ratio (the historical behaviour).
    #[default]
    Stretch,
    /// Scale to fit, letterboxing the remainder.
    Contain,
    /// Scale to fill, cropping the overflow.
    Cover,
}

impl Fit {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            None | Some("stretch") => Fit::Stretch,
            Some("contain") => Fit::Contain,
            Some("cover") => Fit::Cover,
            Some(other) => {
                crate::warning!(
                    "lcd.fit = \"{other}\" is not recognized (use stretch, contain or cover), using stretch"
                );
                Fit::Stretch
            }
        }
    }
}

/// Источник контента для LCD.
pub enum LcdAsset {
    Svg {
        template: crate::utils::svg_template::SvgTemplate,
        spec: LcdDeviceSpec,
    },
    /// Already scaled to the panel at load time.
    Static { frame: Framebuffer },
    /// Frames are pre-composited and pre-scaled; the loop only pushes bytes.
    Animated { frames: Vec<(Framebuffer, Duration)> },
}

impl LcdAsset {
    pub fn load(source: &Path, spec: LcdDeviceSpec, fit: Fit) -> Result<Self> {
        let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        match ext.as_str() {
            "svg" => Ok(LcdAsset::Svg {
                template: crate::utils::svg_template::SvgTemplate::load(source)?,
                spec,
            }),
            "png" | "jpg" | "jpeg" | "bmp" => {
                let img = image::open(source)
                    .map_err(|e| Error::Asset(format!("failed to load image {}: {e}", source.display())))?
                    .into_rgba8();
                Ok(LcdAsset::Static {
                    frame: image_to_framebuffer(&img, spec.width, spec.height, fit),
                })
            }
            "gif" => {
                let frames = load_gif(source)?
                    .into_iter()
                    // Scale once here rather than running Lanczos3 on every
                    // iteration of the animation loop.
                    .map(|(img, delay)| (image_to_framebuffer(&img, spec.width, spec.height, fit), delay))
                    .collect();
                Ok(LcdAsset::Animated { frames })
            }
            "" => Err(Error::Asset(format!(
                "LCD source {} has no file extension; expected .svg, .png, .jpg, .bmp or .gif",
                source.display()
            ))),
            other => Err(Error::Asset(format!(
                "unsupported LCD source format: .{other} (expected .svg, .png, .jpg, .bmp or .gif)"
            ))),
        }
    }

    /// Renders the SVG variant. Raster variants carry their frames already.
    pub fn render_svg(&self, ctx: &impl Serialize) -> Result<Framebuffer> {
        match self {
            LcdAsset::Svg { template, spec } => {
                let svg = template.render(ctx)?;
                let mut fb = Framebuffer::new(spec.width as usize, spec.height as usize);
                rasterize_svg_into(&svg, &mut fb)?;
                Ok(fb)
            }
            _ => Err(Error::Asset("asset is not an SVG template".to_string())),
        }
    }
}

/// Метрики, попадающие в шаблон. `None` — сенсор недоступен.
#[derive(Debug, Clone, Copy, Default)]
pub struct LcdMetrics {
    pub sample: Sample,
}

impl LcdMetrics {
    /// Конвертирует метрики в контекст для MiniJinja-шаблона.
    ///
    /// Числовые поля всегда присутствуют (отсутствующий сенсор даёт 0), а
    /// параллельная карта `valid` говорит, было ли значение реально измерено:
    /// `{% if valid.cpu_temp %}{{ cpu_temp }}{% else %}--{% endif %}`.
    /// Без неё «датчик отсутствует» и «ровно ноль» неразличимы на экране.
    pub fn into_context(self, spec: &LcdDeviceSpec, clock: String) -> LcdContext {
        let s = self.sample;
        LcdContext {
            cpu_temp: s.cpu_temp.unwrap_or(0),
            gpu_temp: s.gpu_temp.unwrap_or(0),
            vcore: s.vcore.unwrap_or(0),
            liquid_temp: s.liquid_temp.unwrap_or(0),
            liquid_fan_rpm: s.pump_fan_rpm.unwrap_or(0),
            cpu_fan_rpm: s.cpu_fan_rpm.unwrap_or(0),
            cpu_usage: s.cpu_usage.unwrap_or(0),
            vram_usage: s.vram_usage.unwrap_or(0),
            gpu_usage: s.gpu_usage.unwrap_or(0),
            ram_usage: s.ram_usage.unwrap_or(0),
            sys_fan_rpm: s.sys_fan_rpm.unwrap_or(0),
            cpu_power: s.cpu_power.unwrap_or(0),
            gpu_power: s.gpu_power.unwrap_or(0),
            cpu_freq: s.cpu_freq.unwrap_or(0),
            gpu_freq: s.gpu_freq.unwrap_or(0),
            clock,
            bar_track_width: spec.bar_track_width,
            valid: Validity {
                cpu_temp: s.cpu_temp.is_some(),
                gpu_temp: s.gpu_temp.is_some(),
                vcore: s.vcore.is_some(),
                liquid_temp: s.liquid_temp.is_some(),
                liquid_fan_rpm: s.pump_fan_rpm.is_some(),
                cpu_fan_rpm: s.cpu_fan_rpm.is_some(),
                cpu_usage: s.cpu_usage.is_some(),
                vram_usage: s.vram_usage.is_some(),
                gpu_usage: s.gpu_usage.is_some(),
                ram_usage: s.ram_usage.is_some(),
                sys_fan_rpm: s.sys_fan_rpm.is_some(),
                cpu_power: s.cpu_power.is_some(),
                gpu_power: s.gpu_power.is_some(),
                cpu_freq: s.cpu_freq.is_some(),
                gpu_freq: s.gpu_freq.is_some(),
            },
        }
    }
}

/// Контекст для MiniJinja-шаблона.
#[derive(Serialize, Debug, Clone)]
pub struct LcdContext {
    pub cpu_temp: u8,
    pub gpu_temp: u8,
    pub vcore: u16,
    pub liquid_temp: u8,
    pub liquid_fan_rpm: u16,
    pub cpu_fan_rpm: u16,
    pub cpu_usage: u8,
    pub vram_usage: u8,
    pub gpu_usage: u8,
    pub ram_usage: u8,
    pub sys_fan_rpm: u16,
    pub cpu_power: u16,
    pub gpu_power: u16,
    pub cpu_freq: u16,
    pub gpu_freq: u16,
    pub clock: String,
    pub bar_track_width: u32,
    pub valid: Validity,
}

/// Which of the numeric fields were actually measured this frame.
#[derive(Serialize, Debug, Clone, Copy)]
pub struct Validity {
    pub cpu_temp: bool,
    pub gpu_temp: bool,
    pub vcore: bool,
    pub liquid_temp: bool,
    pub liquid_fan_rpm: bool,
    pub cpu_fan_rpm: bool,
    pub cpu_usage: bool,
    pub vram_usage: bool,
    pub gpu_usage: bool,
    pub ram_usage: bool,
    pub sys_fan_rpm: bool,
    pub cpu_power: bool,
    pub gpu_power: bool,
    pub cpu_freq: bool,
    pub gpu_freq: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };

    pub fn to_rgb565(self) -> u16 {
        ((self.r as u16 >> 3) << 11) | ((self.g as u16 >> 2) << 5) | (self.b as u16 >> 3)
    }
}

#[derive(Clone)]
pub struct Framebuffer {
    width: usize,
    height: usize,
    pixels: Vec<u16>,
    /// Reused wire buffer, so a 153,600-byte Vec is not allocated per frame.
    wire: Vec<u8>,
}

impl Framebuffer {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width * height],
            wire: vec![0; width * height * 2],
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn clear(&mut self, color: Color) {
        self.pixels.fill(color.to_rgb565());
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x < self.width && y < self.height {
            self.pixels[y * self.width + x] = color.to_rgb565();
        }
    }

    /// Loads premultiplied RGBA (as produced by `tiny_skia::Pixmap`) by
    /// compositing it over the framebuffer's current contents.
    ///
    /// Previously the RGB bytes were taken as-is, which composites every
    /// antialiased edge against black regardless of what was underneath.
    pub fn blend_premultiplied(&mut self, rgba: &[u8], w: usize, h: usize) -> Result<()> {
        if w != self.width || h != self.height {
            return Err(Error::Asset(format!(
                "RGBA size mismatch: expected {}x{}, got {w}x{h}",
                self.width, self.height
            )));
        }
        if rgba.len() != w * h * 4 {
            return Err(Error::Asset(format!(
                "RGBA buffer length mismatch: expected {}, got {}",
                w * h * 4,
                rgba.len()
            )));
        }
        for (i, chunk) in rgba.as_chunks::<4>().0.iter().enumerate() {
            let (sr, sg, sb, sa) = (chunk[0] as u32, chunk[1] as u32, chunk[2] as u32, chunk[3] as u32);
            let dst = self.pixels[i];
            // Source is premultiplied: out = src + dst * (1 - alpha).
            let inv = 255 - sa;
            let dr = ((dst >> 11) & 0x1F) as u32 * 255 / 31;
            let dg = ((dst >> 5) & 0x3F) as u32 * 255 / 63;
            let db = (dst & 0x1F) as u32 * 255 / 31;
            self.pixels[i] = Color {
                r: (sr + dr * inv / 255).min(255) as u8,
                g: (sg + dg * inv / 255).min(255) as u8,
                b: (sb + db * inv / 255).min(255) as u8,
            }
            .to_rgb565();
        }
        Ok(())
    }

    /// The framebuffer as little-endian RGB565 bytes, as expected on the wire.
    ///
    /// Borrows an internal buffer instead of allocating one per frame.
    pub fn as_bytes(&mut self) -> &[u8] {
        for (i, pixel) in self.pixels.iter().enumerate() {
            let [lo, hi] = pixel.to_le_bytes();
            self.wire[i * 2] = lo;
            self.wire[i * 2 + 1] = hi;
        }
        &self.wire
    }
}

// ── GIF loading ─────────────────────────────────────────────────────────

/// Decodes a GIF into fully composited frames.
///
/// The previous implementation drew each frame's sub-rectangle onto a fresh
/// black canvas using only the *global* palette, and ignored the disposal
/// method — so every frame of an optimised GIF (i.e. almost all of them) came
/// out mostly black, and a frame with a local colour table came out entirely
/// black while still reporting success.
fn load_gif(path: &Path) -> Result<Vec<(RgbaImage, Duration)>> {
    let file = fs::File::open(path).map_err(|e| Error::Asset(format!("failed to open {}: {e}", path.display())))?;
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::Indexed);
    let mut reader = opts
        .read_info(file)
        .map_err(|e| Error::Asset(format!("GIF decode: {e}")))?;

    let w = reader.width() as u32;
    let h = reader.height() as u32;
    let global_pal = reader.global_palette().map(<[u8]>::to_vec);

    let mut frames = Vec::new();
    let mut canvas = RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));

    while let Some(frame) = reader
        .read_next_frame()
        .map_err(|e| Error::Asset(format!("GIF read: {e}")))?
    {
        // A local colour table takes precedence over the global one.
        let palette = frame.palette.as_deref().or(global_pal.as_deref()).ok_or_else(|| {
            Error::Asset(format!(
                "GIF {} has neither a global nor a local colour palette",
                path.display()
            ))
        })?;

        let saved = matches!(frame.dispose, gif::DisposalMethod::Previous).then(|| canvas.clone());

        let (fw, fh) = (frame.width as u32, frame.height as u32);
        let (fx, fy) = (frame.left as u32, frame.top as u32);
        for y in 0..fh {
            for x in 0..fw {
                let (px, py) = (fx + x, fy + y);
                if px >= w || py >= h {
                    continue;
                }
                let idx = frame.buffer[(y * fw + x) as usize];
                if Some(idx) == frame.transparent {
                    continue;
                }
                // `idx as usize * 3`, not `idx * 3`: the latter is u8 arithmetic
                // and wraps for any palette index >= 86.
                let base = idx as usize * 3;
                let (Some(&r), Some(&g), Some(&b)) = (palette.get(base), palette.get(base + 1), palette.get(base + 2))
                else {
                    continue;
                };
                canvas.put_pixel(px, py, image::Rgba([r, g, b, 255]));
            }
        }

        frames.push((canvas.clone(), gif_delay(frame.delay)));

        match frame.dispose {
            gif::DisposalMethod::Background => {
                for y in fy..(fy + fh).min(h) {
                    for x in fx..(fx + fw).min(w) {
                        canvas.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                    }
                }
            }
            gif::DisposalMethod::Previous => {
                if let Some(prev) = saved {
                    canvas = prev;
                }
            }
            // Keep / Any: leave the canvas as it is.
            _ => {}
        }
    }

    if frames.is_empty() {
        Err(Error::Asset("GIF has no frames".to_string()))
    } else {
        Ok(frames)
    }
}

/// GIF delay is in hundredths of a second. 0 (and 1) mean "as fast as possible",
/// which encoders emit routinely; browsers clamp these to 100 ms and so do we,
/// otherwise the USB write loop spins as fast as the device will ACK.
fn gif_delay(raw: u16) -> Duration {
    let cs = if raw <= 1 { 10 } else { raw };
    Duration::from_millis(cs as u64 * 10)
}

// ── Image helpers ───────────────────────────────────────────────────────

/// Scales `img` onto a `width`x`height` framebuffer using the requested fit.
pub fn image_to_framebuffer(img: &RgbaImage, width: u32, height: u32, fit: Fit) -> Framebuffer {
    let mut fb = Framebuffer::new(width as usize, height as usize);
    fb.clear(Color::BLACK);

    let (src_w, src_h) = (img.width().max(1), img.height().max(1));
    let (target_w, target_h) = match fit {
        Fit::Stretch => (width, height),
        Fit::Contain | Fit::Cover => {
            let sx = width as f64 / src_w as f64;
            let sy = height as f64 / src_h as f64;
            let scale = if fit == Fit::Contain { sx.min(sy) } else { sx.max(sy) };
            (
                ((src_w as f64 * scale).round() as u32).max(1),
                ((src_h as f64 * scale).round() as u32).max(1),
            )
        }
    };

    let resized = image::imageops::resize(img, target_w, target_h, image::imageops::FilterType::Lanczos3);

    // Centre the scaled image; Cover overflows and is cropped by set_pixel.
    let off_x = (width as i64 - target_w as i64) / 2;
    let off_y = (height as i64 - target_h as i64) / 2;

    for (x, y, pixel) in resized.enumerate_pixels() {
        let dx = off_x + x as i64;
        let dy = off_y + y as i64;
        if dx < 0 || dy < 0 {
            continue;
        }
        fb.set_pixel(
            dx as usize,
            dy as usize,
            Color {
                r: pixel[0],
                g: pixel[1],
                b: pixel[2],
            },
        );
    }
    fb
}

// ── SVG rendering ───────────────────────────────────────────────────────

/// Font database and render options, built once.
///
/// `load_system_fonts()` alone measured 7.3 ms warm and 102 ms cold on a machine
/// with 370 faces — it was previously paid on every frame.
fn svg_options() -> &'static usvg::Options<'static> {
    static OPTIONS: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let mut fontdb = usvg::fontdb::Database::new();
        fontdb.load_system_fonts();
        load_custom_fonts(&mut fontdb);

        #[cfg(target_os = "windows")]
        let (sans, serif, mono) = ("Segoe UI", "Times New Roman", "Consolas");
        #[cfg(not(target_os = "windows"))]
        let (sans, serif, mono) = ("DejaVu Sans", "DejaVu Serif", "DejaVu Sans Mono");

        fontdb.set_sans_serif_family(sans);
        fontdb.set_serif_family(serif);
        fontdb.set_monospace_family(mono);

        crate::debug!("font database built: {} faces", fontdb.len());

        usvg::Options {
            font_family: sans.to_string(),
            fontdb: Arc::new(fontdb),
            ..usvg::Options::default()
        }
    })
}

/// Rasterises `svg` into an existing framebuffer.
pub fn rasterize_svg_into(svg: &str, fb: &mut Framebuffer) -> Result<()> {
    let (w, h) = (fb.width() as u32, fb.height() as u32);
    // usvg skips XML comments per the spec, so no pre-stripping is needed. The
    // old helper also silently truncated the document after an unterminated one.
    let tree =
        usvg::Tree::from_str(svg, svg_options()).map_err(|e| Error::Asset(format!("failed to parse SVG: {e}")))?;
    let mut pixmap =
        tiny_skia::Pixmap::new(w, h).ok_or_else(|| Error::Asset(format!("failed to allocate {w}x{h} pixmap")))?;
    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
    fb.clear(Color::BLACK);
    fb.blend_premultiplied(pixmap.data(), w as usize, h as usize)
}

fn load_custom_fonts(fontdb: &mut usvg::fontdb::Database) {
    // Resolved against the config's directory, not the process CWD — otherwise
    // the bundled Saira faces silently fail to load for any normal install and
    // the template falls back to a system font with different metrics.
    // Both layouts are searched: the repo (configs/assets) and a packaged
    // release, where config.toml sits beside an `assets` directory.
    let configured = AppConfig::get().lcd.as_ref().and_then(|c| c.font_dir.clone());
    let dir = match &configured {
        Some(explicit) => crate::config::resolve_path(explicit),
        None => ["configs/assets", "assets"]
            .into_iter()
            .map(crate::config::resolve_path)
            .find(|p| p.is_dir())
            .unwrap_or_else(|| crate::config::resolve_path("configs/assets")),
    };

    let Ok(entries) = fs::read_dir(&dir) else {
        crate::warning!(
            "LCD font directory {} not found — bundled faces will not be available to templates",
            dir.display()
        );
        return;
    };

    let mut loaded = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc") {
            match fontdb.load_font_file(&path) {
                Ok(()) => loaded += 1,
                Err(e) => crate::warning!("failed to load font {}: {e}", path.display()),
            }
        }
    }
    if loaded == 0 {
        crate::warning!("no font files were loaded from {}", dir.display());
    } else {
        crate::debug!("loaded {loaded} custom font file(s) from {}", dir.display());
    }
}

/// Local wall-clock time as `HH:MM:SS`.
///
/// Previously this took `SystemTime::now() % 86400`, i.e. UTC, so the clock on
/// the panel was wrong for everyone outside UTC+0.
pub fn clock_string() -> String {
    let (h, m, s) = local_time();
    format!("{h:02}:{m:02}:{s:02}")
}

#[cfg(target_os = "linux")]
fn local_time() -> (u32, u32, u32) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            let d = secs.rem_euclid(86_400);
            return ((d / 3600) as u32, ((d % 3600) / 60) as u32, (d % 60) as u32);
        }
        (tm.tm_hour as u32, tm.tm_min as u32, tm.tm_sec as u32)
    }
}

#[cfg(windows)]
fn local_time() -> (u32, u32, u32) {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    (t.wHour as u32, t.wMinute as u32, t.wSecond as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb565_packs_the_expected_channels() {
        assert_eq!(Color { r: 0, g: 0, b: 0 }.to_rgb565(), 0x0000);
        assert_eq!(Color { r: 255, g: 255, b: 255 }.to_rgb565(), 0xFFFF);
        assert_eq!(Color { r: 255, g: 0, b: 0 }.to_rgb565(), 0xF800);
        assert_eq!(Color { r: 0, g: 255, b: 0 }.to_rgb565(), 0x07E0);
        assert_eq!(Color { r: 0, g: 0, b: 255 }.to_rgb565(), 0x001F);
    }

    #[test]
    fn wire_bytes_are_little_endian_and_the_right_length() {
        let mut fb = Framebuffer::new(2, 1);
        fb.set_pixel(0, 0, Color { r: 255, g: 0, b: 0 }); // 0xF800
        fb.set_pixel(1, 0, Color { r: 0, g: 0, b: 255 }); // 0x001F
        assert_eq!(fb.as_bytes(), &[0x00, 0xF8, 0x1F, 0x00]);
    }

    #[test]
    fn full_frame_is_the_expected_size() {
        let mut fb = Framebuffer::new(320, 240);
        assert_eq!(fb.as_bytes().len(), 153_600);
    }

    #[test]
    fn opaque_source_replaces_the_background() {
        let mut fb = Framebuffer::new(1, 1);
        fb.clear(Color { r: 255, g: 0, b: 0 });
        // Premultiplied opaque white.
        fb.blend_premultiplied(&[255, 255, 255, 255], 1, 1).unwrap();
        assert_eq!(fb.pixels[0], 0xFFFF);
    }

    #[test]
    fn fully_transparent_source_keeps_the_background() {
        let mut fb = Framebuffer::new(1, 1);
        fb.clear(Color { r: 0, g: 0, b: 255 });
        fb.blend_premultiplied(&[0, 0, 0, 0], 1, 1).unwrap();
        assert_eq!(fb.pixels[0], Color { r: 0, g: 0, b: 255 }.to_rgb565());
    }

    #[test]
    fn size_mismatch_is_reported_not_silently_ignored() {
        let mut fb = Framebuffer::new(2, 2);
        assert!(fb.blend_premultiplied(&[0; 4], 1, 1).is_err());
        assert!(fb.blend_premultiplied(&[0; 4], 2, 2).is_err());
    }

    #[test]
    fn gif_delay_clamps_the_as_fast_as_possible_encoding() {
        assert_eq!(gif_delay(0), Duration::from_millis(100));
        assert_eq!(gif_delay(1), Duration::from_millis(100));
        assert_eq!(gif_delay(5), Duration::from_millis(50));
        assert_eq!(gif_delay(100), Duration::from_millis(1000));
    }

    #[test]
    fn contain_letterboxes_and_cover_crops() {
        // A 2:1 source onto a 4:3 panel.
        let src = RgbaImage::from_pixel(200, 100, image::Rgba([255, 255, 255, 255]));

        let contain = image_to_framebuffer(&src, 320, 240, Fit::Contain);
        // Top rows stay background (black) because the image is letterboxed.
        assert_eq!(contain.pixels[0], 0x0000);
        // Centre is the image.
        assert_eq!(contain.pixels[120 * 320 + 160], 0xFFFF);

        let cover = image_to_framebuffer(&src, 320, 240, Fit::Cover);
        // Cover fills every pixel, including the corner.
        assert_eq!(cover.pixels[0], 0xFFFF);

        let stretch = image_to_framebuffer(&src, 320, 240, Fit::Stretch);
        assert_eq!(stretch.pixels[0], 0xFFFF);
    }

    #[test]
    fn fit_parsing_falls_back_with_a_warning() {
        assert_eq!(Fit::parse(None), Fit::Stretch);
        assert_eq!(Fit::parse(Some("contain")), Fit::Contain);
        assert_eq!(Fit::parse(Some(" COVER ")), Fit::Cover);
        assert_eq!(Fit::parse(Some("nonsense")), Fit::Stretch);
    }

    /// End-to-end: the shipped template goes through the real load/render path
    /// and produces a full-size framebuffer. This is the check that catches a
    /// template referencing a variable the context does not provide, a font that
    /// fails to load, or an SVG that stops parsing halfway.
    #[test]
    fn the_shipped_dashboard_renders_to_a_full_framebuffer() {
        let spec = LcdDeviceSpec {
            width: 320,
            height: 240,
            bar_track_width: 296,
        };
        let path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/configs/svg-templates/lm360-dashboard.svg"
        ));
        let asset = LcdAsset::load(path, spec, Fit::Stretch).expect("shipped template must load");

        let ctx = LcdMetrics {
            sample: Sample {
                cpu_temp: Some(62),
                cpu_usage: Some(37),
                gpu_temp: Some(55),
                gpu_usage: Some(22),
                ram_usage: Some(58),
                vram_usage: Some(41),
                vcore: Some(1234),
                cpu_fan_rpm: Some(1200),
                ..Sample::default()
            },
        }
        .into_context(&spec, "12:34:56".to_string());

        let mut fb = asset.render_svg(&ctx).expect("shipped template must render");
        assert_eq!(fb.width(), 320);
        assert_eq!(fb.height(), 240);
        assert_eq!(fb.as_bytes().len(), 153_600);

        // A blank frame would mean the SVG parsed but drew nothing.
        let lit = fb.pixels.iter().filter(|p| **p != 0).count();
        assert!(lit > 500, "rendered frame is essentially empty ({lit} non-black pixels)");
    }

    #[test]
    fn a_template_asset_rejects_raster_only_operations() {
        let spec = LcdDeviceSpec {
            width: 8,
            height: 8,
            bar_track_width: 8,
        };
        let img = RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255]));
        let asset = LcdAsset::Static {
            frame: image_to_framebuffer(&img, spec.width, spec.height, Fit::Contain),
        };
        assert!(asset.render_svg(&()).is_err());
    }

    #[test]
    fn unknown_and_missing_extensions_are_reported_clearly() {
        let spec = LcdDeviceSpec {
            width: 8,
            height: 8,
            bar_track_width: 8,
        };
        let Err(err) = LcdAsset::load(std::path::Path::new("/nonexistent/x.webp"), spec, Fit::Stretch) else {
            panic!("an unknown extension must be rejected");
        };
        assert!(format!("{err}").contains("unsupported"), "{err}");

        let Err(err) = LcdAsset::load(std::path::Path::new("/nonexistent/noext"), spec, Fit::Stretch) else {
            panic!("a missing extension must be rejected");
        };
        assert!(format!("{err}").contains("no file extension"), "{err}");
    }

    #[test]
    fn clock_is_a_well_formed_local_time() {
        let s = clock_string();
        assert_eq!(s.len(), 8, "{s}");
        let parts: Vec<u32> = s.split(':').map(|p| p.parse().unwrap()).collect();
        assert!(parts[0] < 24 && parts[1] < 60 && parts[2] < 60, "{s}");
    }
}
