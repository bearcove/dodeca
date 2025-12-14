//! Protocol definitions for the dodeca TUI plugin.
//!
//! The TUI is a "reverse plugin" - dodeca hosts the `TuiHost` service,
//! and the TUI binary connects to subscribe to streams and send commands.

use rapace::Streaming;

// ============================================================================
// Build Progress Types
// ============================================================================

/// Status of a build task
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, facet::Facet)]
#[repr(u8)]
pub enum TaskStatus {
    #[default]
    Pending,
    Running,
    Done,
    Error,
}

/// Progress state for a single task
#[derive(Debug, Clone, facet::Facet)]
pub struct TaskProgress {
    pub name: String,
    pub total: u32,
    pub completed: u32,
    pub status: TaskStatus,
    pub message: Option<String>,
}

impl TaskProgress {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
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
}

/// All build progress state
#[derive(Debug, Clone, Default, facet::Facet)]
pub struct BuildProgress {
    pub parse: TaskProgress,
    pub render: TaskProgress,
    pub sass: TaskProgress,
    pub links: TaskProgress,
    pub search: TaskProgress,
}

impl Default for TaskProgress {
    fn default() -> Self {
        Self::new("Unknown")
    }
}

// ============================================================================
// Log Event Types
// ============================================================================

/// Log level for activity events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, facet::Facet)]
#[repr(u8)]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

/// Kind of activity event (for display styling)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, facet::Facet)]
#[repr(u8)]
pub enum EventKind {
    /// HTTP request with status code
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
    /// Salsa/build related
    Build,
    /// Generic info
    #[default]
    Generic,
}

/// A log event with level, kind, and message
#[derive(Debug, Clone, facet::Facet)]
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

    pub fn with_kind(mut self, kind: EventKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn http(status: u16, message: impl Into<String>) -> Self {
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

    pub fn reload(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Reload,
            message: message.into(),
        }
    }

    pub fn patch(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Patch,
            message: message.into(),
        }
    }

    pub fn search(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Search,
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

// ============================================================================
// Server Status Types
// ============================================================================

/// Server binding mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, facet::Facet)]
#[repr(u8)]
pub enum BindMode {
    /// Local only (127.0.0.1)
    #[default]
    Local,
    /// LAN interfaces (private IPs)
    Lan,
}

/// Server status for serve mode TUI
#[derive(Debug, Clone, Default, facet::Facet)]
pub struct ServerStatus {
    pub urls: Vec<String>,
    pub is_running: bool,
    pub bind_mode: BindMode,
    /// Salsa cache size in bytes
    pub salsa_cache_size: u64,
    /// CAS/image cache size in bytes
    pub cas_cache_size: u64,
}

// ============================================================================
// Commands (TUI -> Host)
// ============================================================================

/// Command sent from TUI to server
#[derive(Debug, Clone, facet::Facet)]
#[repr(u8)]
pub enum ServerCommand {
    /// Switch to LAN mode (bind to 0.0.0.0)
    GoPublic,
    /// Switch to local mode (bind to 127.0.0.1)
    GoLocal,
    /// Toggle salsa debug logging
    ToggleSalsaDebug,
    /// Cycle log level
    CycleLogLevel,
}

/// Result of a command
#[derive(Debug, Clone, facet::Facet)]
#[repr(u8)]
pub enum CommandResult {
    Ok,
    Error { message: String },
}

// ============================================================================
// TuiHost Service (hosted by dodeca, called by TUI plugin)
// ============================================================================

/// Service hosted by dodeca for the TUI to connect to.
///
/// The TUI subscribes to streams for live updates and sends commands back.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait TuiHost {
    /// Subscribe to build progress updates.
    /// Stream emits whenever progress changes.
    async fn subscribe_progress(&self) -> Streaming<BuildProgress>;

    /// Subscribe to log events.
    /// Stream emits for each new event (HTTP requests, file changes, etc.)
    async fn subscribe_events(&self) -> Streaming<LogEvent>;

    /// Subscribe to server status updates.
    /// Stream emits when URLs, bind mode, or cache sizes change.
    async fn subscribe_server_status(&self) -> Streaming<ServerStatus>;

    /// Send a command to the server.
    async fn send_command(&self, command: ServerCommand) -> CommandResult;
}
