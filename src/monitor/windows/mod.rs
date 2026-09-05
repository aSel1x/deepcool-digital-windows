pub mod cpu;
pub mod fan;
pub mod gpu;
pub mod ram;

use crate::utils::lhm::{fetch, find, find_all};

/// Looks a sensor up in this frame's LHM snapshot.
///
/// The search itself lives in `utils::lhm` so it can be unit-tested against a
/// captured sensor tree on any host; the four monitor modules used to carry
/// byte-identical private copies of it.
pub(crate) fn read(keywords: &[&str], unit: char) -> Option<f64> {
    let root = fetch()?;
    find(&root, keywords, unit)
}

/// Collects every sensor whose name matches, in tree order.
pub(crate) fn read_all(keyword: &str, unit: char) -> Vec<f64> {
    match fetch() {
        Some(root) => find_all(&root, keyword, unit),
        None => Vec::new(),
    }
}

/// Rounds and clamps a sensor reading into `u16`.
pub(crate) fn to_u16(v: f64) -> u16 {
    v.round().clamp(0.0, u16::MAX as f64) as u16
}

/// Rounds and clamps a percentage into `u8`.
pub(crate) fn to_percent(v: f64) -> u8 {
    v.round().clamp(0.0, 100.0) as u8
}
