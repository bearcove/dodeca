//! Protocol definitions for the dodeca TUI cell.
//!
//! Bidirectional RPC:
//! - Host calls TuiDisplay on the TUI cell to push updates
//! - TUI cell calls HostService::send_command() to send commands to host

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

    pub fn start(&mut self, total: usize) {
        self.total = total as u32;
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
    /// Picante/build related
    Build,
    /// Generic info
    #[default]
    Generic,
}

/// A log event with level, kind, message, and structured fields
#[derive(Debug, Clone, facet::Facet)]
pub struct LogEvent {
    pub level: LogLevel,
    pub kind: EventKind,
    pub message: String,
    /// Structured key-value fields for colored display
    pub fields: Vec<(String, String)>,
}

impl LogEvent {
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Generic,
            message: message.into(),
            fields: vec![],
        }
    }

    pub fn warn(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Warn,
            kind: EventKind::Generic,
            message: message.into(),
            fields: vec![],
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Error,
            kind: EventKind::Generic,
            message: message.into(),
            fields: vec![],
        }
    }

    pub fn with_kind(mut self, kind: EventKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_fields(mut self, fields: Vec<(String, String)>) -> Self {
        self.fields = fields;
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
            fields: vec![],
        }
    }

    pub fn file_change(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::FileChange,
            message: message.into(),
            fields: vec![],
        }
    }

    pub fn reload(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Reload,
            message: message.into(),
            fields: vec![],
        }
    }

    pub fn patch(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Patch,
            message: message.into(),
            fields: vec![],
        }
    }

    pub fn search(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Search,
            message: message.into(),
            fields: vec![],
        }
    }

    pub fn server(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Server,
            message: message.into(),
            fields: vec![],
        }
    }

    pub fn build(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            kind: EventKind::Build,
            message: message.into(),
            fields: vec![],
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
    /// Picante cache size in bytes
    pub picante_cache_size: u64,
    /// CAS/image cache size in bytes
    pub cas_cache_size: u64,
    /// Code execution cache size in bytes
    pub code_exec_cache_size: u64,
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
    /// Toggle picante debug logging
    TogglePicanteDebug,
    /// Cycle log level
    CycleLogLevel,
    /// Set a custom log filter expression (RUST_LOG style)
    SetLogFilter { filter: String },
}

/// Result of a command
#[derive(Debug, Clone, facet::Facet)]
#[repr(u8)]
pub enum CommandResult {
    Ok,
    Error { message: String },
}

// ============================================================================
// TuiDisplay Service (hosted by TUI cell, called by host)
// ============================================================================

/// Service hosted by the TUI cell for the host to push updates.
///
/// The host calls these methods to update the TUI display.
/// For commands from TUI → host, use HostService::send_command().
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait TuiDisplay {
    /// Host pushes a progress update.
    async fn update_progress(&self, progress: BuildProgress);

    /// Host pushes a log event.
    async fn push_event(&self, event: LogEvent);

    /// Host pushes a server status update.
    async fn update_status(&self, status: ServerStatus);
}
