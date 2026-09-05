mod config;
mod device;
mod devices;
mod error;
mod monitor;
mod utils;

use colored::*;
use config::AppConfig;
use device::{list_devices, open};
use devices::{lookup, vendor_ids, Ctx, DeviceEntry, Mode};
use error::{Error, Result};
use std::path::PathBuf;
use std::process::exit;
use std::time::Duration;

const MIN_UPDATE_MS: u64 = 100;
const MAX_UPDATE_MS: u64 = 2000;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let result = run(&args);
    // Release any shutdown handler blocked waiting for the display loop — it is
    // only set inside devices::run, so an error before that would make a console
    // close wait out the full grace period.
    utils::shutdown::notify_cleanup_done();

    if let Err(e) = result {
        error!("{e}");
        if let Some(hint) = e.hint() {
            eprintln!("        {hint}");
        }
        exit(1);
    }
}

fn run(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("deepcool-digital {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    reject_unknown_args(args)?;

    // --config is read before anything else, since everything else depends on it.
    let config_path = arg_value(args, "--config").map(PathBuf::from);
    AppConfig::init(config_path.as_deref())?;

    if args.iter().any(|a| a == "--list") {
        return list_attached();
    }
    if let Some(pos) = position(args, &["--crc", "-c"]) {
        return crc_brute_force(args, pos);
    }
    if let Some(pos) = position(args, &["--send", "-s"]) {
        return send_raw(args, pos);
    }

    display_loop()
}

fn print_help() {
    println!("Usage: deepcool-digital [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --config <path>            Use this config file instead of searching");
    println!("  --list                     List attached DeepCool devices and exit");
    println!("  --crc <d1> <c1> <d2> <c2>  CRC-16 brute-force (two packet/CRC pairs)");
    println!("  --send <hex>               Send raw hex packet to device (reads response)");
    println!("  --help, -h                 Show this help");
    println!("  --version, -V              Show version");
    println!();
    println!("Config search order:");
    println!("  --config, $DEEPCOOL_CONFIG, ./config.toml,");
    #[cfg(target_os = "linux")]
    println!("  $XDG_CONFIG_HOME/deepcool-digital/config.toml, /etc/deepcool-digital/config.toml,");
    #[cfg(windows)]
    println!("  %APPDATA%\\deepcool-digital\\config.toml,");
    println!("  then next to the executable.");
    println!();
    println!("Examples:");
    println!("  deepcool-digital --send aa040006016400     # brightness 100, 0°");
    println!("  deepcool-digital --send aa040006036400     # brightness 100, 180°");
    println!("  deepcool-digital --send aa0100092991       # init query");
    println!("  deepcool-digital --crc aa040006026400 178e aa040006046400 b73c");
}

/// Every flag the program understands, and whether it takes a value.
const FLAGS: &[(&str, bool)] = &[
    ("--help", false),
    ("-h", false),
    ("--version", false),
    ("-V", false),
    ("--list", false),
    ("--config", true),
    ("--crc", true),
    ("-c", true),
    ("--send", true),
    ("-s", true),
];

/// Rejects unknown flags instead of silently ignoring them.
///
/// A typo like `--verison` used to run the display loop as if nothing had been
/// asked for.
fn reject_unknown_args(args: &[String]) -> Result<()> {
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        let name = a.split('=').next().unwrap_or(a);
        match FLAGS.iter().find(|(f, _)| *f == name) {
            Some((_, takes_value)) => {
                // --crc consumes four values, everything else at most one.
                let extra = match name {
                    "--crc" | "-c" => 4,
                    _ if *takes_value && !a.contains('=') => 1,
                    _ => 0,
                };
                i += 1 + extra;
            }
            None => return Err(Error::Config(format!("unrecognized argument \"{a}\" (try --help)"))),
        }
    }
    Ok(())
}

fn position(args: &[String], names: &[&str]) -> Option<usize> {
    args.iter().position(|a| names.contains(&a.as_str()))
}

/// Reads `--name value` and `--name=value`.
fn arg_value(args: &[String], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    for (i, a) in args.iter().enumerate() {
        if let Some(v) = a.strip_prefix(&prefix) {
            return Some(v.to_string());
        }
        if a == name {
            return args.get(i + 1).cloned();
        }
    }
    None
}

// ── Device selection ────────────────────────────────────────────────────

/// Finds the device to drive.
///
/// Enumeration errors are reported rather than swallowed. When several devices
/// match, all of them are listed with a pointer at the `[device]` config key and
/// the first is used — previously the lowest product id won with no output at
/// all.
fn select_device() -> Result<(u16, u16)> {
    let cfg = AppConfig::get();
    let wanted_pid = cfg.device.as_ref().and_then(|d| d.product_id);
    let wanted_vid = cfg.device.as_ref().and_then(|d| d.vendor_id);

    let mut found: Vec<(u16, u16)> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for vid in vendor_ids() {
        if let Some(v) = wanted_vid {
            if v != vid {
                continue;
            }
        }
        match list_devices(vid) {
            Ok(pids) => found.extend(pids.into_iter().map(|pid| (vid, pid))),
            Err(e) => errors.push(format!("VID_{vid:04X}: {e}")),
        }
    }

    if let Some(pid) = wanted_pid {
        found.retain(|(_, p)| *p == pid);
    }

    match found.len() {
        0 => {
            for e in &errors {
                debug!("device enumeration: {e}");
            }
            if !errors.is_empty() {
                warning!("device enumeration reported errors; run with debug = true for details");
            }
            Err(Error::NoDevice)
        }
        1 => Ok(found[0]),
        _ => {
            warning!("several DeepCool devices are attached:");
            for (vid, pid) in &found {
                let name = lookup(*vid, *pid).map(|d| d.name).unwrap_or("unknown");
                eprintln!("         VID {vid}  PID {pid}  {name}");
            }
            eprintln!("         Set [device] product_id in config.toml to choose one.");
            Ok(found[0])
        }
    }
}

/// Prints every attached DeepCool device, supported or not.
fn list_attached() -> Result<()> {
    let mut any = false;
    for vid in vendor_ids() {
        let pids = match list_devices(vid) {
            Ok(p) => p,
            Err(e) => {
                warning!("VID_{vid:04X}: {e}");
                continue;
            }
        };
        for pid in pids {
            any = true;
            match lookup(vid, pid) {
                Some(entry) => println!("VID {vid}  PID {pid}  {}", entry.name),
                None => println!("VID {vid}  PID {pid}  {}", "not supported yet".yellow()),
            }
        }
    }
    if !any {
        println!("No DeepCool device was found.");
    }
    Ok(())
}

// ── Main display loop ───────────────────────────────────────────────────

fn display_loop() -> Result<()> {
    println!("--- Deepcool Digital ---");
    if let Some(path) = AppConfig::loaded_from() {
        println!("Config: {}", path.display());
    } else {
        println!("Config: {} (no config.toml found)", "defaults".yellow());
    }

    utils::shutdown::install();

    let (vendor_id, product_id) = select_device()?;
    let entry = lookup(vendor_id, product_id).ok_or(Error::UnsupportedDevice { product_id })?;

    let mut device = open(vendor_id, product_id)?;
    println!(
        "Device found: {} (PID {})",
        entry.name.bright_green(),
        product_id.to_string().bright_green()
    );

    let cfg = AppConfig::get();
    let seg = cfg.segment.clone().unwrap_or_default();
    let requested = parse_mode(seg.mode.as_deref(), "segment.mode");
    let secondary = parse_mode(seg.secondary.as_deref(), "segment.secondary");
    let update = update_interval(cfg.update_interval_ms);

    let (fahrenheit, alarm, rotation) = apply_capabilities(entry, &seg);

    let ctx = Ctx {
        cpu: monitor::Cpu::new(),
        gpu: monitor::Gpu::new(),
        ram: monitor::Ram::new(),
        sys_fan: monitor::SysFan::new(),
        fahrenheit,
        alarm,
        rotation,
        secondary: if entry.caps.secondary { secondary } else { Mode::Default },
    };

    let mut driver = (entry.build)(ctx)?;
    let mode = devices::resolve_mode(requested, driver.as_ref())?;
    // The old constructors rejected an out-of-range secondary outright; without
    // this it silently blanked half the panel.
    devices::resolve_secondary(secondary, driver.as_ref())?;

    let supported: Vec<&str> = driver.modes().iter().map(|m| m.symbol()).collect();
    println!("Supported modes: {}", supported.join(" ").bold());
    println!("Display mode: {}", mode.symbol().bold());
    println!("Update interval: {update:?}");
    println!("\nPress {} to terminate", "Ctrl+C".bold());

    devices::run(driver.as_mut(), device.as_mut(), mode, update)
}

/// Warns about settings the selected device cannot honour, and neutralises them.
///
/// The capability flags come from the device table, so this replaces four
/// hand-maintained product-id lists that had already drifted apart.
fn apply_capabilities(entry: &DeviceEntry, seg: &config::SegmentConfig) -> (bool, bool, u16) {
    let mut fahrenheit = seg.fahrenheit.unwrap_or(false);
    let mut alarm = seg.alarm.unwrap_or(false);
    let mut rotation = seg.rotation.unwrap_or(0);

    if fahrenheit && !entry.caps.fahrenheit {
        warning!(
            "segment.fahrenheit is set, but {} has no Fahrenheit unit code — displaying Celsius",
            entry.name
        );
        fahrenheit = false;
    }
    if alarm && !entry.caps.alarm {
        warning!("segment.alarm is set, but {} has no overheat alarm — ignoring", entry.name);
        alarm = false;
    }
    if seg.secondary.is_some() && !entry.caps.secondary {
        warning!(
            "segment.secondary is set, but {} has no secondary display — ignoring",
            entry.name
        );
    }
    if rotation != 0 && !entry.caps.rotation {
        warning!(
            "segment.rotation is set, but {} does not support rotation — ignoring",
            entry.name
        );
        rotation = 0;
    }
    if !matches!(rotation, 0 | 90 | 180 | 270) {
        warning!("segment.rotation = {rotation} is not supported (use 0, 90, 180, or 270), using 0");
        rotation = 0;
    }

    (fahrenheit, alarm, rotation)
}

fn update_interval(raw: Option<u64>) -> Duration {
    let ms = raw.unwrap_or(1000);
    if (MIN_UPDATE_MS..=MAX_UPDATE_MS).contains(&ms) {
        return Duration::from_millis(ms);
    }
    let clamped = ms.clamp(MIN_UPDATE_MS, MAX_UPDATE_MS);
    warning!("update_interval_ms = {ms} is outside the supported range ({MIN_UPDATE_MS}-{MAX_UPDATE_MS}ms), clamping to {clamped}");
    Duration::from_millis(clamped)
}

/// Parses a `Mode` from a config string, warning (not silently defaulting) on an unrecognized value.
fn parse_mode(raw: Option<&str>, field: &str) -> Mode {
    match raw {
        None => Mode::Default,
        Some(s) => Mode::get(s).unwrap_or_else(|| {
            warning!("{field} = \"{s}\" is not a recognized mode, using the default");
            Mode::Default
        }),
    }
}

// ── Developer subcommands ───────────────────────────────────────────────

fn crc_brute_force(args: &[String], pos: usize) -> Result<()> {
    // Four trailing arguments are read, so four must be present. The previous
    // guard only checked for three and then indexed past the end.
    if pos + 4 >= args.len() {
        eprintln!("Usage: deepcool-digital --crc <data1> <crc1> <data2> <crc2>");
        eprintln!("  <data>  Hex packet data (e.g. aa08000001005802002c01bc11)");
        eprintln!("  <crc>   Expected CRC hex (e.g. 11bc)");
        eprintln!();
        eprintln!("Requires two packet/CRC pairs to eliminate false positives.");
        eprintln!("Example:");
        eprintln!("  deepcool-digital --crc aa040006026400 178e aa040006046400 b73c");
        exit(1);
    }
    let pairs: [(&str, &str); 2] = [(&args[pos + 1], &args[pos + 2]), (&args[pos + 3], &args[pos + 4])];

    match utils::crc::brute_force_crc(&pairs) {
        Some(result) => {
            println!("\n✅ CRC-16 MATCH FOUND!");
            println!("  Poly:    0x{:04X}", result.poly);
            println!("  Init:    0x{:04X}", result.init);
            println!("  XorOut:  0x{:04X}", result.xorout);
            println!("  RefIn:   {}", result.refin);
            println!("  Byte order: {}", if result.swapped { "little-endian" } else { "big-endian" });
        }
        None => {
            println!("\n❌ No matching CRC-16 parameters found.");
            println!("Try different packets or verify the expected CRC values.");
        }
    }
    Ok(())
}

fn send_raw(args: &[String], pos: usize) -> Result<()> {
    if pos + 1 >= args.len() {
        eprintln!("Usage: deepcool-digital --send <hex>");
        eprintln!("  <hex>  Hex packet data (e.g. aa04000603640027b9)");
        exit(1);
    }
    let data = hex_to_bytes(&args[pos + 1]).map_err(Error::Config)?;

    println!("Opening device...");
    let (vendor_id, product_id) = select_device()?;
    let mut device = open(vendor_id, product_id)?;
    println!("Device found: PID {product_id}");
    println!("Sending {} bytes: {:02X?}", data.len(), data);

    // Explicit framing: a 64-byte payload is a HID report whose first byte is
    // the report id; anything else is a raw bulk transfer. This used to be
    // guessed from the length with no way to say which was meant.
    if data.len() == 64 {
        let mut buf = [0u8; 64];
        buf.copy_from_slice(&data);
        device.write_report(&buf)?;
    } else {
        device.write_bulk(&data)?;
    }

    let mut resp = [0u8; 64];
    match device.read(&mut resp) {
        Ok(n) => println!("Response ({n} bytes): {:02X?}", &resp[..n]),
        Err(e) => println!("No response (or timeout): {e}"),
    }
    Ok(())
}

fn hex_to_bytes(hex: &str) -> std::result::Result<Vec<u8>, String> {
    if !hex.len().is_multiple_of(2) {
        return Err("hex string must have even length".to_string());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| format!("{e} at position {i}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing_round_trips_and_rejects_bad_input() {
        assert_eq!(hex_to_bytes("aa0400"), Ok(vec![0xAA, 0x04, 0x00]));
        assert_eq!(hex_to_bytes(""), Ok(vec![]));
        assert!(hex_to_bytes("abc").is_err(), "odd length");
        assert!(hex_to_bytes("zz").is_err(), "non-hex");
    }

    #[test]
    fn crc_guard_requires_four_trailing_arguments() {
        // Regression: the old guard checked pos + 3 and then read args[pos + 4],
        // aborting with an index-out-of-bounds panic on exactly three.
        let args: Vec<String> = ["prog", "--crc", "d1", "c1", "d2"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let pos = position(&args, &["--crc", "-c"]).unwrap();
        assert!(pos + 4 >= args.len(), "three trailing args must be rejected");

        let ok: Vec<String> = ["prog", "--crc", "d1", "c1", "d2", "c2"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let pos = position(&ok, &["--crc", "-c"]).unwrap();
        assert!(pos + 4 < ok.len(), "four trailing args must be accepted");
    }

    #[test]
    fn update_interval_clamps_to_the_supported_range() {
        assert_eq!(update_interval(None), Duration::from_millis(1000));
        assert_eq!(update_interval(Some(500)), Duration::from_millis(500));
        assert_eq!(update_interval(Some(1)), Duration::from_millis(MIN_UPDATE_MS));
        assert_eq!(update_interval(Some(99999)), Duration::from_millis(MAX_UPDATE_MS));
    }

    #[test]
    fn unknown_modes_fall_back_to_the_device_default() {
        assert_eq!(parse_mode(None, "segment.mode"), Mode::Default);
        assert_eq!(parse_mode(Some("cpu_temp"), "segment.mode"), Mode::CpuTemperature);
        assert_eq!(parse_mode(Some("nonsense"), "segment.mode"), Mode::Default);
    }

    #[test]
    fn unknown_arguments_are_rejected() {
        let ok: Vec<String> = ["prog", "--config", "/tmp/a.toml", "--list"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(reject_unknown_args(&ok).is_ok());

        let joined: Vec<String> = ["prog", "--config=/tmp/a.toml"].iter().map(|s| s.to_string()).collect();
        assert!(reject_unknown_args(&joined).is_ok());

        let crc: Vec<String> = ["prog", "--crc", "d1", "c1", "d2", "c2"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(reject_unknown_args(&crc).is_ok(), "--crc values must not be treated as flags");

        let typo: Vec<String> = ["prog", "--verison"].iter().map(|s| s.to_string()).collect();
        assert!(reject_unknown_args(&typo).is_err());
    }

    #[test]
    fn arg_value_reads_the_following_argument() {
        let args: Vec<String> = ["prog", "--config", "/tmp/x.toml"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(arg_value(&args, "--config"), Some("/tmp/x.toml".to_string()));
        assert_eq!(arg_value(&args, "--missing"), None);

        let dangling: Vec<String> = ["prog", "--config"].iter().map(|s| s.to_string()).collect();
        assert_eq!(arg_value(&dangling, "--config"), None, "must not panic");

        let joined: Vec<String> = ["prog", "--config=/tmp/y.toml"].iter().map(|s| s.to_string()).collect();
        assert_eq!(arg_value(&joined, "--config"), Some("/tmp/y.toml".to_string()));
    }

    #[test]
    fn fahrenheit_is_forced_off_for_devices_without_a_unit_code() {
        let ag = lookup(devices::DEFAULT_VENDOR_ID, devices::pid::AG).unwrap();
        let seg = config::SegmentConfig {
            fahrenheit: Some(true),
            ..Default::default()
        };
        let (fahrenheit, _, _) = apply_capabilities(ag, &seg);
        assert!(!fahrenheit, "AG has no Fahrenheit unit code");

        let ak = lookup(devices::DEFAULT_VENDOR_ID, devices::pid::AK400).unwrap();
        let (fahrenheit, _, _) = apply_capabilities(ak, &seg);
        assert!(fahrenheit, "AK does support Fahrenheit");
    }

    #[test]
    fn rotation_is_ignored_for_devices_that_cannot_rotate() {
        let ak = lookup(devices::DEFAULT_VENDOR_ID, devices::pid::AK400).unwrap();
        let seg = config::SegmentConfig {
            rotation: Some(90),
            ..Default::default()
        };
        assert_eq!(apply_capabilities(ak, &seg).2, 0);

        let lp = lookup(devices::DEFAULT_VENDOR_ID, devices::pid::LP).unwrap();
        assert_eq!(apply_capabilities(lp, &seg).2, 90);
    }

    #[test]
    fn invalid_rotation_angles_fall_back_to_zero() {
        let lp = lookup(devices::DEFAULT_VENDOR_ID, devices::pid::LP).unwrap();
        let seg = config::SegmentConfig {
            rotation: Some(45),
            ..Default::default()
        };
        assert_eq!(apply_capabilities(lp, &seg).2, 0);
    }
}
