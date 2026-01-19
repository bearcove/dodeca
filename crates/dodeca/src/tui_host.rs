//! TUI type conversion helpers.
//!
//! This module provides conversion functions between the old tui types
//! (used internally) and the proto types (used for RPC).
//!
//! Note: TUI command forwarding now goes through Host::get().handle_tui_command().

use cell_tui_proto::{
    BindMode, BuildProgress, EventKind, LogEvent, LogLevel, ServerCommand, ServerStatus,
    TaskProgress, TaskStatus,
};

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
        total: task.total,
        completed: task.completed,
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
        fields: event.fields.clone(),
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
