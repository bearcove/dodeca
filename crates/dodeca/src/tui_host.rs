//! TuiHost service implementation for dodeca
//!
//! This module implements the TuiHost trait from cell-tui-proto, allowing
//! the TUI cell to connect and receive streaming updates.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use cell_tui_proto::{
    BindMode, BuildProgress, CommandResult, EventKind, LogEvent, LogLevel, ServerCommand,
    ServerStatus, TaskProgress, TaskStatus, TuiHost,
};
use futures_util::StreamExt;
use roam::Tx;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_stream::wrappers::{BroadcastStream, WatchStream, errors::BroadcastStreamRecvError};

/// Capacity for broadcast channels
const BROADCAST_CAPACITY: usize = 256;

/// Number of recent events to retain for late subscribers.
const EVENT_HISTORY_CAPACITY: usize = 512;

/// TuiHost implementation that broadcasts updates to connected TUI clients
#[derive(Clone)]
pub struct TuiHostImpl {
    /// Sender for progress updates (latest-value semantics)
    progress_tx: watch::Sender<BuildProgress>,
    /// Sender for log events
    events_tx: broadcast::Sender<LogEvent>,
    /// Recent log events for late subscribers
    events_history: Arc<Mutex<VecDeque<LogEvent>>>,
    /// Sender for server status updates (latest-value semantics)
    status_tx: watch::Sender<ServerStatus>,
    /// Channel to send commands back to the main server loop
    command_tx: mpsc::UnboundedSender<ServerCommand>,
}

#[allow(dead_code)]
impl TuiHostImpl {
    /// Create a new TuiHost implementation
    pub fn new(command_tx: mpsc::UnboundedSender<ServerCommand>) -> Self {
        let (progress_tx, _) = watch::channel(BuildProgress::default());
        let (events_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let events_history = Arc::new(Mutex::new(VecDeque::with_capacity(EVENT_HISTORY_CAPACITY)));
        let (status_tx, _) = watch::channel(ServerStatus::default());

        Self {
            progress_tx,
            events_tx,
            events_history,
            status_tx,
            command_tx,
        }
    }

    /// Send a progress update to all connected TUI clients
    pub fn send_progress(&self, progress: BuildProgress) {
        let _ = self.progress_tx.send_replace(progress);
    }

    /// Send a log event to all connected TUI clients
    pub fn send_event(&self, event: LogEvent) {
        record_event(&self.events_history, event.clone());
        let _ = self.events_tx.send(event);
    }

    /// Send a server status update to all connected TUI clients
    pub fn send_status(&self, status: ServerStatus) {
        let _ = self.status_tx.send_replace(status);
    }

    /// Get a handle for sending updates (can be cloned and passed around)
    pub fn handle(&self) -> TuiHostHandle {
        TuiHostHandle {
            progress_tx: self.progress_tx.clone(),
            events_tx: self.events_tx.clone(),
            events_history: self.events_history.clone(),
            status_tx: self.status_tx.clone(),
        }
    }
}

/// A handle for sending TUI updates (can be cloned)
#[derive(Clone)]
pub struct TuiHostHandle {
    progress_tx: watch::Sender<BuildProgress>,
    events_tx: broadcast::Sender<LogEvent>,
    events_history: Arc<Mutex<VecDeque<LogEvent>>>,
    status_tx: watch::Sender<ServerStatus>,
}

impl TuiHostHandle {
    /// Send a progress update
    pub fn send_progress(&self, progress: BuildProgress) {
        let _ = self.progress_tx.send_replace(progress);
    }

    /// Send a log event
    pub fn send_event(&self, event: LogEvent) {
        record_event(&self.events_history, event.clone());
        let _ = self.events_tx.send(event);
    }

    /// Send a server status update
    pub fn send_status(&self, status: ServerStatus) {
        let _ = self.status_tx.send_replace(status);
    }
}

impl TuiHost for TuiHostImpl {
    async fn subscribe_progress(&self, tx: Tx<BuildProgress>) {
        let rx = self.progress_tx.subscribe();
        tokio::spawn(async move {
            let mut stream = WatchStream::new(rx);
            while let Some(progress) = stream.next().await {
                if tx.send(&progress).await.is_err() {
                    break;
                }
            }
        });
    }

    async fn subscribe_events(&self, tx: Tx<LogEvent>) {
        let history = {
            let history = self.events_history.lock().unwrap();
            history.iter().cloned().collect::<Vec<_>>()
        };
        let events_rx = self.events_tx.subscribe();

        tokio::spawn(async move {
            // First replay history
            for event in history {
                if tx.send(&event).await.is_err() {
                    return;
                }
            }
            // Then stream live events
            let mut stream = BroadcastStream::new(events_rx);
            while let Some(result) = stream.next().await {
                match result {
                    Ok(event) => {
                        if tx.send(&event).await.is_err() {
                            break;
                        }
                    }
                    Err(BroadcastStreamRecvError::Lagged(_)) => continue,
                }
            }
        });
    }

    async fn subscribe_server_status(&self, tx: Tx<ServerStatus>) {
        let rx = self.status_tx.subscribe();
        tokio::spawn(async move {
            let mut stream = WatchStream::new(rx);
            while let Some(status) = stream.next().await {
                if tx.send(&status).await.is_err() {
                    break;
                }
            }
        });
    }

    async fn send_command(&self, command: ServerCommand) -> CommandResult {
        let command_tx = self.command_tx.clone();
        match command_tx.send(command) {
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

fn record_event(history: &Arc<Mutex<VecDeque<LogEvent>>>, event: LogEvent) {
    let mut history = history.lock().unwrap();
    history.push_back(event);
    while history.len() > EVENT_HISTORY_CAPACITY {
        history.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn progress_and_status_are_retained_without_subscribers() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel::<ServerCommand>();
        let host = TuiHostImpl::new(cmd_tx);

        let mut progress = BuildProgress::default();
        progress.parse.name = "parse".to_string();
        progress.parse.total = 10;
        progress.parse.completed = 3;
        host.send_progress(progress.clone());

        let status = ServerStatus {
            urls: vec!["http://127.0.0.1:4000".to_string()],
            is_running: true,
            bind_mode: BindMode::Local,
            picante_cache_size: 1,
            cas_cache_size: 2,
            code_exec_cache_size: 3,
        };
        host.send_status(status.clone());

        // Subscribe and verify we get the retained values
        let (progress_tx, mut progress_rx) = roam::channel::<BuildProgress>();
        let (status_tx, mut status_rx) = roam::channel::<ServerStatus>();

        host.subscribe_progress(progress_tx).await;
        host.subscribe_server_status(status_tx).await;

        // Give the spawned tasks time to send
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let first_progress = progress_rx.recv().await.unwrap().unwrap();
        let first_status = status_rx.recv().await.unwrap().unwrap();

        assert_eq!(first_progress.parse.name, progress.parse.name);
        assert_eq!(first_progress.parse.total, progress.parse.total);
        assert_eq!(first_progress.parse.completed, progress.parse.completed);

        assert_eq!(first_status.urls, status.urls);
        assert_eq!(first_status.is_running, status.is_running);
        assert_eq!(first_status.bind_mode, status.bind_mode);
        assert_eq!(first_status.picante_cache_size, status.picante_cache_size);
        assert_eq!(first_status.cas_cache_size, status.cas_cache_size);
        assert_eq!(
            first_status.code_exec_cache_size,
            status.code_exec_cache_size
        );
    }

    #[tokio::test]
    async fn events_are_replayed_for_late_subscribers() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel::<ServerCommand>();
        let host = TuiHostImpl::new(cmd_tx);

        host.send_event(LogEvent {
            level: LogLevel::Info,
            kind: EventKind::Server,
            message: "first".to_string(),
        });
        host.send_event(LogEvent {
            level: LogLevel::Info,
            kind: EventKind::Server,
            message: "second".to_string(),
        });

        let (events_tx, mut events_rx) = roam::channel::<LogEvent>();
        host.subscribe_events(events_tx).await;

        // Give the spawned task time to send history
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let a = events_rx.recv().await.unwrap().unwrap();
        let b = events_rx.recv().await.unwrap().unwrap();
        assert_eq!(a.message, "first");
        assert_eq!(b.message, "second");
    }
}
