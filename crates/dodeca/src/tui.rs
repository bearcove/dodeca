//! TUI types for dodeca build progress
//!
//! Re-exports types from cell-tui-proto and provides channel helpers.

use std::sync::{Arc, Mutex};
use tokio::sync::watch;

// Re-export all types from the proto (the canonical definitions)
pub use cell_tui_proto::{
    BindMode, BuildProgress, EventKind, LogEvent, LogLevel, ServerCommand, ServerStatus,
    TaskProgress, TaskStatus,
};

// ============================================================================
// Local types for host-side use
// ============================================================================

/// Shared progress state for use across threads (legacy, for build mode)
pub type SharedProgress = Arc<Mutex<BuildProgress>>;

/// Create a new shared progress state (legacy, for build mode)
#[allow(dead_code)]
pub fn new_shared_progress() -> SharedProgress {
    Arc::new(Mutex::new(BuildProgress::default()))
}

// ============================================================================
// Channel-based types for serve mode
// ============================================================================

/// Progress sender - producers call send_modify to update progress
pub type ProgressTx = watch::Sender<BuildProgress>;
/// Progress receiver - TUI reads latest progress
pub type ProgressRx = watch::Receiver<BuildProgress>;

/// Create a new progress channel
pub fn progress_channel() -> (ProgressTx, ProgressRx) {
    watch::channel(BuildProgress::default())
}

/// Server status sender
pub type ServerStatusTx = watch::Sender<ServerStatus>;
/// Server status receiver
pub type ServerStatusRx = watch::Receiver<ServerStatus>;

/// Create a new server status channel
pub fn server_status_channel() -> (ServerStatusTx, ServerStatusRx) {
    watch::channel(ServerStatus::default())
}

/// Event sender - multiple producers can clone and send
pub type EventTx = std::sync::mpsc::Sender<LogEvent>;
/// Event receiver - TUI drains events
pub type EventRx = std::sync::mpsc::Receiver<LogEvent>;

/// Create a new event channel
pub fn event_channel() -> (EventTx, EventRx) {
    std::sync::mpsc::channel()
}

/// Helper to update progress - works with either SharedProgress or ProgressTx
pub enum ProgressReporter {
    /// Legacy mutex-based (for build command)
    Shared(SharedProgress),
    /// Channel-based (for serve mode)
    Channel(ProgressTx),
}

impl ProgressReporter {
    /// Update progress with a closure
    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut BuildProgress),
    {
        match self {
            ProgressReporter::Shared(p) => {
                let mut prog = p.lock().unwrap();
                f(&mut prog);
            }
            ProgressReporter::Channel(tx) => {
                tx.send_modify(f);
            }
        }
    }
}

/// Get all LAN (private) IPv4 addresses from network interfaces
pub fn get_lan_ips() -> Vec<std::net::Ipv4Addr> {
    if let Ok(interfaces) = if_addrs::get_if_addrs() {
        interfaces
            .into_iter()
            .filter_map(|iface| {
                if let if_addrs::IfAddr::V4(addr) = iface.addr {
                    let ip = addr.ip;
                    // Include private IPs but not loopback
                    if ip.is_private() || ip.is_link_local() {
                        Some(ip)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_progress_ratio() {
        let mut task = TaskProgress::new("Test");
        assert_eq!(task.ratio(), 0.0);

        task.start(10);
        assert_eq!(task.ratio(), 0.0);

        task.advance();
        task.advance();
        assert!((task.ratio() - 0.2).abs() < f64::EPSILON);

        task.finish();
        assert_eq!(task.ratio(), 1.0);
    }
}
