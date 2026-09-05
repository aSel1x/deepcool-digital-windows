//! Native WinUSB backend.

use super::{classify_open_error, Device, Transport, IN_TRANSFER_TIMEOUT_MS, OUT_TRANSFER_TIMEOUT_MS};
use crate::error::{Error, Result};
use std::ffi::c_void;
use std::mem::size_of;
use windows::{
    core::PCWSTR,
    Win32::{
        Devices::DeviceAndDriverInstallation::{
            SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
            SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA,
            SP_DEVICE_INTERFACE_DETAIL_DATA_W,
        },
        Devices::Usb::{
            UsbdPipeTypeBulk, UsbdPipeTypeInterrupt, WinUsb_FlushPipe, WinUsb_Free, WinUsb_Initialize,
            WinUsb_QueryInterfaceSettings, WinUsb_QueryPipe, WinUsb_ReadPipe, WinUsb_ResetPipe, WinUsb_SetPipePolicy,
            WinUsb_WritePipe, GUID_DEVINTERFACE_USB_DEVICE, PIPE_TRANSFER_TIMEOUT, USBD_PIPE_TYPE,
            WINUSB_INTERFACE_HANDLE, WINUSB_PIPE_INFORMATION,
        },
        Foundation::{CloseHandle, GetLastError, ERROR_NO_MORE_ITEMS, HANDLE},
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
    },
};

/// Upper bound on the interface enumeration.
///
/// The loop skips indices whose details cannot be read, so without a bound a
/// persistently failing enumeration would spin forever.
const MAX_INTERFACES: u32 = 1024;

/// Largest single WinUsb_WritePipe transfer. A 320x240 RGB565 frame is 153,600
/// bytes; splitting it keeps each transfer well inside the default WinUSB
/// MAXIMUM_TRANSFER_SIZE on every driver version.
const MAX_CHUNK: usize = 64 * 1024;

pub struct WinUsbDevice {
    handle: HANDLE,
    interface: WINUSB_INTERFACE_HANDLE,
    out_pipe: u8,
    in_pipe: u8,
}

/// One step of the interface enumeration.
enum IfaceStep {
    /// A usable device path.
    Path(String),
    /// This index failed, but later indices may still be valid.
    Skip,
    /// The enumeration is genuinely finished.
    Done,
}

impl WinUsbDevice {
    pub fn list_devices(vendor_id: u16) -> Result<Vec<u16>> {
        let vid = format!("VID_{vendor_id:04X}");
        let mut ids = Vec::new();

        unsafe {
            let info = SetupDiGetClassDevsW(
                Some(&GUID_DEVINTERFACE_USB_DEVICE),
                None,
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
            .map_err(|e| Error::Transfer(werr(e)))?;
            let _g = InfoGuard(info);

            for i in 0..MAX_INTERFACES {
                match iface_path(info, i) {
                    // A single interface whose details cannot be read must not
                    // hide every device enumerated after it.
                    IfaceStep::Skip => continue,
                    IfaceStep::Done => break,
                    IfaceStep::Path(path) => {
                        let u = path.to_ascii_uppercase();
                        if u.contains(&vid) {
                            if let Some(pid) = extract_pid(&u) {
                                ids.push(pid);
                            }
                        }
                    }
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    pub fn open(vendor_id: u16, product_id: u16) -> Result<Self> {
        let vid = format!("VID_{vendor_id:04X}");
        let pid = format!("PID_{product_id:04X}");

        unsafe {
            let info = SetupDiGetClassDevsW(
                Some(&GUID_DEVINTERFACE_USB_DEVICE),
                None,
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
            .map_err(|e| Error::Transfer(werr(e)))?;
            let _g = InfoGuard(info);

            let mut last_err = None;
            let mut found = false;

            for i in 0..MAX_INTERFACES {
                let path = match iface_path(info, i) {
                    IfaceStep::Skip => continue,
                    IfaceStep::Done => break,
                    IfaceStep::Path(p) => p,
                };
                let u = path.to_ascii_uppercase();

                if u.contains(&vid) && u.contains(&pid) {
                    found = true;
                    match open_iface(&path) {
                        Ok(dev) => return Ok(dev),
                        Err(e) => last_err = Some(e),
                    }
                }
            }

            if found {
                Err(classify_open_error(
                    vendor_id,
                    product_id,
                    last_err.unwrap_or_else(|| "failed to open device interface".into()),
                ))
            } else {
                Err(Error::Open {
                    vendor_id,
                    product_id,
                    source: "no matching WinUSB interface".to_string(),
                })
            }
        }
    }

    /// Writes the whole slice, looping over partial transfers and chunking large
    /// frames. Returns only once every byte has been accepted.
    fn write_raw(&mut self, data: &[u8]) -> Result<()> {
        let mut offset = 0usize;
        while offset < data.len() {
            let end = (offset + MAX_CHUNK).min(data.len());
            let chunk = &data[offset..end];
            let mut written = 0u32;
            unsafe {
                WinUsb_WritePipe(self.interface, self.out_pipe, chunk, Some(&mut written), None)
                    .map_err(|e| Error::Transfer(werr(e)))?;
            }
            if written == 0 {
                return Err(Error::Disconnected(format!(
                    "write stalled after {offset}/{} bytes",
                    data.len()
                )));
            }
            offset += written as usize;
        }
        Ok(())
    }
}

impl Device for WinUsbDevice {
    fn write_report(&mut self, data: &[u8; 64]) -> Result<()> {
        self.write_raw(data)
    }

    fn write_report_exact(&mut self, data: &[u8]) -> Result<()> {
        self.write_raw(data)
    }

    fn write_bulk(&mut self, data: &[u8]) -> Result<()> {
        self.write_raw(data)
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let mut read_count = 0u32;
        unsafe {
            WinUsb_ReadPipe(self.interface, self.in_pipe, Some(buf), Some(&mut read_count), None)
                .map_err(|e| Error::Transfer(werr(e)))?;
        }
        Ok(read_count as usize)
    }

    fn transport(&self) -> Transport {
        Transport::Bulk
    }

    fn set_read_timeout(&mut self, ms: u32) {
        set_pipe_timeout(self.interface, self.in_pipe, ms);
    }

    /// Clears a stalled pipe so the next frame starts from a known state instead
    /// of inheriting the failure. Without this, one timeout ends the session.
    fn recover(&mut self) {
        unsafe {
            let _ = WinUsb_ResetPipe(self.interface, self.out_pipe);
            let _ = WinUsb_ResetPipe(self.interface, self.in_pipe);
            let _ = WinUsb_FlushPipe(self.interface, self.in_pipe);
        }
    }
}

impl Drop for WinUsbDevice {
    fn drop(&mut self) {
        unsafe {
            let _ = WinUsb_Free(self.interface);
            let _ = CloseHandle(self.handle);
        }
    }
}

unsafe impl Send for WinUsbDevice {}

// ── Helpers ──────────────────────────────────────────────────────────

fn set_pipe_timeout(iface: WINUSB_INTERFACE_HANDLE, pipe: u8, timeout_ms: u32) {
    unsafe {
        let _ = WinUsb_SetPipePolicy(
            iface,
            pipe,
            PIPE_TRANSFER_TIMEOUT,
            size_of::<u32>() as u32,
            &timeout_ms as *const u32 as *const c_void,
        );
    }
}

fn open_iface(path: &str) -> std::result::Result<WinUsbDevice, String> {
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    unsafe {
        // FILE_FLAG_OVERLAPPED is required by WinUsb_Initialize. Transfers below
        // pass a NULL OVERLAPPED, which makes WinUSB run them synchronously.
        let handle = CreateFileW(
            PCWSTR(wide.as_ptr()),
            0x8000_0000 | 0x4000_0000, // GENERIC_READ | GENERIC_WRITE
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
            None,
        )
        .map_err(werr)?;

        let mut iface = WINUSB_INTERFACE_HANDLE::default();
        if let Err(e) = WinUsb_Initialize(handle, &mut iface) {
            let _ = CloseHandle(handle);
            return Err(werr(e));
        }

        let mut desc = Default::default();
        if let Err(e) = WinUsb_QueryInterfaceSettings(iface, 0, &mut desc) {
            let msg = werr(e);
            let _ = WinUsb_Free(iface);
            let _ = CloseHandle(handle);
            return Err(format!("failed to query interface settings: {msg}"));
        }

        // Prefer bulk endpoints; fall back to interrupt. Previously any endpoint
        // won, last one seen, and a failed query silently produced pipe 0x00 —
        // an invalid IN address that only failed later, at the first read.
        let mut out_pipe: Option<u8> = None;
        let mut in_pipe: Option<u8> = None;
        for i in 0..desc.bNumEndpoints {
            let mut info = WINUSB_PIPE_INFORMATION::default();
            if WinUsb_QueryPipe(iface, 0, i, &mut info).is_err() {
                continue;
            }
            if !is_stream_pipe(info.PipeType) {
                continue;
            }
            let slot = if (info.PipeId & 0x80) == 0 {
                &mut out_pipe
            } else {
                &mut in_pipe
            };
            let prefer = slot.is_none() || info.PipeType == UsbdPipeTypeBulk;
            if prefer {
                *slot = Some(info.PipeId);
            }
        }

        let (Some(out_pipe), Some(in_pipe)) = (out_pipe, in_pipe) else {
            let _ = WinUsb_Free(iface);
            let _ = CloseHandle(handle);
            return Err(format!(
                "no usable bulk/interrupt endpoint pair on this interface (OUT: {out_pipe:?}, IN: {in_pipe:?})"
            ));
        };

        set_pipe_timeout(iface, out_pipe, OUT_TRANSFER_TIMEOUT_MS);
        set_pipe_timeout(iface, in_pipe, IN_TRANSFER_TIMEOUT_MS);

        Ok(WinUsbDevice {
            handle,
            interface: iface,
            out_pipe,
            in_pipe,
        })
    }
}

fn is_stream_pipe(kind: USBD_PIPE_TYPE) -> bool {
    kind == UsbdPipeTypeBulk || kind == UsbdPipeTypeInterrupt
}

struct InfoGuard(windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO);
impl Drop for InfoGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

fn iface_path(info: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO, index: u32) -> IfaceStep {
    unsafe {
        let mut data = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        if SetupDiEnumDeviceInterfaces(info, None, &GUID_DEVINTERFACE_USB_DEVICE, index, &mut data).is_err() {
            // Only ERROR_NO_MORE_ITEMS means the list is exhausted.
            return if GetLastError() == ERROR_NO_MORE_ITEMS {
                IfaceStep::Done
            } else {
                IfaceStep::Skip
            };
        }
        let mut req = 0u32;
        let _ = SetupDiGetDeviceInterfaceDetailW(info, &data, None, 0, Some(&mut req), None);
        if req == 0 {
            return IfaceStep::Skip;
        }

        let count = req.div_ceil(size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32);
        let mut buf: Vec<SP_DEVICE_INTERFACE_DETAIL_DATA_W> = vec![Default::default(); count.max(1) as usize];
        buf[0].cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;

        if SetupDiGetDeviceInterfaceDetailW(info, &data, Some(buf.as_mut_ptr()), req, Some(&mut req), None).is_err() {
            return IfaceStep::Skip;
        }

        // The returned byte count covers the cbSize header too; subtract it so the
        // scan cannot run past the path characters.
        let header = size_of::<u32>();
        let chars = (req as usize).saturating_sub(header) / 2;
        match utf16_str((*buf.as_ptr()).DevicePath.as_ptr(), chars) {
            Some(p) => IfaceStep::Path(p),
            None => IfaceStep::Skip,
        }
    }
}

fn utf16_str(ptr: *const u16, max: usize) -> Option<String> {
    let mut len = 0;
    unsafe {
        while len < max && *ptr.add(len) != 0 {
            len += 1;
        }
    }
    if len == 0 {
        return None;
    }
    Some(unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len)) })
}

fn extract_pid(u: &str) -> Option<u16> {
    let p = u.find("PID_")?;
    u16::from_str_radix(u.get(p + 4..p + 8)?, 16).ok()
}

fn werr(e: windows::core::Error) -> String {
    format!("Win32 error {}: {}", e.code().0, e.message())
}

#[cfg(test)]
mod tests {
    use super::extract_pid;

    #[test]
    fn extracts_pid_from_device_path() {
        assert_eq!(
            extract_pid(r"\\?\USB#VID_3633&PID_0026#5&1B2C3D4E&0&2#{A5DCBF10}"),
            Some(0x0026)
        );
    }

    #[test]
    fn rejects_paths_without_a_pid() {
        assert_eq!(extract_pid(r"\\?\USB#VID_3633#5&1B2C3D4E"), None);
        // Truncated PID field must not panic on the slice.
        assert_eq!(extract_pid("PID_00"), None);
    }
}
