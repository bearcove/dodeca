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
    // backtrace and eprintln are not strictly signal-safe, but for debugging
    // purposes this is acceptable - we're already in a hung state.

    let process_name = PROCESS_NAME.load();
    let pid = std::process::id();

    eprintln!("\n{}", "=".repeat(60));
    eprintln!("[{process_name}] SIGUSR1 received - dumping stack traces (pid={pid})");
    eprintln!("{}\n", "=".repeat(60));

    // Forward SIGUSR1 to all registered child processes
    if let Ok(pids) = CHILD_PIDS.read()
        && !pids.is_empty()
    {
        eprintln!(
            "[{process_name}] Forwarding SIGUSR1 to {} child processes: {:?}",
            pids.len(),
            *pids
        );
        for &child_pid in pids.iter() {
            unsafe {
                libc::kill(child_pid as i32, libc::SIGUSR1);
            }
        }
        // Give children a moment to print their traces
        // (not ideal in signal handler, but we're debugging)
        unsafe {
            libc::usleep(100_000); // 100ms
        }
    }

    // Get current thread's backtrace
    let bt = backtrace::Backtrace::new();
    eprintln!("[{process_name}] Main thread backtrace:\n{bt:?}");

    // Note: Getting backtraces of OTHER threads from a signal handler is tricky.
    // The backtrace crate only captures the current thread.
    // For a full solution, we'd need to use ptrace or /proc/self/task/*/stack

    // Let's try reading /proc/self/task to list all threads
    if let Ok(entries) = std::fs::read_dir("/proc/self/task") {
        let tids: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()))
            .collect();

        eprintln!("[{process_name}] Thread IDs in this process: {tids:?}");

        // Full kernel stacks first (for detailed analysis)
        for tid in &tids {
            if let Ok(stack) = std::fs::read_to_string(format!("/proc/self/task/{tid}/stack")) {
                eprintln!("[{process_name}] Thread {tid} kernel stack:\n{stack}");
            }
            // Also try to get the thread's status
            if let Ok(status) = std::fs::read_to_string(format!("/proc/self/task/{tid}/status")) {
                // Extract just the State line
                for line in status.lines() {
                    if line.starts_with("State:") || line.starts_with("Name:") {
                        eprintln!("[{process_name}] Thread {tid}: {line}");
                    }
                }
            }
        }

        // CONCISE SUMMARY at the end: Show top 3 kernel stack frames for each thread
        eprintln!("\n[{process_name}] === THREAD SUMMARY (top 3 frames) ===");
        for tid in &tids {
            let name = std::fs::read_to_string(format!("/proc/self/task/{tid}/comm"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "?".to_string());
            let state = std::fs::read_to_string(format!("/proc/self/task/{tid}/status"))
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find(|l| l.starts_with("State:"))
                        .map(|l| l.trim_start_matches("State:").trim().to_string())
                })
                .unwrap_or_else(|| "?".to_string());

            let top_frames: String =
                std::fs::read_to_string(format!("/proc/self/task/{tid}/stack"))
                    .map(|s| {
                        s.lines()
                            .take(3)
                            .map(|l| {
                                // Extract just the function name from kernel stack line
                                // Format: "[<addr>] func_name+0x..."
                                if let Some(start) = l.find(']') {
                                    l[start + 1..]
                                        .trim()
                                        .split('+')
                                        .next()
                                        .unwrap_or("?")
                                        .trim()
                                } else {
                                    l.trim()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" → ")
                    })
                    .unwrap_or_else(|_| "?".to_string());

            eprintln!("  [{tid}] {name} ({state}): {top_frames}");
        }
        eprintln!("[{process_name}] === END SUMMARY ===");
    }

    // Call registered diagnostic callbacks
    if let Ok(callbacks) = DIAGNOSTIC_CALLBACKS.read()
        && !callbacks.is_empty()
    {
        eprintln!(
            "\n[{process_name}] Running {} diagnostic callback(s)...",
            callbacks.len()
        );
        for callback in callbacks.iter() {
            callback();
        }
    }

    eprintln!("\n{}", "=".repeat(60));
    eprintln!("[{process_name}] End of stack trace dump");
    eprintln!("{}\n", "=".repeat(60));
}
