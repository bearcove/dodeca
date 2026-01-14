//! TUI types for dodeca build progress
//!
//! These types are used for channel communication with the TUI cell.
//! The actual TUI rendering is done by the mod-tui cell.

use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// Progress state for a single task
#[derive(Debug, Clone)]
pub struct TaskProgress {
    pub name: &'static str,
    pub total: usize,
    pub completed: usize,
    pub status: TaskStatus,
    pub message: Option<String>,
}

/// Status of a build task
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskStatus {
    #[default]
    Pending,
    Running,
    Done,
    Error,
}

impl TaskProgress {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            total: 0,
            completed: 0,
            status: TaskStatus::Pending,
            message: None,
        }
    }

    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.completed as f64 / self.total as f64
        }
    }

    pub fn start(&mut self, total: usize) {
        self.total = total;
        self.completed = 0;
        self.status = TaskStatus::Running;
    }

    pub fn advance(&mut self) {
        self.completed = (self.completed + 1).min(self.total);
    }

    pub fn finish(&mut self) {
        self.completed = self.total;
        self.status = TaskStatus::Done;
    }

    pub fn fail(&mut self, msg: impl Into<String>) {
        self.status = TaskStatus::Error;
        self.message = Some(msg.into());
    }
}

/// All build progress state
#[derive(Debug, Clone)]
pub struct BuildProgress {
    pub parse: TaskProgress,
    pub render: TaskProgress,
    pub sass: TaskProgress,
    pub links: TaskProgress,
    pub search: TaskProgress,
}

impl Default for BuildProgress {
    fn default() -> Self {
        Self {
            parse: TaskProgress::new("Parsing"),
            render: TaskProgress::new("Rendering"),
            sass: TaskProgress::new("Sass"),
            links: TaskProgress::new("Links"),
            search: TaskProgress::new("Search"),
        }
    }
}

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

/// Log level for activity events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

/// Kind of activity event (for display styling)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventKind {
    /// HTTP request (GET, POST, etc.)
    Http { status: u16 },
    /// File system change
    FileChange,
    /// Live reload triggered
    Reload,
    /// DOM patches sent
    Patch,
    /// Search index update
    Search,
    /// Server status
    Server,
    /// Picante/build related
    Build,
    /// Generic info
    #[default]
    Generic,
}

/// A log event with level, kind, and message
#[derive(Debug, Clone)]
pub struct LogEvent {
    pub level: LogLevel,
    pub kind: EventKind,
    pub message: String,
}

impl LogEvent {
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Generic,
            message: message.into(),
        }
    }

    pub fn warn(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Warn,
            kind: EventKind::Generic,
            message: message.into(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Error,
            kind: EventKind::Generic,
            message: message.into(),
        }
    }

    // Convenience constructors for specific event types
    fn _http(status: u16, message: impl Into<String>) -> Self {
        Self {
            level: if status >= 400 {
                LogLevel::Warn
            } else {
                LogLevel::Info
            },
            kind: EventKind::Http { status },
            message: message.into(),
        }
    }

    pub fn file_change(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::FileChange,
            message: message.into(),
        }
    }

    fn _reload(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Reload,
            message: message.into(),
        }
    }

    fn _patch(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Patch,
            message: message.into(),
        }
    }

    pub fn server(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Server,
            message: message.into(),
        }
    }

    pub fn build(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Build,
            message: message.into(),
        }
    }
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

/// Server binding mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BindMode {
    /// Local only (127.0.0.1)
    #[default]
    Local,
    /// LAN interfaces (private IPs)
    Lan,
}

/// Server status for serve mode TUI
#[derive(Debug, Clone, Default)]
pub struct ServerStatus {
    pub urls: Vec<String>,
    pub is_running: bool,
    pub bind_mode: BindMode,
    /// Picante cache size in bytes (dodeca.bin)
    pub picante_cache_size: usize,
    /// CAS/image cache size in bytes (.cache directory)
    pub cas_cache_size: usize,
    /// Code execution cache size in bytes (.cache/code-execution)
    pub code_exec_cache_size: usize,
}

/// Command sent from TUI to server
#[derive(Debug, Clone)]
pub enum ServerCommand {
    /// Switch to LAN mode (bind to 0.0.0.0)
    GoPublic,
    /// Switch to local mode (bind to 127.0.0.1)
    GoLocal,
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
