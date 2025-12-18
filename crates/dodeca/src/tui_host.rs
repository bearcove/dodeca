//! TuiHost service implementation for dodeca
//!
//! This module implements the TuiHost trait from cell-tui-proto, allowing
//! the TUI cell to connect and receive streaming updates.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use cell_tui_proto::{
    BindMode, BuildProgress, CommandResult, EventKind, LogEvent, LogLevel, ServerCommand,
    ServerStatus, TaskProgress, TaskStatus, TuiHost, TuiHostServer,
};
use eyre::Result;
use futures::StreamExt;
use rapace::RpcSession;
use rapace::Streaming;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_stream::wrappers::{BroadcastStream, WatchStream, errors::BroadcastStreamRecvError};

/// Capacity for broadcast channels
const BROADCAST_CAPACITY: usize = 256;

/// Number of recent events to retain for late subscribers.
const EVENT_HISTORY_CAPACITY: usize = 512;

/// TuiHost implementation that broadcasts updates to connected TUI clients
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
        // Use `send_replace` so the latest value is retained even if there are
        // currently no connected TUI clients.
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

    /// Create the rapace server wrapper
    pub fn into_server(self) -> TuiHostServer<Self> {
        TuiHostServer::new(self)
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
    async fn subscribe_progress(&self) -> Streaming<BuildProgress> {
        let rx = self.progress_tx.subscribe();
        let stream = WatchStream::new(rx).map(Ok);
        Box::pin(stream) as Streaming<BuildProgress>
    }

    async fn subscribe_events(&self) -> Streaming<LogEvent> {
        let history = {
            let history = self.events_history.lock().unwrap();
            history.iter().cloned().collect::<Vec<_>>()
        };
        let rx = self.events_tx.subscribe();
        let live = BroadcastStream::new(rx).filter_map(|result| async move {
            match result {
                Ok(item) => Some(Ok(item)),
                Err(BroadcastStreamRecvError::Lagged(_)) => None,
            }
        });
        let stream = futures::stream::iter(history.into_iter().map(Ok)).chain(live);
        Box::pin(stream) as Streaming<LogEvent>
    }

    async fn subscribe_server_status(&self) -> Streaming<ServerStatus> {
        let rx = self.status_tx.subscribe();
        let stream = WatchStream::new(rx).map(Ok);
        Box::pin(stream) as Streaming<ServerStatus>
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
        salsa_cache_size: status.salsa_cache_size as u64,
        cas_cache_size: status.cas_cache_size as u64,
    }
}

/// Convert from proto ServerCommand to old tui::ServerCommand
/// Note: CycleLogLevel and ToggleSalsaDebug are handled directly in main.rs bridge
pub fn convert_server_command(cmd: ServerCommand) -> crate::tui::ServerCommand {
    match cmd {
        ServerCommand::GoPublic => crate::tui::ServerCommand::GoPublic,
        ServerCommand::GoLocal => crate::tui::ServerCommand::GoLocal,
        // These are handled directly in the bridge, should never reach here
        ServerCommand::ToggleSalsaDebug | ServerCommand::CycleLogLevel => {
            unreachable!("ToggleSalsaDebug and CycleLogLevel are handled directly in main.rs")
        }
    }
}

// ============================================================================
// TUI Cell Spawning
// ============================================================================

/// Wrapper struct that implements TuiHost by delegating to `Arc<TuiHostImpl>`
struct TuiHostWrapper(Arc<TuiHostImpl>);

impl TuiHost for TuiHostWrapper {
    async fn subscribe_progress(&self) -> Streaming<BuildProgress> {
        self.0.subscribe_progress().await
    }

    async fn subscribe_events(&self) -> Streaming<LogEvent> {
        self.0.subscribe_events().await
    }

    async fn subscribe_server_status(&self) -> Streaming<ServerStatus> {
        self.0.subscribe_server_status().await
    }

    async fn send_command(&self, command: ServerCommand) -> CommandResult {
        self.0.send_command(command).await
    }
}

impl Clone for TuiHostWrapper {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Start the TUI cell and run the host service
///
/// This spawns the TUI cell through the hub (like all other cells) and sets up
/// the TuiHost RPC dispatcher so the cell can subscribe to updates.
///
/// The `shutdown_rx` allows external signaling to stop the TUI.
pub async fn start_tui_cell(
    tui_host: TuiHostImpl,
    mut shutdown_rx: Option<watch::Receiver<bool>>,
) -> Result<()> {
    let tui_host_arc = Arc::new(tui_host);
    let dispatcher_factory = move |session: Arc<RpcSession>| {
        let wrapper = TuiHostWrapper(tui_host_arc.clone());
        TuiHostServer::new(wrapper).into_session_dispatcher(session.transport().clone())
    };

    let (_rpc_session, mut child) = crate::cells::spawn_cell_with_dispatcher(
        "ddc-cell-tui",
        dispatcher_factory,
    )
    .await
    .ok_or_else(|| {
        eyre::eyre!(
            "Failed to spawn TUI cell. Make sure ddc-cell-tui is built and in the cell path."
        )
    })?;

    // Wait for TUI process to exit or shutdown signal
    #[allow(clippy::never_loop)] // Loop is for select! - exits on either branch
    loop {
        tokio::select! {
            status = child.wait() => {
                match status {
                    Ok(s) => tracing::debug!("TUI cell exited with status: {}", s),
                    Err(e) => tracing::error!("TUI cell wait error: {:?}", e),
                }
                break;
            }
            _ = async {
                if let Some(ref mut rx) = shutdown_rx {
                    rx.changed().await.ok();
                    if *rx.borrow() {
                        return;
                    }
                }
                // Never complete if no shutdown receiver
                std::future::pending::<()>().await
            } => {
                tracing::info!("Shutdown signal received, stopping TUI cell");
                let _ = child.kill().await;
                break;
            }
        }
    }

    Ok(())
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
    use futures::StreamExt;

    #[expect(
        clippy::disallowed_methods,
        reason = "tokio::test uses block_on internally"
    )]
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
            salsa_cache_size: 1,
            cas_cache_size: 2,
        };
        host.send_status(status.clone());

        let mut progress_stream = host.subscribe_progress().await;
        let mut status_stream = host.subscribe_server_status().await;

        let first_progress = progress_stream.next().await.unwrap().unwrap();
        let first_status = status_stream.next().await.unwrap().unwrap();

        assert_eq!(first_progress.parse.name, progress.parse.name);
        assert_eq!(first_progress.parse.total, progress.parse.total);
        assert_eq!(first_progress.parse.completed, progress.parse.completed);

        assert_eq!(first_status.urls, status.urls);
        assert_eq!(first_status.is_running, status.is_running);
        assert_eq!(first_status.bind_mode, status.bind_mode);
        assert_eq!(first_status.salsa_cache_size, status.salsa_cache_size);
        assert_eq!(first_status.cas_cache_size, status.cas_cache_size);
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "tokio::test uses block_on internally"
    )]
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

        let mut stream = host.subscribe_events().await;
        let a = stream.next().await.unwrap().unwrap();
        let b = stream.next().await.unwrap().unwrap();
        assert_eq!(a.message, "first");
        assert_eq!(b.message, "second");
    }
}
