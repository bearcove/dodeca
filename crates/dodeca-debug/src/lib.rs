//! Debug utilities for dodeca processes.
//!
//! Provides SIGUSR1 handler that dumps stack traces of all threads and
//! optional transport diagnostics (ring status, doorbell pending bytes, etc.).
//!
//! Note: SIGUSR1 handling is only available on Unix platforms.

#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(unix)]
static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

// Static storage for child PIDs (for forwarding SIGUSR1)
#[cfg(unix)]
static CHILD_PIDS: std::sync::RwLock<Vec<u32>> = std::sync::RwLock::new(Vec::new());

// Static storage for diagnostic callbacks
#[cfg(unix)]
static DIAGNOSTIC_CALLBACKS: std::sync::RwLock<Vec<Box<dyn Fn() + Send + Sync>>> =
    std::sync::RwLock::new(Vec::new());

/// Register a diagnostic callback to be called on SIGUSR1.
///
/// The callback should print diagnostic information to stderr.
/// Multiple callbacks can be registered and will be called in order.
///
/// # Note
///
/// Callbacks are called from a signal handler context, so they should
/// be careful about what operations they perform. For debugging purposes,
/// simple stderr output and memory reads are acceptable.
///
/// This is a no-op on non-Unix platforms.
///
/// # Example
///
/// ```ignore
/// dodeca_debug::register_diagnostic(|| {
///     eprintln!("My diagnostic info: ...");
/// });
/// ```
#[cfg(unix)]
pub fn register_diagnostic<F>(callback: F)
where
    F: Fn() + Send + Sync + 'static,
{
    if let Ok(mut callbacks) = DIAGNOSTIC_CALLBACKS.write() {
        callbacks.push(Box::new(callback));
    }
}

/// Register a diagnostic callback to be called on SIGUSR1.
///
/// This is a no-op on non-Unix platforms.
#[cfg(not(unix))]
pub fn register_diagnostic<F>(_callback: F)
where
    F: Fn() + Send + Sync + 'static,
{
}

/// Register a child process PID for SIGUSR1 forwarding.
///
/// When the host receives SIGUSR1, it will forward it to all registered children.
///
/// This is a no-op on non-Unix platforms.
#[cfg(unix)]
pub fn register_child_pid(pid: u32) {
    if let Ok(mut pids) = CHILD_PIDS.write() {
        pids.push(pid);
    }
}

/// Register a child process PID for SIGUSR1 forwarding.
///
/// This is a no-op on non-Unix platforms.
#[cfg(not(unix))]
pub fn register_child_pid(_pid: u32) {}

/// Install a SIGUSR1 handler that dumps stack traces of all threads.
///
/// Call this early in main() for both host and plugins.
/// When the process receives SIGUSR1, it will print stack traces to stderr.
///
/// This is a no-op on non-Unix platforms.
///
/// # Example
///
/// ```ignore
/// fn main() {
///     dodeca_debug::install_sigusr1_handler("my-process");
///     // ... rest of main
/// }
/// ```
#[cfg(unix)]
pub fn install_sigusr1_handler(process_name: &'static str) {
    if HANDLER_INSTALLED.swap(true, Ordering::SeqCst) {
        return; // Already installed
    }

    // Store process name in a static for the signal handler
    PROCESS_NAME.store(process_name);

    unsafe {
        // `libc::sighandler_t` is an integer type on Unix targets in the `libc` crate, so
        // cast via a raw pointer to avoid the `function-casts-as-integer` lint.
        libc::signal(
            libc::SIGUSR1,
            sigusr1_handler as *const () as libc::sighandler_t,
        );
    }

    // Only print if RAPACE_DEBUG is set
    if std::env::var("RAPACE_DEBUG").is_ok() {
        eprintln!(
            "[{}] SIGUSR1 handler installed (send signal to dump stack traces)",
            process_name
        );
    }
}

/// Install a SIGUSR1 handler that dumps stack traces of all threads.
///
/// This is a no-op on non-Unix platforms.
#[cfg(not(unix))]
pub fn install_sigusr1_handler(_process_name: &'static str) {}

// Static storage for process name (signal handlers can't capture environment)
#[cfg(unix)]
static PROCESS_NAME: ProcessName = ProcessName::new();

#[cfg(unix)]
struct ProcessName {
    name: std::sync::atomic::AtomicPtr<&'static str>,
}

#[cfg(unix)]
impl ProcessName {
    const fn new() -> Self {
        Self {
            name: std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    fn store(&self, name: &'static str) {
        let boxed = Box::new(name);
        self.name.store(Box::into_raw(boxed), Ordering::SeqCst);
    }

    fn load(&self) -> &'static str {
        let ptr = self.name.load(Ordering::SeqCst);
        if ptr.is_null() {
            "unknown"
        } else {
            unsafe { *ptr }
        }
    }
}

#[cfg(unix)]
extern "C" fn sigusr1_handler(_sig: libc::c_int) {
    // SAFETY: We're in a signal handler, so we need to be careful.
    // eprintln is not strictly signal-safe, but for debugging
    // purposes this is acceptable - we're already in a hung state.

    let process_name = PROCESS_NAME.load();
    let pid = std::process::id();

    eprintln!("[{process_name}] SIGUSR1 (pid={pid})");

    // Forward SIGUSR1 to all registered child processes
    // Use try_read to avoid deadlock if lock is held by interrupted thread
    if let Ok(pids) = CHILD_PIDS.try_read()
        && !pids.is_empty()
    {
        eprintln!("[{process_name}] forwarding to {} children", pids.len());
        for &child_pid in pids.iter() {
            unsafe {
                libc::kill(child_pid as i32, libc::SIGUSR1);
            }
        }
        // Give children a moment to print their output
        unsafe {
            libc::usleep(100_000); // 100ms
        }
    }

    // Call registered diagnostic callbacks
    // Use try_read to avoid deadlock if lock is held by interrupted thread
    if let Ok(callbacks) = DIAGNOSTIC_CALLBACKS.try_read() {
        for callback in callbacks.iter() {
            callback();
        }
    }
}
