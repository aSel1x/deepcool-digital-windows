//! LibreHardwareMonitor HTTP API client.
//!
//! Windows-only: LHM предоставляет сенсоры CPU/GPU/RAM, недоступные через
//! стандартные API.
//!
//! # One request per frame
//!
//! Every getter used to call [`fetch`] itself, so a single LM frame cost eleven
//! HTTP round-trips and eleven full deserialisations of the ~76 KB sensor tree —
//! about 7.5 ms of pure JSON parsing per frame before LHM's own response time.
//! Now the tree is fetched at most once per frame: [`begin_frame`] marks the
//! cache stale, the first getter refreshes it, and the rest of the frame reads
//! the same snapshot.

#[cfg(windows)]
use crate::error::Error;
#[cfg(windows)]
use base64::prelude::{Engine, BASE64_STANDARD};
use serde::Deserialize;
#[cfg(windows)]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(windows)]
use std::time::{Duration, Instant};

#[cfg(windows)]
const DEFAULT_URL: &str = "http://127.0.0.1:8085/data.json";
#[cfg(windows)]
const DEFAULT_TIMEOUT_MS: u64 = 2_000;
/// A snapshot older than this is reported as absent rather than served as if it
/// were live. A stale number that looks current is worse than a missing one.
#[cfg(windows)]
const MAX_STALE: Duration = Duration::from_secs(5);

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct LhmWebNode {
    pub text: Option<String>,
    pub value: Option<String>,
    pub children: Option<Vec<LhmWebNode>>,
}

#[cfg(windows)]
struct Cache {
    tree: Option<Arc<LhmWebNode>>,
    fetched_at: Option<Instant>,
    /// Set by `begin_frame`; the next `fetch` performs a real request.
    stale: bool,
    /// Whether the last request failed, so the warning is only printed once.
    warned: bool,
}

#[cfg(windows)]
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(Cache {
            tree: None,
            fetched_at: None,
            stale: true,
            warned: false,
        })
    })
}

#[cfg(windows)]
fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let timeout = crate::config::AppConfig::get()
            .lhm
            .as_ref()
            .and_then(|c| c.timeout_ms)
            .unwrap_or(DEFAULT_TIMEOUT_MS);
        // Without an explicit timeout a hung LHM stalls the frame loop forever.
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(timeout)))
            .build()
            .new_agent()
    })
}

/// Marks the cached sensor tree stale. Call once at the top of every frame.
#[cfg(windows)]
pub fn begin_frame() {
    if let Ok(mut c) = cache().lock() {
        c.stale = true;
    }
}

/// Получить данные от LHM. URL и auth берёт из config.toml.
///
/// Returns the snapshot for the current frame, fetching it at most once.
#[cfg(windows)]
pub fn fetch() -> Option<Arc<LhmWebNode>> {
    let mut c = cache().lock().ok()?;

    if !c.stale {
        return c.tree.clone();
    }

    match request() {
        Ok(tree) => {
            c.tree = Some(Arc::new(tree));
            c.fetched_at = Some(Instant::now());
            c.stale = false;
            c.warned = false;
        }
        Err(e) => {
            c.stale = false;
            if !c.warned {
                c.warned = true;
                crate::warning!("LibreHardwareMonitor request failed: {e}");
                eprintln!("         Enable LibreHardwareMonitor's 'Run Remote Web Server'.");
            }
            // Serve the previous snapshot only while it is plausibly still true.
            let too_old = c.fetched_at.map(|t| t.elapsed() > MAX_STALE).unwrap_or(true);
            if too_old {
                c.tree = None;
            }
        }
    }

    c.tree.clone()
}

#[cfg(windows)]
fn request() -> Result<LhmWebNode, Error> {
    let cfg = crate::config::AppConfig::get();
    let lhm = cfg.lhm.as_ref();

    let url = lhm
        .and_then(|c| c.url.as_deref())
        .filter(|u| u.starts_with("http"))
        .unwrap_or(DEFAULT_URL);

    let auth = lhm
        .and_then(|c| c.auth_bearer.as_deref())
        .filter(|t| !t.trim().is_empty())
        .map(|t| format!("Bearer {t}"))
        .or_else(|| {
            let c = lhm?;
            let user = c.auth_basic_user.as_deref().filter(|u| !u.trim().is_empty())?;
            let pass = c.auth_basic_pass.as_deref().filter(|p| !p.trim().is_empty())?;
            let encoded = BASE64_STANDARD.encode(format!("{user}:{pass}"));
            Some(format!("Basic {encoded}"))
        });

    let mut req = agent().get(url);
    if let Some(ref token) = auth {
        req = req.header("Authorization", token);
    }

    let resp = req.call().map_err(|e| Error::Transfer(e.to_string()))?;
    resp.into_body()
        .read_json()
        .map_err(|e| Error::Transfer(format!("malformed LHM response: {e}")))
}

/// Распарсить числовое значение из строки LHM (запятая → точка).
pub fn parse_value(raw: &str) -> Option<f64> {
    // LHM formats with the host locale, so the decimal separator may be ',' and
    // large values may carry a thousands separator. Keep the last separator as
    // the decimal point and drop the rest.
    let mut digits = String::with_capacity(raw.len());
    let mut negative = false;
    for (i, c) in raw.chars().enumerate() {
        match c {
            '-' if i == 0 => negative = true,
            '0'..='9' => digits.push(c),
            '.' | ',' => digits.push('.'),
            _ => {}
        }
    }
    if digits.is_empty() {
        return None;
    }
    // "1.234.5" -> "1234.5": everything but the final separator is grouping.
    let normalized = match digits.rfind('.') {
        Some(pos) => {
            let (head, tail) = digits.split_at(pos);
            format!("{}{}", head.replace('.', ""), tail)
        }
        None => digits,
    };
    let value: f64 = normalized.parse().ok()?;
    let value = if negative { -value } else { value };
    value.is_finite().then_some(value)
}

// ── Sensor tree search ──────────────────────────────────────────────────

/// Finds a sensor by name and unit.
///
/// Two passes: exact name match across every keyword, then substring. Keyword
/// order matters within a pass, which is why "bus speed" must not appear in the
/// CPU frequency list — an exact "Bus Speed" hit would beat a "Cores (Average)"
/// substring hit and report the ~100 MHz reference clock as the core frequency.
pub fn find(root: &LhmWebNode, keywords: &[&str], unit: char) -> Option<f64> {
    for &kw in keywords {
        if let Some(v) = walk(root, kw, unit, true) {
            return Some(v);
        }
    }
    for &kw in keywords {
        if let Some(v) = walk(root, kw, unit, false) {
            return Some(v);
        }
    }
    None
}

fn walk(node: &LhmWebNode, keyword: &str, unit: char, exact: bool) -> Option<f64> {
    let text = node.text.as_deref().unwrap_or("").to_ascii_lowercase();
    let raw_value = node.value.as_deref().unwrap_or("");
    let value = raw_value.to_ascii_lowercase();

    let name_matches = if exact { text == keyword } else { text.contains(keyword) };
    if name_matches && value.contains(unit) {
        if let Some(v) = parse_value(raw_value) {
            return Some(v);
        }
    }

    for child in node.children.as_deref().unwrap_or(&[]) {
        if let Some(v) = walk(child, keyword, unit, exact) {
            return Some(v);
        }
    }
    None
}

/// Collects every sensor whose name contains `keyword`, in tree order.
pub fn find_all(root: &LhmWebNode, keyword: &str, unit: char) -> Vec<f64> {
    let mut out = Vec::new();
    collect(root, keyword, unit, &mut out);
    out
}

fn collect(node: &LhmWebNode, keyword: &str, unit: char, out: &mut Vec<f64>) {
    let text = node.text.as_deref().unwrap_or("").to_ascii_lowercase();
    let raw_value = node.value.as_deref().unwrap_or("");
    if text.contains(keyword) && raw_value.to_ascii_lowercase().contains(unit) {
        if let Some(v) = parse_value(raw_value) {
            out.push(v);
        }
    }
    for child in node.children.as_deref().unwrap_or(&[]) {
        collect(child, keyword, unit, out);
    }
}

/// Percentage of physical RAM in use, from the "Total Memory" section.
pub fn find_memory_load(root: &LhmWebNode) -> Option<f64> {
    fn section(node: &LhmWebNode, name: &str) -> Option<f64> {
        let text = node.text.as_deref().unwrap_or("").to_ascii_lowercase();
        if text == name {
            for child in node.children.as_deref().unwrap_or(&[]) {
                if !child.text.as_deref().unwrap_or("").eq_ignore_ascii_case("load") {
                    continue;
                }
                for g in child.children.as_deref().unwrap_or(&[]) {
                    let gtext = g.text.as_deref().unwrap_or("").to_ascii_lowercase();
                    let gval = g.value.as_deref().unwrap_or("");
                    if gtext == "memory" && gval.contains('%') {
                        return parse_value(gval);
                    }
                }
            }
        }
        for child in node.children.as_deref().unwrap_or(&[]) {
            if let Some(v) = section(child, name) {
                return Some(v);
            }
        }
        None
    }
    section(root, "total memory").or_else(|| section(root, "virtual memory"))
}

/// Nth system fan (i.e. not the CPU fan, the pump, or an AIO fan).
pub fn find_sys_fan(root: &LhmWebNode, index: usize) -> Option<f64> {
    fn walk(node: &LhmWebNode, target: usize, seen: &mut usize) -> Option<f64> {
        let text = node.text.as_deref().unwrap_or("").to_ascii_lowercase();
        let value = node.value.as_deref().unwrap_or("");
        let is_sys_fan =
            text.contains("fan") && !text.contains("cpu") && !text.contains("pump") && !text.contains("aio");
        if is_sys_fan && value.to_ascii_lowercase().contains('r') {
            if let Some(rpm) = parse_value(value) {
                if *seen == target {
                    return Some(rpm);
                }
                *seen += 1;
            }
        }
        for child in node.children.as_deref().unwrap_or(&[]) {
            if let Some(v) = walk(child, target, seen) {
                return Some(v);
            }
        }
        None
    }
    let mut seen = 0usize;
    walk(root, index, &mut seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_locale_forms_lhm_emits() {
        assert_eq!(parse_value("45.0 °C"), Some(45.0));
        assert_eq!(parse_value("45,0 °C"), Some(45.0));
        assert_eq!(parse_value("1234 RPM"), Some(1234.0));
        assert_eq!(parse_value("1.25 V"), Some(1.25));
        assert_eq!(parse_value("58,5 %"), Some(58.5));
    }

    #[test]
    fn handles_thousands_separators() {
        // The old fixed-buffer parser turned this into "1.234.5" and gave up.
        assert_eq!(parse_value("1,234.5 MHz"), Some(1234.5));
        assert_eq!(parse_value("4.314,0 MHz"), Some(4314.0));
    }

    #[test]
    fn handles_negatives_and_rejects_junk() {
        assert_eq!(parse_value("-12,5 °C"), Some(-12.5));
        assert_eq!(parse_value(""), None);
        assert_eq!(parse_value("n/a"), None);
        assert_eq!(parse_value("-"), None);
    }

    fn fixture() -> LhmWebNode {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/lhm-sample.json");
        let text = std::fs::read_to_string(path).expect("LHM fixture missing");
        serde_json::from_str(&text).expect("LHM fixture is not a sensor tree")
    }

    #[test]
    fn finds_cpu_temperature_in_a_real_sensor_tree() {
        let root = fixture();
        let temp = find(&root, &["core (tctl/tdie)", "core (tdie)", "package", "cpu core"], 'c')
            .expect("no CPU temperature in the fixture");
        assert!((0.0..=150.0).contains(&temp), "implausible CPU temperature {temp}");
    }

    #[test]
    fn cpu_frequency_does_not_fall_back_to_the_bus_clock() {
        let root = fixture();
        // The fixture has both "Bus Speed" (100 MHz) and "Cores (Average)".
        let bus = find(&root, &["bus speed"], 'h').expect("fixture should expose Bus Speed");
        assert!(bus < 200.0, "sanity: bus speed is the ~100 MHz reference clock");

        // The keyword list the driver actually uses must not pick it up.
        let freq = find(&root, &["cores (average)"], 'h').expect("no core clock in the fixture");
        assert!(
            freq > 1000.0,
            "core clock {freq} looks like the bus clock — the Bus Speed regression is back"
        );
    }

    #[test]
    fn finds_memory_load_and_it_is_a_percentage() {
        let load = find_memory_load(&fixture()).expect("no memory load in the fixture");
        assert!((0.0..=100.0).contains(&load), "implausible memory load {load}");
    }

    #[test]
    fn system_fans_are_enumerated_and_exclude_cpu_and_pump() {
        let root = fixture();
        // Index 0 must exist on a board with case fans; walking past the end
        // must report absent rather than panicking.
        let first = find_sys_fan(&root, 0);
        if let Some(rpm) = first {
            assert!((0.0..=10_000.0).contains(&rpm), "implausible fan speed {rpm}");
        }
        assert!(find_sys_fan(&root, 9_999).is_none());
    }

    #[test]
    fn a_missing_sensor_reports_absent_rather_than_zero() {
        let root = fixture();
        assert!(find(&root, &["definitely not a sensor"], 'c').is_none());
        assert!(find_all(&root, "definitely not a sensor", 'c').is_empty());
    }

    #[test]
    fn is_not_limited_to_32_characters() {
        // The previous implementation wrote into a fixed [u8; 32] and silently
        // truncated anything longer.
        let padded = format!("{}42.5 W", " ".repeat(64));
        assert_eq!(parse_value(&padded), Some(42.5));
    }
}
