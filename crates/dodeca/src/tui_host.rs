//! TUI host state for dodeca
//!
//! This module manages TUI state and command forwarding. The host pushes
//! updates to the TUI cell via TuiDisplayClient; the TUI cell sends commands
//! back via HostService::send_command() which gets forwarded here.

use cell_tui_proto::{
    BindMode, BuildProgress, CommandResult, EventKind, LogEvent, LogLevel, ServerCommand,
    ServerStatus, TaskProgress, TaskStatus,
};
use tokio::sync::mpsc;

/// TUI command forwarder
///
/// Receives commands from the TUI cell (via HostService::send_command) and
/// forwards them to the main server loop.
#[derive(Clone)]
pub struct TuiHostImpl {
    /// Channel to send commands back to the main server loop
    command_tx: mpsc::UnboundedSender<ServerCommand>,
}

impl TuiHostImpl {
    /// Create a new TuiHostImpl
    pub fn new(command_tx: mpsc::UnboundedSender<ServerCommand>) -> Self {
        Self { command_tx }
    }

    /// Handle a command from the TUI cell
    pub fn handle_command(&self, command: ServerCommand) -> CommandResult {
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
        picante_cache_size: status.picante_cache_size as u64,
        cas_cache_size: status.cas_cache_size as u64,
        code_exec_cache_size: status.code_exec_cache_size as u64,
    }
}

/// Convert from proto ServerCommand to old tui::ServerCommand
/// Note: CycleLogLevel, TogglePicanteDebug, and SetLogFilter are handled directly in main.rs bridge
pub fn convert_server_command(cmd: ServerCommand) -> crate::tui::ServerCommand {
    match cmd {
        ServerCommand::GoPublic => crate::tui::ServerCommand::GoPublic,
        ServerCommand::GoLocal => crate::tui::ServerCommand::GoLocal,
        // These are handled directly in the bridge, should never reach here
        ServerCommand::TogglePicanteDebug
        | ServerCommand::CycleLogLevel
        | ServerCommand::SetLogFilter { .. } => {
            unreachable!(
                "TogglePicanteDebug, CycleLogLevel, and SetLogFilter are handled directly in main.rs"
            )
        }
    }
}
