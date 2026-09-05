pub mod crc;
pub mod lcd_renderer;
// The HTTP client half is Windows-only; the sensor-tree search is not, so the
// module compiles everywhere and its parsing logic is testable on any host.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod lhm;
pub mod shutdown;
pub mod svg_template;

use colored::Colorize;

#[doc(hidden)]
pub fn log_line(level: &str, message: String) {
    let prefix = match level {
        "DEBUG" => "[DEBUG]".bright_black().bold(),
        "INFO" => "[INFO ]".bright_cyan().bold(),
        "WARN" => "[WARN ]".yellow().bold(),
        "ERROR" => "[ERROR]".red().bold(),
        _ => "[LOG  ]".bold(),
    };
    eprintln!("{} {}", prefix, message);
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {{
        if $crate::config::AppConfig::get().debug.unwrap_or(false) {
            $crate::utils::log_line("DEBUG", format!($($arg)*));
        }
    }};
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {{
        $crate::utils::log_line("INFO", format!($($arg)*));
    }};
}

#[macro_export]
macro_rules! warning {
    ($($arg:tt)*) => {{
        $crate::utils::log_line("WARN", format!($($arg)*));
    }};
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {{
        $crate::utils::log_line("ERROR", format!($($arg)*));
    }};
}
