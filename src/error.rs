//! Crate-wide error type.
//!
//! Every fallible boundary returns `Error` so callers can tell *which* failure
//! happened (and, for the transport, whether it is worth retrying). `exit()` is
//! reserved for `main`.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// No device with a known DeepCool vendor id is attached.
    NoDevice,

    /// A device matched, but the interface could not be opened.
    ///
    /// On Linux this is almost always a missing udev rule; on Windows it is
    /// usually a device bound to a driver other than WinUSB.
    Open {
        vendor_id: u16,
        product_id: u16,
        source: String,
    },

    /// Permission denied while opening the device (missing udev rule / not admin).
    Access {
        vendor_id: u16,
        product_id: u16,
        source: String,
    },

    /// The device is attached but this build has no driver for it.
    UnsupportedDevice { product_id: u16 },

    /// The device does not implement the requested display mode.
    UnsupportedMode {
        mode: &'static str,
        supported: Vec<&'static str>,
    },

    /// A USB/HID transfer failed.
    Transfer(String),

    /// The device did not answer within the read timeout.
    ///
    /// Reported by the HID backend; the WinUSB backend surfaces its own Win32
    /// timeout error instead.
    #[cfg_attr(
        not(any(target_os = "linux", all(windows, feature = "windows-hid-fallback"))),
        allow(dead_code)
    )]
    Timeout,

    /// The device stopped responding mid-session (unplugged, stalled pipe).
    #[cfg_attr(not(windows), allow(dead_code))]
    Disconnected(String),

    /// The configuration file could not be read or parsed.
    Config(String),

    /// An LCD asset (SVG template, image, GIF) could not be loaded or rendered.
    Asset(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoDevice => write!(f, "no DeepCool device was found"),
            Error::Open {
                vendor_id,
                product_id,
                source,
            } => write!(f, "failed to open device VID_{vendor_id:04X}/PID_{product_id:04X}: {source}"),
            Error::Access {
                vendor_id,
                product_id,
                source,
            } => write!(
                f,
                "permission denied opening device VID_{vendor_id:04X}/PID_{product_id:04X}: {source}"
            ),
            Error::UnsupportedDevice { product_id } => {
                write!(f, "device PID {product_id} is not supported yet")
            }
            Error::UnsupportedMode { mode, supported } => write!(
                f,
                "display mode \"{}\" is not supported by this device (supported: {})",
                mode,
                supported.join(" ")
            ),
            Error::Transfer(e) => write!(f, "USB transfer failed: {e}"),
            Error::Timeout => write!(f, "the device did not respond within the timeout"),
            Error::Disconnected(e) => write!(f, "the device stopped responding: {e}"),
            Error::Config(e) => write!(f, "{e}"),
            Error::Asset(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    /// A one-line hint printed under the error, when there is a useful next step.
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            Error::Access { .. } => Some(if cfg!(windows) {
                "Run the program as Administrator."
            } else {
                "Install a udev rule for the device, or run with sudo."
            }),
            Error::NoDevice => Some("Check that the device is connected and powered."),
            Error::UnsupportedDevice { .. } => Some("Please report the PID via GitHub issues so support can be added."),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
