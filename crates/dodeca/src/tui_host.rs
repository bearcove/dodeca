//! TuiHost service implementation for dodeca
//!
//! This module implements the TuiHost trait from mod-tui-proto, allowing
//! the TUI plugin to connect and receive streaming updates.

use futures::StreamExt;
use mod_tui_proto::{
    BindMode, BuildProgress, CommandResult, EventKind, LogEvent, LogLevel, ServerCommand,
    ServerStatus, TaskProgress, TaskStatus, TuiHost, TuiHostServer,
};
use rapace::Streaming;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::{BroadcastStream, errors::BroadcastStreamRecvError};

/// Capacity for broadcast channels
const BROADCAST_CAPACITY: usize = 256;

/// TuiHost implementation that broadcasts updates to connected TUI clients
pub struct TuiHostImpl {
    /// Sender for progress updates
    progress_tx: broadcast::Sender<BuildProgress>,
    /// Sender for log events
    events_tx: broadcast::Sender<LogEvent>,
    /// Sender for server status updates
    status_tx: broadcast::Sender<ServerStatus>,
    /// Channel to send commands back to the main server loop
    command_tx: mpsc::UnboundedSender<ServerCommand>,
}

impl TuiHostImpl {
    /// Create a new TuiHost implementation
    pub fn new(command_tx: mpsc::UnboundedSender<ServerCommand>) -> Self {
        let (progress_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (events_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (status_tx, _) = broadcast::channel(BROADCAST_CAPACITY);

        Self {
            progress_tx,
            events_tx,
            status_tx,
            command_tx,
        }
    }

    /// Send a progress update to all connected TUI clients
    pub fn send_progress(&self, progress: BuildProgress) {
        let _ = self.progress_tx.send(progress);
    }

    /// Send a log event to all connected TUI clients
    pub fn send_event(&self, event: LogEvent) {
        let _ = self.events_tx.send(event);
    }

    /// Send a server status update to all connected TUI clients
    pub fn send_status(&self, status: ServerStatus) {
        let _ = self.status_tx.send(status);
    }

    /// Get a handle for sending updates (can be cloned and passed around)
    pub fn handle(&self) -> TuiHostHandle {
        TuiHostHandle {
            progress_tx: self.progress_tx.clone(),
            events_tx: self.events_tx.clone(),
            status_tx: self.status_tx.clone(),
        }
    }

    /// Create the rapace server wrapper
    pub fn into_server(self) -> TuiHostServer<Self> {
        TuiHostServer::new(self)
    }
}

/// A handle for sending TUI updates (can be cloned)
#[derive(Clone)]
pub struct TuiHostHandle {
    progress_tx: broadcast::Sender<BuildProgress>,
    events_tx: broadcast::Sender<LogEvent>,
    status_tx: broadcast::Sender<ServerStatus>,
}

impl TuiHostHandle {
    /// Send a progress update
    pub fn send_progress(&self, progress: BuildProgress) {
        let _ = self.progress_tx.send(progress);
    }

    /// Send a log event
    pub fn send_event(&self, event: LogEvent) {
        let _ = self.events_tx.send(event);
    }

    /// Send a server status update
    pub fn send_status(&self, status: ServerStatus) {
        let _ = self.status_tx.send(status);
    }
}

impl TuiHost for TuiHostImpl {
    async fn subscribe_progress(&self) -> Streaming<BuildProgress> {
        let rx = self.progress_tx.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|result| async move {
            match result {
                Ok(item) => Some(Ok(item)),
                Err(BroadcastStreamRecvError::Lagged(_)) => None, // Skip lagged messages
            }
        });
        Box::pin(stream)
    }

    async fn subscribe_events(&self) -> Streaming<LogEvent> {
        let rx = self.events_tx.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|result| async move {
            match result {
                Ok(item) => Some(Ok(item)),
                Err(BroadcastStreamRecvError::Lagged(_)) => None,
            }
        });
        Box::pin(stream)
    }

    async fn subscribe_server_status(&self) -> Streaming<ServerStatus> {
        let rx = self.status_tx.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|result| async move {
            match result {
                Ok(item) => Some(Ok(item)),
                Err(BroadcastStreamRecvError::Lagged(_)) => None,
            }
        });
        Box::pin(stream)
    }

    async fn send_command(&self, command: ServerCommand) -> CommandResult {
        match self.command_tx.send(command) {
            Ok(_) => CommandResult::Ok,
            Err(e) => CommandResult::Error {
                message: format!("Failed to send command: {}", e),
            },
        }
    }
}

// ============================================================================
// Conversion helpers from old tui types to proto types
// ============================================================================

/// Convert from the old tui::TaskStatus to proto TaskStatus
pub fn convert_task_status(status: crate::tui::TaskStatus) -> TaskStatus {
    match status {
        crate::tui::TaskStatus::Pending => TaskStatus::Pending,
        crate::tui::TaskStatus::Running => TaskStatus::Running,
        crate::tui::TaskStatus::Done => TaskStatus::Done,
        crate::tui::TaskStatus::Error => TaskStatus::Error,
    }
}

/// Convert from old tui::TaskProgress to proto TaskProgress
pub fn convert_task_progress(task: &crate::tui::TaskProgress) -> TaskProgress {
    TaskProgress {
        name: task.name.to_string(),
        total: task.total as u32,
        completed: task.completed as u32,
        status: convert_task_status(task.status),
        message: task.message.clone(),
    }
}

/// Convert from old tui::BuildProgress to proto BuildProgress
pub fn convert_build_progress(progress: &crate::tui::BuildProgress) -> BuildProgress {
    BuildProgress {
        parse: convert_task_progress(&progress.parse),
        render: convert_task_progress(&progress.render),
        sass: convert_task_progress(&progress.sass),
        links: convert_task_progress(&progress.links),
        search: convert_task_progress(&progress.search),
    }
}

/// Convert from old tui::LogLevel to proto LogLevel
pub fn convert_log_level(level: crate::tui::LogLevel) -> LogLevel {
    match level {
        crate::tui::LogLevel::Trace => LogLevel::Trace,
        crate::tui::LogLevel::Debug => LogLevel::Debug,
        crate::tui::LogLevel::Info => LogLevel::Info,
        crate::tui::LogLevel::Warn => LogLevel::Warn,
        crate::tui::LogLevel::Error => LogLevel::Error,
    }
}

/// Convert from old tui::EventKind to proto EventKind
pub fn convert_event_kind(kind: crate::tui::EventKind) -> EventKind {
    match kind {
        crate::tui::EventKind::Http { status } => EventKind::Http { status },
        crate::tui::EventKind::FileChange => EventKind::FileChange,
        crate::tui::EventKind::Reload => EventKind::Reload,
        crate::tui::EventKind::Patch => EventKind::Patch,
        crate::tui::EventKind::Search => EventKind::Search,
        crate::tui::EventKind::Server => EventKind::Server,
        crate::tui::EventKind::Build => EventKind::Build,
        crate::tui::EventKind::Generic => EventKind::Generic,
    }
}

/// Convert from old tui::LogEvent to proto LogEvent
pub fn convert_log_event(event: &crate::tui::LogEvent) -> LogEvent {
    LogEvent {
        level: convert_log_level(event.level),
        kind: convert_event_kind(event.kind),
        message: event.message.clone(),
    }
}

/// Convert from old tui::BindMode to proto BindMode
pub fn convert_bind_mode(mode: crate::tui::BindMode) -> BindMode {
    match mode {
        crate::tui::BindMode::Local => BindMode::Local,
        crate::tui::BindMode::Lan => BindMode::Lan,
    }
}

/// Convert from old tui::ServerStatus to proto ServerStatus
pub fn convert_server_status(status: &crate::tui::ServerStatus) -> ServerStatus {
    ServerStatus {
        urls: status.urls.clone(),
        is_running: status.is_running,
        bind_mode: convert_bind_mode(status.bind_mode),
        salsa_cache_size: status.salsa_cache_size as u64,
        cas_cache_size: status.cas_cache_size as u64,
    }
}

/// Convert from proto ServerCommand to old tui::ServerCommand
pub fn convert_server_command(cmd: ServerCommand) -> crate::tui::ServerCommand {
    match cmd {
        ServerCommand::GoPublic => crate::tui::ServerCommand::GoPublic,
        ServerCommand::GoLocal => crate::tui::ServerCommand::GoLocal,
        // New commands - need to extend the old enum or handle differently
        ServerCommand::ToggleSalsaDebug | ServerCommand::CycleLogLevel => {
            // For now, just ignore these as the old TUI handled them internally
            crate::tui::ServerCommand::GoLocal // placeholder
        }
    }
}
