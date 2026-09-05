use crate::error::{Error, Result};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

static CONFIG: OnceLock<AppConfig> = OnceLock::new();
/// Directory that the loaded `config.toml` lives in; relative asset paths resolve against it.
static CONFIG_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
/// Full path of the loaded config, for logging.
static CONFIG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub update_interval_ms: Option<u64>,
    pub debug: Option<bool>,
    pub device: Option<DeviceConfig>,
    pub lcd: Option<LcdConfig>,
    pub segment: Option<SegmentConfig>,
    /// Present on every platform even though only Windows reads it: with
    /// `deny_unknown_fields`, cfg-gating the field away made a Linux config
    /// carrying an `[lhm]` section — including the shipped example — a hard
    /// parse error.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub lhm: Option<LhmConfig>,
}

/// Explicit device selection, for machines with more than one DeepCool device.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DeviceConfig {
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    /// Which GPU to read, replacing the removed `--gpuid` flag.
    ///
    /// Either a PCI address (`0000:01:00.0`) or `vendor:index`
    /// (`nvidia:0`, `amd:1`, `intel:0`). Unset means auto-detect.
    /// Windows reads whatever LibreHardwareMonitor reports, so it is Linux-only.
    #[cfg_attr(windows, allow(dead_code))]
    pub gpu: Option<String>,
}

/// Конфигурация LCD-устройств (LM, MYSTIQUE).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LcdConfig {
    pub brightness: Option<u8>,
    pub rotation: Option<u16>,
    pub source: Option<String>,
    /// Directory scanned for extra .ttf/.otf faces. Defaults to `configs/assets`.
    pub font_dir: Option<String>,
    /// How a raster source is fitted to the panel: `stretch`, `contain` or `cover`.
    pub fit: Option<String>,
}

/// Конфигурация сегментных устройств (AK, AG, LS, CH, LP, ...).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SegmentConfig {
    pub mode: Option<String>,
    pub secondary: Option<String>,
    pub fahrenheit: Option<bool>,
    pub alarm: Option<bool>,
    /// Matrix rotation in degrees (0, 90, 180, 270). LP-series only.
    pub rotation: Option<u16>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct LhmConfig {
    pub url: Option<String>,
    pub auth_bearer: Option<String>,
    pub auth_basic_user: Option<String>,
    pub auth_basic_pass: Option<String>,
    /// Request timeout in milliseconds (default 2000).
    pub timeout_ms: Option<u64>,
}

impl AppConfig {
    /// Reads the config once and publishes it. Call exactly once, from `main`.
    ///
    /// A missing config file is not an error — defaults are used. A malformed
    /// one is, because silently ignoring a typo'd setting is worse than stopping.
    pub fn init(cli_path: Option<&Path>) -> Result<&'static AppConfig> {
        let path = match cli_path {
            Some(p) => {
                if !p.is_file() {
                    return Err(Error::Config(format!("config file not found: {}", p.display())));
                }
                Some(p.to_path_buf())
            }
            None => Self::discover(),
        };

        let cfg = match &path {
            Some(p) => {
                let text =
                    fs::read_to_string(p).map_err(|e| Error::Config(format!("failed to read {}: {e}", p.display())))?;
                toml::from_str(&text).map_err(|e| Error::Config(format!("failed to parse {}: {e}", p.display())))?
            }
            None => Self::default(),
        };

        let _ = CONFIG_DIR.set(path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)));
        let _ = CONFIG_PATH.set(path.clone());
        // `set` silently fails once the cell is populated, which would leave the
        // program running on defaults while reporting that it loaded a file.
        CONFIG
            .set(cfg)
            .map_err(|_| Error::Config("configuration was already initialised".to_string()))?;
        Ok(Self::get())
    }

    /// The loaded config, or defaults if [`AppConfig::init`] has not run.
    ///
    /// The initializer is `Self::default`, which never touches `CONFIG` — a
    /// closure that wrote back into this same `OnceLock` would deadlock here.
    pub fn get() -> &'static AppConfig {
        CONFIG.get_or_init(Self::default)
    }

    /// Path of the config file that was actually loaded, for logging.
    ///
    /// The real path, not `<dir>/config.toml` — `--config other-name.toml`
    /// would otherwise be reported under the wrong name.
    pub fn loaded_from() -> Option<PathBuf> {
        CONFIG_PATH.get().and_then(|p| p.clone())
    }

    /// Search order: `$DEEPCOOL_CONFIG`, CWD, user config dir, `/etc`, next to the executable.
    fn discover() -> Option<PathBuf> {
        Self::candidates().into_iter().find(|p| p.is_file())
    }

    fn candidates() -> Vec<PathBuf> {
        let mut out = Vec::new();

        if let Some(env) = std::env::var_os("DEEPCOOL_CONFIG") {
            out.push(PathBuf::from(env));
        }
        out.push(PathBuf::from("config.toml"));

        #[cfg(target_os = "linux")]
        {
            let xdg = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
            if let Some(dir) = xdg {
                out.push(dir.join("deepcool-digital/config.toml"));
            }
            out.push(PathBuf::from("/etc/deepcool-digital/config.toml"));
        }
        #[cfg(windows)]
        {
            if let Some(appdata) = std::env::var_os("APPDATA") {
                out.push(PathBuf::from(appdata).join("deepcool-digital/config.toml"));
            }
        }

        if let Some(dir) = exe_dir() {
            out.push(dir.join("config.toml"));
        }
        out
    }
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// Resolves a relative path from the config against the config's own directory,
/// falling back to the executable's directory and finally the working directory.
///
/// Every user-supplied path — templates, images, font directories — goes through
/// here, so "it works from the repo root but not when installed" cannot happen.
pub fn resolve_path<P: AsRef<Path>>(raw: P) -> PathBuf {
    let path = raw.as_ref();
    if path.is_absolute() {
        return path.to_path_buf();
    }

    let bases = [CONFIG_DIR.get().and_then(|d| d.clone()), exe_dir()];
    for base in bases.into_iter().flatten() {
        let candidate = base.join(path);
        if candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_example_config_parses_into_appconfig() {
        // Regression guard: `deny_unknown_fields` plus a cfg-gated `[lhm]`
        // field made every Linux config carrying an [lhm] section a hard error.
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/configs/example.toml")).unwrap();
        let cfg: AppConfig = toml::from_str(&text).expect("configs/example.toml must parse on every platform");
        assert_eq!(cfg.update_interval_ms, Some(1000));
        assert!(cfg.lcd.is_some());
        assert!(cfg.segment.is_some());
    }

    #[test]
    fn an_unknown_key_is_rejected() {
        let err = toml::from_str::<AppConfig>("nonsense_key = 1").unwrap_err();
        assert!(err.to_string().contains("nonsense_key"), "{err}");
    }

    #[test]
    fn an_lhm_section_is_accepted_on_every_platform() {
        let text = "[lhm]\nurl = \"http://127.0.0.1:8085/data.json\"\ntimeout_ms = 1500\n";
        toml::from_str::<AppConfig>(text).expect("[lhm] must parse on Linux too");
    }
}
