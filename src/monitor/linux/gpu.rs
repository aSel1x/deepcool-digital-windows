//! GPU monitoring — Linux (AMD sysfs, Intel DRM, NVIDIA NVML).
//!
//! As in `cpu.rs`, every getter returns `Option` and nothing here exits the
//! process. A metric that cannot be read honestly reports "absent" rather than
//! a number that looks live.

use super::cpu::read_u64;
use crate::monitor::scale_temp;
use libloading::{Library, Symbol};
use std::ffi::CString;
use std::fs::{read_dir, read_to_string};
use std::os::raw::{c_char, c_int};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};

// ── AMD GPU ────────────────────────────────────────────────────────────────

pub(crate) struct AmdGpu {
    usage_file: PathBuf,
    hwmon_dir: PathBuf,
}

impl AmdGpu {
    fn new(pci_address: &str) -> Option<Self> {
        let path = PathBuf::from("/sys/bus/pci/devices").join(pci_address);
        Some(Self {
            usage_file: find_amd_card(&path)?,
            hwmon_dir: find_amd_hwmon(&path)?,
        })
    }

    fn get_temp(&self, fahrenheit: bool) -> Option<u8> {
        let milli = read_u64(&self.hwmon_dir.join("temp1_input"))?;
        Some(scale_temp(milli as f64 / 1000.0, fahrenheit))
    }

    fn get_usage(&self) -> Option<u8> {
        read_u64(&self.usage_file).map(|v| v.min(100) as u8)
    }

    fn get_power(&self) -> Option<u16> {
        // power1_average is microwatts; some cards only expose power1_input.
        let micro = read_u64(&self.hwmon_dir.join("power1_average"))
            .or_else(|| read_u64(&self.hwmon_dir.join("power1_input")))?;
        Some((micro / 1_000_000).min(u16::MAX as u64) as u16)
    }

    fn get_frequency(&self) -> Option<u16> {
        let hz = read_u64(&self.hwmon_dir.join("freq1_input"))?;
        Some((hz / 1_000_000).min(u16::MAX as u64) as u16)
    }

    fn get_vram_usage(&self) -> Option<u8> {
        let dir = self.usage_file.parent()?;
        let used = read_u64(&dir.join("mem_info_vram_used"))?;
        let total = read_u64(&dir.join("mem_info_vram_total"))?;
        if total == 0 {
            return None;
        }
        Some(((used as f64 / total as f64) * 100.0).round().clamp(0.0, 100.0) as u8)
    }
}

fn find_amd_card(path: &Path) -> Option<PathBuf> {
    let data = read_to_string(path.join("uevent")).ok()?;
    if data.lines().next()?.ends_with("amdgpu") {
        let f = path.join("gpu_busy_percent");
        f.exists().then_some(f)
    } else {
        None
    }
}

fn find_amd_hwmon(path: &Path) -> Option<PathBuf> {
    let hwmon_path = read_dir(path.join("hwmon")).ok()?.flatten().next()?.path();
    let name = read_to_string(hwmon_path.join("name")).ok()?;
    if name.starts_with("amdgpu") {
        Some(hwmon_path)
    } else {
        None
    }
}

// ── Intel GPU ──────────────────────────────────────────────────────────────

pub(crate) struct IntelGpu {
    drm_dir: Option<PathBuf>,
    hwmon_dir: Option<PathBuf>,
    /// True when the only hwmon we could find belongs to the CPU package.
    ///
    /// In that case there is no GPU temperature to report. The previous code
    /// returned the coretemp reading, i.e. CPU package temperature labelled as
    /// GPU temperature, with no warning.
    shared_cpu_hwmon: bool,
    warned_usage: AtomicBool,
}

impl IntelGpu {
    fn new(pci_address: &str) -> Option<Self> {
        let pci_path = PathBuf::from("/sys/bus/pci/devices").join(pci_address);
        let drm_dir = find_intel_drm_dir(&pci_path);
        let (hwmon_dir, shared) = match find_intel_hwmon() {
            Some((dir, shared)) => (Some(dir), shared),
            None => (None, false),
        };
        Some(Self {
            drm_dir,
            hwmon_dir,
            shared_cpu_hwmon: shared,
            warned_usage: AtomicBool::new(false),
        })
    }

    fn get_temp(&self, fahrenheit: bool) -> Option<u8> {
        if self.shared_cpu_hwmon {
            return None;
        }
        let dir = self.hwmon_dir.as_ref()?;
        if let Some(milli) = read_u64(&dir.join("temp1_input")) {
            return Some(scale_temp(milli as f64 / 1000.0, fahrenheit));
        }
        for idx in 2..=5 {
            let label = read_to_string(dir.join(format!("temp{idx}_label"))).unwrap_or_default();
            let label = label.trim();
            if label.eq_ignore_ascii_case("pkg") || label.eq_ignore_ascii_case("package id 0") {
                let milli = read_u64(&dir.join(format!("temp{idx}_input")))?;
                return Some(scale_temp(milli as f64 / 1000.0, fahrenheit));
            }
        }
        None
    }

    /// Real utilisation only.
    ///
    /// The old implementation reported `gt_cur_freq / gt_max_freq` as "usage",
    /// which looks plausible and is not utilisation at all — an idle GPU parked
    /// at a high clock reads as busy, and a loaded one at low clock reads as
    /// idle. If no busy counter is exposed, say so once and report absent.
    fn get_usage(&self) -> Option<u8> {
        let drm = self.drm_dir.as_ref()?;
        for candidate in ["device/gpu_busy_percent", "gpu_busy_percent"] {
            if let Some(v) = read_u64(&drm.join(candidate)) {
                return Some(v.min(100) as u8);
            }
        }
        if !self.warned_usage.swap(true, Ordering::Relaxed) {
            crate::warning!("Intel GPU utilization is unavailable (no gpu_busy_percent counter exposed)");
        }
        None
    }

    fn get_power(&self) -> Option<u16> {
        if self.shared_cpu_hwmon {
            return None;
        }
        let dir = self.hwmon_dir.as_ref()?;
        let micro = read_u64(&dir.join("power1_average")).or_else(|| read_u64(&dir.join("power1_input")))?;
        Some((micro / 1_000_000).min(u16::MAX as u64) as u16)
    }

    fn get_frequency(&self) -> Option<u16> {
        if let Some(drm) = &self.drm_dir {
            for candidate in ["device/gt_cur_freq_mhz", "device/tile0/gt0/freq0/cur_freq"] {
                if let Some(mhz) = read_u64(&drm.join(candidate)) {
                    return Some(mhz.min(u16::MAX as u64) as u16);
                }
            }
        }
        let dir = self.hwmon_dir.as_ref()?;
        if self.shared_cpu_hwmon {
            return None;
        }
        let hz = read_u64(&dir.join("freq1_input"))?;
        Some((hz / 1_000_000).min(u16::MAX as u64) as u16)
    }
}

fn find_intel_drm_dir(path: &Path) -> Option<PathBuf> {
    let data = read_to_string(path.join("uevent")).ok()?;
    if !data
        .lines()
        .any(|l| l.contains("DRIVER=i915") || l.contains("DRIVER=xe"))
    {
        return None;
    }
    for dir in read_dir(path.join("drm")).ok()?.flatten() {
        let name = dir.file_name().to_string_lossy().to_string();
        if name.starts_with("card") {
            return Some(path.join("drm").join(name));
        }
    }
    None
}

/// Returns the hwmon directory and whether it is really the CPU package sensor.
fn find_intel_hwmon() -> Option<(PathBuf, bool)> {
    let mut fallback = None;
    for entry in read_dir("/sys/class/hwmon").ok()?.flatten() {
        let path = entry.path();
        let Ok(name) = read_to_string(path.join("name")) else {
            continue;
        };
        let name = name.trim();
        if name.contains("xe") || name.contains("i915") || name.contains("intel_arc") || name.contains("drm") {
            return Some((path, false));
        }
        if name == "coretemp" {
            fallback = Some(path);
        }
    }
    fallback.map(|p| (p, true))
}

// ── NVIDIA GPU ─────────────────────────────────────────────────────────────

// nvmlReturn_t is a C enum, i.e. `int`. Declaring these as `-> u16` read only
// half of the return register.
type NvmlInit = unsafe extern "C" fn() -> c_int;
type NvmlShutdown = unsafe extern "C" fn() -> c_int;
type NvmlGetHandle = unsafe extern "C" fn(*const c_char, *mut *mut u8) -> c_int;
type NvmlGetUtilization = unsafe extern "C" fn(*mut u8, *mut Utilization) -> c_int;
type NvmlGetTemp = unsafe extern "C" fn(*mut u8, u32, *mut u32) -> c_int;
type NvmlGetPower = unsafe extern "C" fn(*mut u8, *mut u32) -> c_int;
type NvmlGetClock = unsafe extern "C" fn(*mut u8, u32, *mut u32) -> c_int;
type NvmlGetMemory = unsafe extern "C" fn(*mut u8, *mut MemoryInfo) -> c_int;

#[repr(C)]
struct Utilization {
    gpu: u32,
    memory: u32,
}

#[repr(C)]
struct MemoryInfo {
    total: u64,
    free: u64,
    used: u64,
}

const NVML_SUCCESS: c_int = 0;

const NVML_LIBS: [&str; 12] = [
    "/usr/lib/x86_64-linux-gnu/nvidia/current/libnvidia-ml.so",
    "/usr/lib/x86_64-linux-gnu/nvidia/current/libnvidia-ml.so.1",
    "/usr/lib/x86_64-linux-gnu/libnvidia-ml.so",
    "/usr/lib/x86_64-linux-gnu/libnvidia-ml.so.1",
    "/usr/lib/libnvidia-ml.so",
    "/usr/lib/libnvidia-ml.so.1",
    "/usr/lib64/libnvidia-ml.so",
    "/usr/lib64/libnvidia-ml.so.1",
    "/usr/lib32/libnvidia-ml.so",
    "/usr/lib32/libnvidia-ml.so.1",
    "/run/opengl-driver/lib/libnvidia-ml.so",
    "/run/opengl-driver/lib/libnvidia-ml.so.1",
];

pub(crate) struct NvidiaGpu {
    lib: Library,
    device: *mut u8,
}
unsafe impl Send for NvidiaGpu {}

impl NvidiaGpu {
    fn new(pci_address: &str) -> Option<Self> {
        unsafe {
            let mut lib = Library::new("libnvidia-ml.so.1")
                .or_else(|_| Library::new("libnvidia-ml.so"))
                .ok();
            if lib.is_none() {
                for path in NVML_LIBS {
                    if Path::new(path).exists() {
                        if let Ok(l) = Library::new(path) {
                            lib = Some(l);
                            break;
                        }
                    }
                }
            }
            let lib = lib?;

            let init: Symbol<NvmlInit> = lib.get(b"nvmlInit_v2\0").ok()?;
            if init() != NVML_SUCCESS {
                crate::warning!("Failed to initialize NVML; NVIDIA metrics will be unavailable");
                return None;
            }

            // NVML expects a NUL-terminated C string. Passing `&str::as_ptr()`
            // let it strlen past the end of the allocation — undefined behaviour
            // whose benign outcome was a silently failing lookup.
            // Anything that fails from here on must undo nvmlInit_v2, otherwise
            // the driver keeps a reference for the life of the process.
            let shutdown = |lib: &Library| {
                if let Ok(f) = lib.get::<NvmlShutdown>(b"nvmlShutdown\0") {
                    let _ = f();
                }
            };

            let Ok(bus_id) = CString::new(pci_address) else {
                shutdown(&lib);
                return None;
            };
            let mut device: *mut u8 = null_mut();
            let Ok(get_handle) = lib.get::<NvmlGetHandle>(b"nvmlDeviceGetHandleByPciBusId_v2\0") else {
                shutdown(&lib);
                return None;
            };
            if get_handle(bus_id.as_ptr(), &mut device) != NVML_SUCCESS {
                crate::debug!("NVML has no handle for PCI address {pci_address}");
                shutdown(&lib);
                return None;
            }
            Some(Self { lib, device })
        }
    }

    /// Calls an NVML getter, returning `None` on any driver-reported failure.
    fn call<T, F>(&self, symbol: &[u8], f: F) -> Option<T>
    where
        F: FnOnce(&Library, *mut u8) -> Option<T>,
    {
        // Symbol lookup itself can fail on older drivers; never unwrap it.
        unsafe {
            if self.lib.get::<*const ()>(symbol).is_err() {
                return None;
            }
        }
        f(&self.lib, self.device)
    }

    fn get_temp(&self, fahrenheit: bool) -> Option<u8> {
        self.call(b"nvmlDeviceGetTemperature\0", |lib, dev| unsafe {
            let f: Symbol<NvmlGetTemp> = lib.get(b"nvmlDeviceGetTemperature\0").ok()?;
            let mut temp: u32 = 0;
            (f(dev, 0, &mut temp) == NVML_SUCCESS).then(|| scale_temp(temp as f64, fahrenheit))
        })
    }

    fn get_usage(&self) -> Option<u8> {
        self.call(b"nvmlDeviceGetUtilizationRates\0", |lib, dev| unsafe {
            let f: Symbol<NvmlGetUtilization> = lib.get(b"nvmlDeviceGetUtilizationRates\0").ok()?;
            let mut u = Utilization { gpu: 0, memory: 0 };
            (f(dev, &mut u) == NVML_SUCCESS).then(|| u.gpu.min(100) as u8)
        })
    }

    fn get_power(&self) -> Option<u16> {
        self.call(b"nvmlDeviceGetPowerUsage\0", |lib, dev| unsafe {
            let f: Symbol<NvmlGetPower> = lib.get(b"nvmlDeviceGetPowerUsage\0").ok()?;
            let mut milliwatts: u32 = 0;
            (f(dev, &mut milliwatts) == NVML_SUCCESS)
                .then(|| (milliwatts as f64 / 1000.0).round().clamp(0.0, u16::MAX as f64) as u16)
        })
    }

    fn get_frequency(&self) -> Option<u16> {
        self.call(b"nvmlDeviceGetClockInfo\0", |lib, dev| unsafe {
            let f: Symbol<NvmlGetClock> = lib.get(b"nvmlDeviceGetClockInfo\0").ok()?;
            let mut mhz: u32 = 0;
            (f(dev, 0, &mut mhz) == NVML_SUCCESS).then(|| mhz.min(u16::MAX as u32) as u16)
        })
    }

    fn get_vram_usage(&self) -> Option<u8> {
        self.call(b"nvmlDeviceGetMemoryInfo\0", |lib, dev| unsafe {
            let f: Symbol<NvmlGetMemory> = lib.get(b"nvmlDeviceGetMemoryInfo\0").ok()?;
            let mut mem = MemoryInfo {
                total: 0,
                free: 0,
                used: 0,
            };
            if f(dev, &mut mem) != NVML_SUCCESS || mem.total == 0 {
                return None;
            }
            Some(((mem.used as f64 / mem.total as f64) * 100.0).round().clamp(0.0, 100.0) as u8)
        })
    }
}

impl Drop for NvidiaGpu {
    fn drop(&mut self) {
        unsafe {
            if let Ok(shutdown) = self.lib.get::<NvmlShutdown>(b"nvmlShutdown\0") {
                let _ = shutdown();
            }
        }
    }
}

// ── Gpu enum ───────────────────────────────────────────────────────────────

pub enum Gpu {
    Amd(AmdGpu),
    Intel(IntelGpu),
    Nvidia(NvidiaGpu),
    None,
}

impl Gpu {
    /// Auto-detects the GPU, honouring `[device] gpu` when set.
    pub fn new() -> Self {
        let wanted = crate::config::AppConfig::get()
            .device
            .as_ref()
            .and_then(|d| d.gpu.clone());
        match wanted {
            Some(spec) => match Self::select(&spec) {
                Some(gpu) => gpu,
                None => {
                    crate::warning!("[device] gpu = \"{spec}\" did not match any GPU; falling back to auto-detection");
                    Self::autodetect()
                }
            },
            None => Self::autodetect(),
        }
    }

    /// Resolves `vendor:index` or a raw PCI address to a specific GPU.
    fn select(spec: &str) -> Option<Self> {
        let candidates = Self::candidates();
        // A raw PCI address wins if it matches exactly.
        if let Some((driver, _, addr)) = candidates.iter().find(|(_, _, a)| a == spec) {
            return Self::open(driver, addr);
        }
        let (vendor, index) = spec.split_once(':')?;
        let index: usize = index.trim().parse().ok()?;
        let driver_for = match vendor.trim().to_ascii_lowercase().as_str() {
            "nvidia" => "nvidia",
            "amd" | "amdgpu" => "amdgpu",
            "intel" => "i915",
            _ => return None,
        };
        let matching: Vec<&(String, String, String)> = candidates
            .iter()
            .filter(|(d, _, _)| d == driver_for || (driver_for == "i915" && d == "xe"))
            .collect();
        let (driver, _, addr) = matching.get(index)?;
        Self::open(driver, addr)
    }

    fn open(driver: &str, pci_addr: &str) -> Option<Self> {
        match driver {
            "nvidia" => NvidiaGpu::new(pci_addr).map(Gpu::Nvidia),
            "amdgpu" => AmdGpu::new(pci_addr).map(Gpu::Amd),
            "xe" | "i915" => IntelGpu::new(pci_addr).map(Gpu::Intel),
            _ => None,
        }
    }

    /// Every PCI device with a driver, sorted by address so the ordering is
    /// stable across boots rather than following directory-read order.
    fn candidates() -> Vec<(String, String, String)> {
        let mut candidates: Vec<(String, String, String)> = read_dir("/sys/bus/pci/devices")
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|dir| {
                let data = read_to_string(dir.path().join("uevent")).ok()?;

                let mut driver = None;
                let mut pci_id = None;
                for line in data.lines() {
                    if let Some(v) = line.strip_prefix("DRIVER=") {
                        driver = Some(v.to_string());
                    }
                    if let Some(v) = line.strip_prefix("PCI_ID=") {
                        pci_id = Some(v.to_string());
                    }
                }
                let pci_addr = dir.file_name().to_string_lossy().to_string();
                Some((driver?, pci_id?, pci_addr))
            })
            .collect();
        candidates.sort_by(|a, b| a.2.cmp(&b.2));
        candidates
    }

    fn autodetect() -> Self {
        let candidates = Self::candidates();

        // Prefer a discrete GPU (NVIDIA, then AMD) over an integrated Intel GPU,
        // since the latter is almost never the intended target on hybrid graphics.
        for (driver, _, pci_addr) in &candidates {
            if driver == "nvidia" {
                if let Some(gpu) = NvidiaGpu::new(pci_addr) {
                    return Gpu::Nvidia(gpu);
                }
            }
        }
        for (driver, _, pci_addr) in &candidates {
            if driver == "amdgpu" {
                if let Some(gpu) = AmdGpu::new(pci_addr) {
                    return Gpu::Amd(gpu);
                }
            }
        }
        for (driver, pci_id, pci_addr) in &candidates {
            let is_discrete_i915 = pci_id.len() >= 7 && ["56", "E2"].contains(&&pci_id[5..7]);
            if driver == "xe" || (driver == "i915" && is_discrete_i915) {
                if let Some(gpu) = IntelGpu::new(pci_addr) {
                    return Gpu::Intel(gpu);
                }
            }
        }
        Gpu::None
    }

    pub fn warn_missing(&self) {
        if matches!(self, Gpu::None) {
            crate::warning!("No supported GPU was found");
            eprintln!(" GPU information will not be displayed.");
        }
    }

    pub fn get_temp(&self, fahrenheit: bool) -> Option<u8> {
        match self {
            Gpu::Amd(g) => g.get_temp(fahrenheit),
            Gpu::Intel(g) => g.get_temp(fahrenheit),
            Gpu::Nvidia(g) => g.get_temp(fahrenheit),
            Gpu::None => None,
        }
    }

    pub fn get_usage(&self) -> Option<u8> {
        match self {
            Gpu::Amd(g) => g.get_usage(),
            Gpu::Intel(g) => g.get_usage(),
            Gpu::Nvidia(g) => g.get_usage(),
            Gpu::None => None,
        }
    }

    pub fn get_power(&self) -> Option<u16> {
        match self {
            Gpu::Amd(g) => g.get_power(),
            Gpu::Intel(g) => g.get_power(),
            Gpu::Nvidia(g) => g.get_power(),
            Gpu::None => None,
        }
    }

    pub fn get_frequency(&self) -> Option<u16> {
        match self {
            Gpu::Amd(g) => g.get_frequency(),
            Gpu::Intel(g) => g.get_frequency(),
            Gpu::Nvidia(g) => g.get_frequency(),
            Gpu::None => None,
        }
    }

    pub fn get_vram_usage(&self) -> Option<u8> {
        match self {
            Gpu::Amd(g) => g.get_vram_usage(),
            // Neither i915 nor xe exposes a VRAM utilisation counter in sysfs.
            Gpu::Intel(_) => None,
            Gpu::Nvidia(g) => g.get_vram_usage(),
            Gpu::None => None,
        }
    }
}

impl Default for Gpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Gpu {
    /// A GPU handle that reports every metric as absent. Test-only, and the
    /// same signature exists on Windows so driver tests are portable.
    #[cfg(test)]
    pub fn absent() -> Self {
        Gpu::None
    }
}
