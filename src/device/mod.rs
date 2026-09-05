//! Device communication trait.
//!
//! Linux:   `HidDevice` (обёртка над hidapi)
//! Windows: `WinUsbDevice` (нативный WinUSB), с опциональным HID-фолбэком
//!
//! # Framing contract
//!
//! The two write methods have *different* wire semantics, and picking the wrong
//! one silently corrupts the stream. They are separate methods, not a length
//! heuristic, so the caller has to state which one it means:
//!
//! * [`Device::write_report`] — a 64-byte HID output report whose **byte 0 is
//!   the report id**. This is what every segment device speaks.
//! * [`Device::write_bulk`] — transmit exactly these bytes, no framing, no
//!   report id. This is what the LM-series framebuffer stream needs. A backend
//!   that cannot do this reports [`Transport::Hid`] from [`Device::transport`]
//!   so the caller can refuse instead of sending garbage.

use crate::error::{Error, Result};

#[cfg(any(target_os = "linux", all(windows, feature = "windows-hid-fallback")))]
mod hid;
#[cfg(windows)]
mod winusb;

#[cfg(any(target_os = "linux", all(windows, feature = "windows-hid-fallback")))]
pub use hid::HidDevice;
#[cfg(windows)]
pub use winusb::WinUsbDevice;

/// Read timeout shared by every backend, so the two cannot drift apart.
pub const IN_TRANSFER_TIMEOUT_MS: u32 = 5_000;
/// Write timeout for the OUT pipe. Generous: a full 150 KB frame is one transfer.
#[cfg(windows)]
pub const OUT_TRANSFER_TIMEOUT_MS: u32 = 30_000;

/// What the underlying transport can actually express on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// Report-framed. Payloads are chunked into HID reports with a report id.
    ///
    /// Only produced by the HID backend, which is Linux-only unless the
    /// `windows-hid-fallback` feature is on.
    #[cfg_attr(
        not(any(target_os = "linux", all(windows, feature = "windows-hid-fallback"))),
        allow(dead_code)
    )]
    Hid,
    /// Raw byte stream. Arbitrary-length transfers arrive exactly as written.
    Bulk,
}

pub trait Device: Send {
    /// Sends a 64-byte HID output report. `data[0]` is the report id.
    fn write_report(&mut self, data: &[u8; 64]) -> Result<()>;

    /// Sends a report of exactly `data.len()` bytes, without padding.
    ///
    /// The CH510 speaks a variable-length ASCII protocol and has always been
    /// sent its bytes unpadded; padding a text protocol out to 64 bytes is a
    /// different thing on the wire.
    fn write_report_exact(&mut self, data: &[u8]) -> Result<()>;

    /// Sends `data` verbatim as a bulk transfer.
    ///
    /// Only meaningful when [`Device::transport`] is [`Transport::Bulk`].
    fn write_bulk(&mut self, data: &[u8]) -> Result<()>;

    /// Reads a response, bounded by [`IN_TRANSFER_TIMEOUT_MS`].
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    fn transport(&self) -> Transport;

    /// Clears a stalled endpoint after a failed transfer, so the next frame has
    /// a chance of succeeding instead of inheriting the broken state.
    fn recover(&mut self) {}

    /// Shortens the read timeout for the shutdown path.
    ///
    /// Windows kills the process about 5 s after a console-close handler is
    /// entered, and the normal 5 s ACK timeout does not fit inside that.
    fn set_read_timeout(&mut self, _ms: u32) {}
}

impl Device for Box<dyn Device> {
    fn write_report(&mut self, data: &[u8; 64]) -> Result<()> {
        self.as_mut().write_report(data)
    }

    fn write_report_exact(&mut self, data: &[u8]) -> Result<()> {
        self.as_mut().write_report_exact(data)
    }

    fn write_bulk(&mut self, data: &[u8]) -> Result<()> {
        self.as_mut().write_bulk(data)
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.as_mut().read(buf)
    }

    fn transport(&self) -> Transport {
        self.as_ref().transport()
    }

    fn recover(&mut self) {
        self.as_mut().recover()
    }

    fn set_read_timeout(&mut self, ms: u32) {
        self.as_mut().set_read_timeout(ms)
    }
}

#[cfg(target_os = "linux")]
pub fn list_devices(vendor_id: u16) -> Result<Vec<u16>> {
    HidDevice::list_devices(vendor_id)
}

#[cfg(windows)]
pub fn list_devices(vendor_id: u16) -> Result<Vec<u16>> {
    {
        #[cfg_attr(not(feature = "windows-hid-fallback"), allow(unused_mut))]
        let mut ids = WinUsbDevice::list_devices(vendor_id)?;
        #[cfg(feature = "windows-hid-fallback")]
        {
            // A DeepCool device bound to the HID class driver instead of WinUSB
            // is invisible to the SetupDi enumeration above.
            if let Ok(hid_ids) = HidDevice::list_devices(vendor_id) {
                ids.extend(hid_ids);
                ids.sort_unstable();
                ids.dedup();
            }
        }
        Ok(ids)
    }
}

#[cfg(target_os = "linux")]
pub fn open(vendor_id: u16, product_id: u16) -> Result<Box<dyn Device>> {
    HidDevice::open(vendor_id, product_id).map(|d| Box::new(d) as Box<dyn Device>)
}

#[cfg(windows)]
pub fn open(vendor_id: u16, product_id: u16) -> Result<Box<dyn Device>> {
    {
        match WinUsbDevice::open(vendor_id, product_id) {
            Ok(d) => Ok(Box::new(d) as Box<dyn Device>),
            Err(e) => {
                #[cfg(feature = "windows-hid-fallback")]
                {
                    crate::debug!("WinUSB open failed ({e}), trying the HID fallback");
                    if let Ok(d) = HidDevice::open(vendor_id, product_id) {
                        crate::warning!("Opened the device through the HID fallback backend");
                        return Ok(Box::new(d) as Box<dyn Device>);
                    }
                }
                Err(e)
            }
        }
    }
}

/// Maps a backend error string onto [`Error::Access`] when it looks like a
/// permission problem, so the user gets the udev-rule / Administrator hint.
pub(crate) fn classify_open_error(vendor_id: u16, product_id: u16, source: String) -> Error {
    let lower = source.to_ascii_lowercase();
    // Match the numeric codes as whole numbers: a substring test for
    // "os error 1" also matches 10, 13, 19, 110...
    let code_is = |prefix: &str, wanted: &[u32]| -> bool {
        lower.split(prefix).skip(1).any(|tail| {
            let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u32>().map(|c| wanted.contains(&c)).unwrap_or(false)
        })
    };
    let denied = lower.contains("permission denied")
        || lower.contains("access is denied")
        || lower.contains("access denied")
        // errno EPERM (1) / EACCES (13); Win32 ERROR_ACCESS_DENIED (5).
        || code_is("os error ", &[1, 13])
        || code_is("win32 error ", &[5]);
    if denied {
        Error::Access {
            vendor_id,
            product_id,
            source,
        }
    } else {
        Error::Open {
            vendor_id,
            product_id,
            source,
        }
    }
}
