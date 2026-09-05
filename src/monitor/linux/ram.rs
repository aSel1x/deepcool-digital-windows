pub struct Ram;

impl Ram {
    pub fn new() -> Self {
        Ram
    }

    pub fn warn_missing(&self) {
        if self.get_usage().is_none() {
            crate::warning!("RAM usage is unavailable (/proc/meminfo could not be read)");
        }
    }

    pub fn get_usage(&self) -> Option<u8> {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        parse_meminfo_usage(&meminfo)
    }
}

impl Default for Ram {
    fn default() -> Self {
        Self::new()
    }
}

/// Percentage of physical RAM in use, from the contents of `/proc/meminfo`.
///
/// Split out from the filesystem read so it can be tested against fixtures.
fn parse_meminfo_usage(meminfo: &str) -> Option<u8> {
    let mut total_kb = None;
    let mut available_kb = None;
    let mut free_kb = None;

    for line in meminfo.lines() {
        if let Some(value) = line.strip_prefix("MemTotal:") {
            total_kb = parse_kib_value(value);
        } else if let Some(value) = line.strip_prefix("MemAvailable:") {
            available_kb = parse_kib_value(value);
        } else if let Some(value) = line.strip_prefix("MemFree:") {
            free_kb = parse_kib_value(value);
        }
    }

    let total = total_kb?;
    if total == 0 {
        return None;
    }
    // MemAvailable accounts for reclaimable page cache; MemFree does not, and
    // over-reports usage badly on a warm system. Prefer it when present.
    let used = total.saturating_sub(available_kb.or(free_kb)?);

    Some(((used as f64 * 100.0) / total as f64).round().clamp(0.0, 100.0) as u8)
}

fn parse_kib_value(raw: &str) -> Option<u64> {
    raw.split_whitespace().next()?.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::parse_meminfo_usage;

    const SAMPLE: &str = "\
MemTotal:       32695260 kB
MemFree:         1234567 kB
MemAvailable:   16347630 kB
Buffers:          123456 kB
Cached:          8888888 kB
";

    #[test]
    fn uses_mem_available_when_present() {
        // Exactly half available -> 50%.
        assert_eq!(parse_meminfo_usage(SAMPLE), Some(50));
    }

    #[test]
    fn falls_back_to_mem_free_without_mem_available() {
        let no_available = "MemTotal: 1000 kB\nMemFree: 250 kB\n";
        assert_eq!(parse_meminfo_usage(no_available), Some(75));
    }

    #[test]
    fn missing_or_degenerate_input_reports_absent() {
        assert_eq!(parse_meminfo_usage(""), None);
        assert_eq!(parse_meminfo_usage("MemTotal: 0 kB\nMemFree: 0 kB\n"), None);
        // Total present but neither MemFree nor MemAvailable.
        assert_eq!(parse_meminfo_usage("MemTotal: 1000 kB\n"), None);
    }

    #[test]
    fn does_not_underflow_when_available_exceeds_total() {
        let odd = "MemTotal: 1000 kB\nMemAvailable: 4000 kB\n";
        assert_eq!(parse_meminfo_usage(odd), Some(0));
    }
}
