use super::{classify_open_error, Device, Transport, IN_TRANSFER_TIMEOUT_MS};
use crate::error::{Error, Result};
use hidapi::HidApi;

/// Distinguishes "the device is gone" from a recoverable transfer error.
///
/// Only the WinUSB backend used to produce `Disconnected`, so on Linux an
/// unplugged device made the frame loop retry forever instead of exiting.
fn classify_write_error(e: &hidapi::HidError) -> Error {
    let msg = e.to_string();
    let lower = msg.to_ascii_lowercase();
    if lower.contains("no such device")
        || lower.contains("not connected")
        || lower.contains("disconnected")
        || lower.contains("device removed")
        || lower.contains("os error 19")
        || lower.contains("os error 5")
    {
        Error::Disconnected(msg)
    } else {
        Error::Transfer(msg)
    }
}

pub struct HidDevice {
    device: hidapi::HidDevice,
}

impl HidDevice {
    pub fn list_devices(vendor_id: u16) -> Result<Vec<u16>> {
        let api = HidApi::new().map_err(|e| Error::Transfer(e.to_string()))?;
        let mut ids: Vec<u16> = api
            .device_list()
            .filter(|d| d.vendor_id() == vendor_id)
            .map(|d| d.product_id())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    pub fn open(vendor_id: u16, product_id: u16) -> Result<Self> {
        let api = HidApi::new().map_err(|e| Error::Transfer(e.to_string()))?;
        let device = api
            .open(vendor_id, product_id)
            .map_err(|e| classify_open_error(vendor_id, product_id, e.to_string()))?;
        // Bound every read the same way the WinUSB backend does, so a device
        // that stops ACKing cannot wedge the frame loop on one platform only.
        device
            .set_blocking_mode(true)
            .map_err(|e| Error::Transfer(e.to_string()))?;
        Ok(Self { device })
    }
}

impl Device for HidDevice {
    fn write_report(&mut self, data: &[u8; 64]) -> Result<()> {
        self.write_report_exact(data)
    }

    fn write_report_exact(&mut self, data: &[u8]) -> Result<()> {
        self.device
            .write(data)
            .map(|_| ())
            .map_err(|e| classify_write_error(&e))
    }

    /// HID cannot carry an unframed byte stream.
    ///
    /// Chunking a bulk payload into output reports would consume byte 0 of every
    /// chunk as the report number and silently drop it — so this refuses instead.
    /// Callers check [`Device::transport`] first; this is the backstop.
    fn write_bulk(&mut self, _data: &[u8]) -> Result<()> {
        Err(Error::Transfer(
            "this device is attached through the HID backend, which cannot send raw bulk \
             transfers; the LM-series framebuffer protocol requires a WinUSB/libusb interface"
                .to_string(),
        ))
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        match self.device.read_timeout(buf, IN_TRANSFER_TIMEOUT_MS as i32) {
            Ok(0) => Err(Error::Timeout),
            Ok(n) => Ok(n),
            Err(e) => Err(classify_write_error(&e)),
        }
    }

    fn transport(&self) -> Transport {
        Transport::Hid
    }
}
