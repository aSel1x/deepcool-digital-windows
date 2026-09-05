//! Graceful shutdown.
//!
//! The display loop must be able to blank the panel before the process dies —
//! on the LM-series an unfinished transaction leaves the screen dark and
//! unresponsive until the machine reboots. Two platform details drive the design:
//!
//! * Windows kills the process as soon as a `CTRL_CLOSE`/`LOGOFF`/`SHUTDOWN`
//!   handler *returns*, whatever it returns. So the handler blocks until the
//!   loop reports that cleanup is done (bounded, because the OS only waits ~5s).
//! * On Linux the default disposition of SIGTERM is immediate termination, and
//!   `systemctl stop` sends SIGTERM — so it must be handled alongside SIGINT.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static CLEANUP_DONE: AtomicBool = AtomicBool::new(false);

/// Longest a shutdown handler will block waiting for the display loop.
#[cfg(windows)]
const CLEANUP_GRACE: Duration = Duration::from_millis(4_000);

/// Returns `true` once a termination signal has been received.
pub fn is_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

/// Called by the display loop once the panel has been left in a safe state.
pub fn notify_cleanup_done() {
    CLEANUP_DONE.store(true, Ordering::SeqCst);
}

/// Sleeps in short slices, returning early if shutdown is requested.
///
/// Returns `false` when the sleep was cut short. Device loops use this instead
/// of `thread::sleep` so Ctrl+C latency is bounded by the slice, not by the
/// update interval (or by a multi-second GIF frame delay).
pub fn sleep_interruptible(total: Duration) -> bool {
    const SLICE: Duration = Duration::from_millis(50);
    let mut left = total;
    while !left.is_zero() {
        if is_requested() {
            return false;
        }
        let step = left.min(SLICE);
        std::thread::sleep(step);
        left -= step;
    }
    !is_requested()
}

#[cfg(windows)]
fn wait_for_cleanup() {
    let deadline = std::time::Instant::now() + CLEANUP_GRACE;
    while !CLEANUP_DONE.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(windows)]
mod imp {
    use super::{wait_for_cleanup, SHUTDOWN_REQUESTED};
    use std::sync::atomic::Ordering;
    use windows::Win32::System::Console::{
        SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
        CTRL_SHUTDOWN_EVENT, PHANDLER_ROUTINE,
    };

    unsafe extern "system" fn handler(ctrl_type: u32) -> i32 {
        match ctrl_type {
            CTRL_C_EVENT | CTRL_BREAK_EVENT => {
                // The process keeps running; the display loop notices the flag,
                // blanks the panel and exits on its own.
                SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
                1
            }
            CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
                // Windows terminates the process the moment this returns, so
                // block here (on the handler's own thread) until the loop has
                // finished, or the OS grace period is nearly up.
                SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
                wait_for_cleanup();
                1
            }
            _ => 0,
        }
    }

    /// Install the console control handler. Call once at startup.
    pub fn install() {
        unsafe {
            // SAFETY: `handler` matches PHANDLER_ROUTINE's signature; the i32
            // return type is ABI-compatible with BOOL.
            let ptr: *const () = handler as *const ();
            let h: PHANDLER_ROUTINE = std::mem::transmute(ptr);
            let _ = SetConsoleCtrlHandler(h, true);
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::SHUTDOWN_REQUESTED;
    use std::sync::atomic::Ordering;

    /// Async-signal-safe: an atomic store and nothing else.
    extern "C" fn handler(_sig: libc::c_int) {
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    }

    pub fn install() {
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = handler as *const () as libc::sighandler_t;
            libc::sigemptyset(&mut sa.sa_mask);
            // No SA_RESTART: a blocking hidraw read should return EINTR so the
            // loop can observe the flag instead of finishing its 5s timeout.
            sa.sa_flags = 0;
            // SIGTERM is what systemctl stop / shutdown send; SIGHUP arrives when
            // the controlling terminal goes away. Both used to kill us outright.
            for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
                libc::sigaction(sig, &sa, std::ptr::null_mut());
            }
        }
    }
}

pub use imp::install;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interruptible_sleep_returns_early_once_requested() {
        // Not requested yet: a short sleep runs to completion.
        assert!(sleep_interruptible(Duration::from_millis(1)));

        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
        let start = std::time::Instant::now();
        assert!(!sleep_interruptible(Duration::from_secs(30)));
        assert!(start.elapsed() < Duration::from_secs(1));

        SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
    }
}
