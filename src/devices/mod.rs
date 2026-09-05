//! Display drivers and the device table.
//!
//! Adding a device means adding one row to [`DEVICES`] and one driver. The
//! table is the single source of truth for which product ids exist, what each
//! one is called, which modes it can display, and which optional features it
//! has. Previously that information lived in four independent literal PID lists
//! in `main.rs` plus a `match` arm inside every driver's constructor, and they
//! drifted — which is how the AG series ended up being sent Fahrenheit values it
//! has no unit code for.

pub mod ag_series;
pub mod ak_pro_series;
pub mod ak_series;
pub mod ch510;
pub mod ch_series;
pub mod ch_series_gen2;
pub mod ld_series;
pub mod lm_series;
pub mod lp_series;
pub mod lq_series;
pub mod ls_series;

use crate::device::Device;
use crate::error::{Error, Result};
use crate::monitor::{Cpu, Gpu, Ram, SysFan};
use std::time::{Duration, Instant};

pub const DEFAULT_VENDOR_ID: u16 = 13875;
pub const CH510_VENDOR_ID: u16 = 13523;

/// Every product id this build knows about, named.
pub mod pid {
    pub const AK400: u16 = 1;
    pub const AK620: u16 = 2;
    pub const AK500: u16 = 3;
    pub const AK500S: u16 = 4;
    pub const CH560: u16 = 5;
    pub const LS: u16 = 6;
    pub const MORPHEUS: u16 = 7;
    pub const AG: u16 = 8;
    pub const LD: u16 = 10;
    pub const LP: u16 = 12;
    pub const LQ: u16 = 13;
    pub const AK620_G2: u16 = 15;
    pub const AK400_PRO: u16 = 16;
    pub const AK500_PRO: u16 = 17;
    pub const AK620_PRO: u16 = 18;
    pub const CH170: u16 = 19;
    pub const CH360: u16 = 21;
    pub const CH270: u16 = 22;
    pub const CH690: u16 = 27;
    pub const AK700: u16 = 31;
    pub const LM: u16 = 38;
    pub const ASSASSIN_IV: u16 = 41;
    pub const LQ_ALT: u16 = 42;
    pub const CH510: u16 = 4352;
}

pub const AUTO_MODE_INTERVAL: Duration = Duration::from_millis(5000);

/// Delay between init and the first frame, so rate metrics have a window.
const PRIME_WINDOW: Duration = Duration::from_millis(120);

/// Consecutive frame failures tolerated before giving up.
///
/// Previously any write error ended the loop; the refactor made every non-
/// `Disconnected` error retry indefinitely, which on the HID backend (where
/// `Disconnected` is never produced) meant an unplugged device spun forever.
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Mode {
    Default,
    Auto,
    CpuTemperature,
    CpuUsage,
    CpuPower,
    CpuFrequency,
    CpuFan,
    GpuTemperature,
    GpuUsage,
    GpuPower,
    Cpu,
    Gpu,
    Psu,
}

impl Mode {
    pub const fn symbol(&self) -> &'static str {
        match self {
            Mode::Default => "",
            Mode::Auto => "auto",
            Mode::CpuTemperature => "cpu_temp",
            Mode::CpuUsage => "cpu_usage",
            Mode::CpuPower => "cpu_power",
            Mode::CpuFrequency => "cpu_freq",
            Mode::CpuFan => "cpu_fan",
            Mode::GpuTemperature => "gpu_temp",
            Mode::GpuUsage => "gpu_usage",
            Mode::GpuPower => "gpu_power",
            Mode::Cpu => "cpu",
            Mode::Gpu => "gpu",
            Mode::Psu => "psu",
        }
    }

    pub fn get(symbol: &str) -> Option<Mode> {
        match symbol {
            "auto" => Some(Self::Auto),
            "cpu_temp" => Some(Self::CpuTemperature),
            "cpu_usage" => Some(Self::CpuUsage),
            "cpu_power" => Some(Self::CpuPower),
            "cpu_freq" => Some(Self::CpuFrequency),
            "cpu_fan" => Some(Self::CpuFan),
            "gpu_temp" => Some(Self::GpuTemperature),
            "gpu_usage" => Some(Self::GpuUsage),
            "gpu_power" => Some(Self::GpuPower),
            "cpu" => Some(Self::Cpu),
            "gpu" => Some(Self::Gpu),
            "psu" => Some(Self::Psu),
            _ => None,
        }
    }
}

/// Optional features a device either has or does not.
///
/// These are checked once, against the table, before anything is constructed —
/// so "AG has no Fahrenheit unit code" is a field, not a forgotten `if`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Caps {
    /// Overheat alarm byte in the packet.
    pub alarm: bool,
    /// Independent secondary display.
    pub secondary: bool,
    /// Software rotation of a dot-matrix panel.
    pub rotation: bool,
    /// A packet field selecting Fahrenheit.
    pub fahrenheit: bool,
}

/// Everything a driver may need to build itself.
pub struct Ctx {
    pub cpu: Cpu,
    pub gpu: Gpu,
    pub ram: Ram,
    pub sys_fan: SysFan,
    pub fahrenheit: bool,
    pub alarm: bool,
    pub rotation: u16,
    pub secondary: Mode,
}

/// A device driver: build the packets, let the runner own the loop.
pub trait DisplayDriver {
    /// Modes this device can display. The first entry is its default.
    fn modes(&self) -> &'static [Mode];

    /// Modes cycled by `auto`. Defaults to every mode except `auto` itself.
    fn auto_cycle(&self) -> &'static [Mode] {
        self.modes()
    }

    /// Modes accepted for the secondary display, if the device has one.
    fn secondary_modes(&self) -> &'static [Mode] {
        &[]
    }

    /// One-shot packets sent before the first frame.
    fn init(&mut self, _device: &mut dyn Device) -> Result<()> {
        Ok(())
    }

    /// Warnings about missing sensors, printed once, for the mode about to run.
    ///
    /// Takes the mode so a device does not warn about sensors the selected mode
    /// never reads — the pre-refactor drivers gated these individually.
    fn warn(&self, _mode: Mode) {}

    /// Builds and sends a single frame.
    fn frame(&mut self, device: &mut dyn Device, mode: Mode) -> Result<()>;

    /// Leaves the panel in a safe state before the process exits.
    fn shutdown(&mut self, _device: &mut dyn Device) {}

    /// How long to wait after this frame. `None` uses the configured interval.
    ///
    /// Only the animated LCD path overrides this, to honour GIF frame delays.
    fn pace(&self) -> Option<Duration> {
        None
    }
}

pub struct DeviceEntry {
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: &'static str,
    pub caps: Caps,
    pub build: fn(Ctx) -> Result<Box<dyn DisplayDriver>>,
}

impl Caps {
    pub const NONE: Caps = Caps {
        alarm: false,
        secondary: false,
        rotation: false,
        fahrenheit: false,
    };
}

const SEGMENT_CAPS: Caps = Caps {
    alarm: true,
    secondary: false,
    rotation: false,
    fahrenheit: true,
};
const PLAIN_CAPS: Caps = Caps {
    alarm: false,
    secondary: false,
    rotation: false,
    fahrenheit: true,
};
const SECONDARY_CAPS: Caps = Caps {
    alarm: false,
    secondary: true,
    rotation: false,
    fahrenheit: true,
};
const LP_CAPS: Caps = Caps {
    alarm: false,
    secondary: true,
    rotation: true,
    fahrenheit: true,
};
const AG_CAPS: Caps = Caps {
    alarm: true,
    secondary: false,
    rotation: false,
    fahrenheit: false,
};

/// The single source of truth for supported devices.
pub static DEVICES: &[DeviceEntry] = &[
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::AK400,
        name: "AK400 DIGITAL",
        caps: SEGMENT_CAPS,
        build: ak_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::AK620,
        name: "AK620 DIGITAL",
        caps: SEGMENT_CAPS,
        build: ak_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::AK500,
        name: "AK500 DIGITAL",
        caps: SEGMENT_CAPS,
        build: ak_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::AK500S,
        name: "AK500S DIGITAL",
        caps: SEGMENT_CAPS,
        build: ak_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::CH560,
        name: "CH560 DIGITAL",
        caps: SECONDARY_CAPS,
        build: ch_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::LS,
        name: "LS520/LS720 SE DIGITAL",
        caps: SEGMENT_CAPS,
        build: ls_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::MORPHEUS,
        name: "MORPHEUS",
        caps: SECONDARY_CAPS,
        build: ch_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::AG,
        name: "AG400/AG620 DIGITAL",
        // The AG protocol has unit code 19 (Celsius) and no Fahrenheit
        // counterpart — see device-list/tables/ag-series.md. Sending an F value
        // under the C code is what froze the panel at "99 °C".
        caps: AG_CAPS,
        build: ag_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::LD,
        name: "LD240/LD360",
        caps: PLAIN_CAPS,
        build: ld_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::LP,
        name: "LP240/LP360",
        caps: LP_CAPS,
        build: lp_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::LQ,
        name: "LQ240/LQ360",
        caps: PLAIN_CAPS,
        build: lq_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::AK620_G2,
        name: "AK620 G2 DIGITAL NYX",
        caps: PLAIN_CAPS,
        build: lq_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::AK400_PRO,
        name: "AK400 DIGITAL PRO",
        caps: PLAIN_CAPS,
        build: ak_pro_series::build_ak400,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::AK500_PRO,
        name: "AK500 DIGITAL PRO",
        caps: PLAIN_CAPS,
        build: ak_pro_series::build_ak620,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::AK620_PRO,
        name: "AK620 DIGITAL PRO",
        caps: PLAIN_CAPS,
        build: ak_pro_series::build_ak620,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::CH170,
        name: "CH170 DIGITAL",
        caps: PLAIN_CAPS,
        build: ch_series_gen2::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::CH360,
        name: "CH360 DIGITAL",
        caps: SECONDARY_CAPS,
        build: ch_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::CH270,
        name: "CH270 DIGITAL",
        caps: PLAIN_CAPS,
        build: ch_series_gen2::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::CH690,
        name: "CH690 DIGITAL",
        caps: PLAIN_CAPS,
        build: ch_series_gen2::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::AK700,
        name: "AK700 DIGITAL NYX",
        caps: PLAIN_CAPS,
        build: lq_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::LM,
        name: "LM-series LCD",
        caps: Caps::NONE,
        build: lm_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::ASSASSIN_IV,
        name: "ASSASSIN IV VC VISION",
        caps: PLAIN_CAPS,
        build: lq_series::build,
    },
    DeviceEntry {
        vendor_id: DEFAULT_VENDOR_ID,
        product_id: pid::LQ_ALT,
        name: "LQ240/LQ360",
        caps: PLAIN_CAPS,
        build: lq_series::build,
    },
    DeviceEntry {
        vendor_id: CH510_VENDOR_ID,
        product_id: pid::CH510,
        name: "CH510 MESH DIGITAL",
        caps: PLAIN_CAPS,
        build: ch510::build,
    },
];

pub fn lookup(vendor_id: u16, product_id: u16) -> Option<&'static DeviceEntry> {
    DEVICES
        .iter()
        .find(|d| d.vendor_id == vendor_id && d.product_id == product_id)
}

/// Every vendor id the table mentions, in table order.
pub fn vendor_ids() -> Vec<u16> {
    let mut ids: Vec<u16> = DEVICES.iter().map(|d| d.vendor_id).collect();
    // dedup() only removes *consecutive* duplicates, so this must be sorted
    // first — the table happens to be grouped today, but that is not a contract.
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Resolves the configured mode against what the device actually supports.
///
/// `auto` is accepted by every device: with more than one displayable mode it
/// cycles them, and with exactly one it *is* that mode. Requiring each driver to
/// list `auto` explicitly made the shipped `mode = "auto"` example a hard
/// startup failure on the LP-series and the CH510.
pub fn resolve_mode(requested: Mode, driver: &dyn DisplayDriver) -> Result<Mode> {
    let modes = driver.modes();
    match requested {
        Mode::Default => Ok(modes[0]),
        Mode::Auto => Ok(Mode::Auto),
        m if modes.contains(&m) => Ok(m),
        m => Err(Error::UnsupportedMode {
            mode: m.symbol(),
            supported: std::iter::once("auto")
                .chain(modes.iter().filter(|x| **x != Mode::Auto).map(|x| x.symbol()))
                .collect(),
        }),
    }
}

/// Validates the configured secondary mode for devices that have one.
pub fn resolve_secondary(requested: Mode, driver: &dyn DisplayDriver) -> Result<Mode> {
    let modes = driver.secondary_modes();
    if requested == Mode::Default || modes.is_empty() || modes.contains(&requested) {
        return Ok(requested);
    }
    Err(Error::UnsupportedMode {
        mode: requested.symbol(),
        supported: modes.iter().map(|m| m.symbol()).collect(),
    })
}

/// The one display loop.
///
/// Was copy-pasted verbatim into five driver modules, which is why a fix to the
/// temperature-unit byte landed in four of them.
pub fn run(driver: &mut dyn DisplayDriver, device: &mut dyn Device, mode: Mode, update: Duration) -> Result<()> {
    driver.warn(mode);
    driver.init(device)?;

    // Rate metrics (CPU usage, RAPL power) need a baseline plus a real interval.
    // Without this pause the first frame divides by a microsecond-long window
    // and reports 0% / an absurd wattage.
    let _ = crate::utils::shutdown::sleep_interruptible(PRIME_WINDOW.min(update));

    let cycle: Vec<Mode> = driver
        .auto_cycle()
        .iter()
        .copied()
        .filter(|m| *m != Mode::Auto)
        .collect();
    let auto = mode == Mode::Auto && cycle.len() > 1;
    // With a single displayable mode, `auto` simply is that mode.
    let single = if cycle.is_empty() { mode } else { cycle[0] };

    let mut index = 0usize;
    let mut consecutive_failures = 0u32;
    let mut switch_at = Instant::now() + AUTO_MODE_INTERVAL;
    let result = loop {
        // Checked at the top, so an error path that `continue`s cannot make the
        // process unstoppable.
        if crate::utils::shutdown::is_requested() {
            break Ok(());
        }

        if auto && Instant::now() >= switch_at {
            index = (index + 1) % cycle.len();
            switch_at = Instant::now() + AUTO_MODE_INTERVAL;
        }
        let active = if auto {
            cycle[index]
        } else if mode == Mode::Auto {
            single
        } else {
            mode
        };

        // One sensor snapshot per frame. On Windows this is what turns eleven
        // HTTP round-trips per frame back into one.
        #[cfg(target_os = "windows")]
        crate::utils::lhm::begin_frame();

        match driver.frame(device, active) {
            Ok(()) => consecutive_failures = 0,
            Err(e) => {
                crate::error!("{e}");
                device.recover();
                // A disconnected device is fatal immediately; anything else gets
                // a few retries, then gives up rather than spinning forever.
                if matches!(e, Error::Disconnected(_)) {
                    break Err(e);
                }
                consecutive_failures += 1;
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    break Err(e);
                }
            }
        }

        if !crate::utils::shutdown::sleep_interruptible(driver.pace().unwrap_or(update)) {
            break Ok(());
        }
    };

    driver.shutdown(device);
    crate::utils::shutdown::notify_cleanup_done();
    result
}

// ── Shared packet helpers ───────────────────────────────────────────────

/// Splits a value into three display digits, clamped to what a 3-digit panel
/// can actually show.
///
/// Values above 999 used to be encoded modulo their own digits — a 1200 W
/// reading rendered as "200" with a bogus leading digit of 12.
pub fn digits3(value: u16) -> [u8; 3] {
    let v = value.min(999);
    [(v / 100) as u8, (v % 100 / 10) as u8, (v % 10) as u8]
}

/// Two display digits, clamped to 99.
pub fn digits2(value: u16) -> [u8; 2] {
    let v = value.min(99);
    [(v / 10) as u8, (v % 10) as u8]
}

/// The 1..=10 status-bar level used by the segment protocols.
pub fn bar_level(usage: u8) -> u8 {
    if usage < 15 {
        1
    } else {
        (usage as f32 / 10.0).round().clamp(1.0, 10.0) as u8
    }
}

/// Sum-modulo-256 checksum over `data[1..=end]`, as every segment device uses.
pub fn checksum(data: &[u8; 64], end: usize) -> u8 {
    (data[1..=end].iter().map(|&x| x as u16).sum::<u16>() % 256) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_product_id_appears_once() {
        let mut seen = std::collections::HashSet::new();
        for d in DEVICES {
            assert!(
                seen.insert((d.vendor_id, d.product_id)),
                "duplicate table entry for VID {} PID {}",
                d.vendor_id,
                d.product_id
            );
        }
    }

    #[test]
    fn lookup_finds_table_entries_and_rejects_unknown_ones() {
        assert_eq!(lookup(DEFAULT_VENDOR_ID, pid::AK400).unwrap().name, "AK400 DIGITAL");
        assert_eq!(lookup(CH510_VENDOR_ID, pid::CH510).unwrap().name, "CH510 MESH DIGITAL");
        // CH510's PID under the *other* vendor id must not match.
        assert!(lookup(DEFAULT_VENDOR_ID, pid::CH510).is_none());
        assert!(lookup(DEFAULT_VENDOR_ID, 9999).is_none());
    }

    #[test]
    fn ag_is_marked_as_having_no_fahrenheit_unit_code() {
        // Regression guard: the AG protocol table lists only unit code 19 (C).
        let ag = lookup(DEFAULT_VENDOR_ID, pid::AG).unwrap();
        assert!(!ag.caps.fahrenheit);
        assert!(ag.caps.alarm);
    }

    #[test]
    fn only_the_lp_matrix_claims_rotation() {
        for d in DEVICES {
            assert_eq!(
                d.caps.rotation,
                d.product_id == pid::LP,
                "{} rotation capability is wrong",
                d.name
            );
        }
    }

    #[test]
    fn digits_clamp_instead_of_wrapping() {
        assert_eq!(digits3(0), [0, 0, 0]);
        assert_eq!(digits3(7), [0, 0, 7]);
        assert_eq!(digits3(42), [0, 4, 2]);
        assert_eq!(digits3(999), [9, 9, 9]);
        // A 1200 W reading must not render as "200".
        assert_eq!(digits3(1200), [9, 9, 9]);
        assert_eq!(digits3(u16::MAX), [9, 9, 9]);
    }

    #[test]
    fn two_digit_helper_clamps_at_99() {
        assert_eq!(digits2(0), [0, 0]);
        assert_eq!(digits2(99), [9, 9]);
        assert_eq!(digits2(100), [9, 9]);
        assert_eq!(digits2(255), [9, 9]);
    }

    #[test]
    fn bar_level_stays_within_the_protocol_range() {
        assert_eq!(bar_level(0), 1);
        assert_eq!(bar_level(14), 1);
        assert_eq!(bar_level(15), 2);
        assert_eq!(bar_level(50), 5);
        assert_eq!(bar_level(100), 10);
        assert_eq!(bar_level(255), 10);
    }

    #[test]
    fn checksum_matches_the_hand_rolled_sums_it_replaces() {
        let mut data = [0u8; 64];
        data[1] = 104;
        data[2] = 1;
        data[3] = 8;
        data[15] = 200;
        data[16] = 100;
        let expected = (data[1..=16].iter().map(|&x| x as u16).sum::<u16>() % 256) as u8;
        assert_eq!(checksum(&data, 16), expected);
    }

    struct Fake(&'static [Mode], &'static [Mode]);
    impl DisplayDriver for Fake {
        fn modes(&self) -> &'static [Mode] {
            self.0
        }
        fn secondary_modes(&self) -> &'static [Mode] {
            self.1
        }
        fn frame(&mut self, _d: &mut dyn Device, _m: Mode) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn auto_is_accepted_by_every_device() {
        // The LP-series and the CH510 do not list `auto`, and the shipped
        // example config used to set it — which made them exit at startup.
        let single = Fake(&[Mode::Auto], &[]);
        let many = Fake(&[Mode::CpuUsage, Mode::CpuTemperature], &[]);
        assert_eq!(resolve_mode(Mode::Auto, &single).unwrap(), Mode::Auto);
        assert_eq!(resolve_mode(Mode::Auto, &many).unwrap(), Mode::Auto);
    }

    #[test]
    fn default_resolves_to_the_first_listed_mode() {
        let d = Fake(&[Mode::CpuTemperature, Mode::CpuUsage], &[]);
        assert_eq!(resolve_mode(Mode::Default, &d).unwrap(), Mode::CpuTemperature);
    }

    #[test]
    fn an_unsupported_mode_is_rejected_and_lists_auto() {
        let d = Fake(&[Mode::CpuTemperature], &[]);
        let Err(Error::UnsupportedMode { mode, supported }) = resolve_mode(Mode::Psu, &d) else {
            panic!("psu must be rejected");
        };
        assert_eq!(mode, "psu");
        assert!(supported.contains(&"auto"), "{supported:?}");
        assert!(supported.contains(&"cpu_temp"), "{supported:?}");
    }

    #[test]
    fn an_unsupported_secondary_is_rejected() {
        let d = Fake(&[Mode::CpuTemperature], &[Mode::GpuTemperature, Mode::GpuUsage]);
        assert!(resolve_secondary(Mode::GpuUsage, &d).is_ok());
        assert!(resolve_secondary(Mode::Default, &d).is_ok());
        // Previously this fell through and silently blanked the second display.
        assert!(resolve_secondary(Mode::CpuPower, &d).is_err());

        // A device with no secondary display accepts anything (main warns).
        let none = Fake(&[Mode::CpuTemperature], &[]);
        assert!(resolve_secondary(Mode::CpuPower, &none).is_ok());
    }

    #[test]
    fn ch_gen2_still_accepts_the_not_yet_implemented_modes() {
        // Rejecting them turned previously working configs into a hard failure.
        let modes = ch_series_gen2::MODES_FOR_TEST;
        assert!(modes.contains(&Mode::CpuFan));
        assert!(modes.contains(&Mode::Psu));
        // ...but auto must not cycle into them.
        assert!(!ch_series_gen2::AUTO_CYCLE_FOR_TEST.contains(&Mode::CpuFan));
        assert!(!ch_series_gen2::AUTO_CYCLE_FOR_TEST.contains(&Mode::Psu));
    }

    #[test]
    fn vendor_ids_are_deduplicated_even_if_the_table_is_regrouped() {
        let ids = vendor_ids();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids, sorted, "vendor_ids must be sorted and unique");
        assert!(ids.contains(&DEFAULT_VENDOR_ID) && ids.contains(&CH510_VENDOR_ID));
    }

    #[test]
    fn mode_symbols_round_trip() {
        for m in [
            Mode::Auto,
            Mode::CpuTemperature,
            Mode::CpuUsage,
            Mode::CpuPower,
            Mode::CpuFrequency,
            Mode::CpuFan,
            Mode::GpuTemperature,
            Mode::GpuUsage,
            Mode::GpuPower,
            Mode::Cpu,
            Mode::Gpu,
            Mode::Psu,
        ] {
            assert_eq!(Mode::get(m.symbol()), Some(m), "{:?}", m);
        }
        assert_eq!(Mode::get("nonsense"), None);
        assert_eq!(Mode::get(""), None);
    }
}
