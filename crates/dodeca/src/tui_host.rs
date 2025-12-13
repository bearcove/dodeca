//! TuiHost service implementation for dodeca
//!
//! This module implements the TuiHost trait from mod-tui-proto, allowing
//! the TUI plugin to connect and receive streaming updates.

use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;

use eyre::Result;
use futures::StreamExt;
use mod_tui_proto::{
    BindMode, BuildProgress, CommandResult, EventKind, LogEvent, LogLevel, ServerCommand,
    ServerStatus, TaskProgress, TaskStatus, TuiHost, TuiHostServer,
};
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace::{Frame, RpcError, RpcSession};
use rapace::Streaming;
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, watch};
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

#[allow(dead_code)]
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

// ============================================================================
// TUI Plugin Spawning
// ============================================================================

/// Type alias for our transport (SHM-based for zero-copy)
type TuiTransport = ShmTransport;

/// SHM configuration for TUI communication
/// This must match the SHM_CONFIG in dodeca-plugin-runtime for plugins to connect.
const SHM_CONFIG: ShmSessionConfig = ShmSessionConfig {
    ring_capacity: 1024, // 1024 descriptors in flight
    slot_size: 65536,    // 64KB per slot
    slot_count: 512,     // 512 slots = 32MB total
};

/// Find the TUI plugin binary path (next to the main executable)
pub fn find_tui_plugin_path() -> Result<PathBuf> {
    let exe_path = std::env::current_exe()?;
    let plugin_path = exe_path
        .parent()
        .ok_or_else(|| eyre::eyre!("Cannot find parent directory of executable"))?
        .join("dodeca-mod-tui");

    if !plugin_path.exists() {
        return Err(eyre::eyre!(
            "TUI plugin binary not found at {}. Build it with: cargo build -p mod-tui --bin dodeca-mod-tui",
            plugin_path.display()
        ));
    }

    Ok(plugin_path)
}

/// Wrapper struct that implements TuiHost by delegating to Arc<TuiHostImpl>
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

/// Create a dispatcher for the TuiHost service
#[allow(clippy::type_complexity)]
pub fn create_tui_dispatcher(
    tui_host: Arc<TuiHostImpl>,
) -> impl Fn(
    u32,
    u32,
    Vec<u8>,
) -> Pin<Box<dyn std::future::Future<Output = Result<Frame, RpcError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_channel_id, method_id, payload| {
        let wrapper = TuiHostWrapper(tui_host.clone());
        Box::pin(async move {
            let server = TuiHostServer::new(wrapper);
            server.dispatch(method_id, &payload).await
        })
    }
}

/// Start the TUI plugin and run the host service
///
/// This:
/// 1. Creates a shared memory segment
/// 2. Spawns the TUI plugin process with --shm-path arg
/// 3. Serves TuiHost RPCs via SHM transport
/// 4. Returns the handle and waits for the TUI to exit
///
/// The `shutdown_rx` allows external signaling to stop the TUI.
pub async fn start_tui_plugin(
    tui_host: TuiHostImpl,
    plugin_path: PathBuf,
    mut shutdown_rx: Option<watch::Receiver<bool>>,
) -> Result<()> {
    // Create SHM file path
    let shm_path = format!("/tmp/dodeca-tui-{}.shm", std::process::id());

    // Clean up any stale SHM file
    let _ = std::fs::remove_file(&shm_path);

    // Create the SHM session (host side)
    let session = ShmSession::create_file(&shm_path, SHM_CONFIG)
        .map_err(|e| eyre::eyre!("Failed to create SHM for TUI: {:?}", e))?;
    tracing::debug!(
        "TUI SHM segment: {} ({}KB)",
        shm_path,
        SHM_CONFIG.slot_size * SHM_CONFIG.slot_count / 1024
    );

    // Spawn the TUI plugin process
    let mut child = Command::new(&plugin_path)
        .arg(format!("--shm-path={}", shm_path))
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    tracing::debug!("Spawned TUI plugin: {}", plugin_path.display());

    // Give the plugin time to map the SHM
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Create the SHM transport and wrap in RpcSession
    let transport: Arc<TuiTransport> = Arc::new(ShmTransport::new(session));

    // Host uses odd channel IDs (1, 3, 5, ...)
    // Plugin uses even channel IDs (2, 4, 6, ...)
    let rpc_session = Arc::new(RpcSession::with_channel_start(transport, 1));
    tracing::debug!("TUI plugin connected via SHM");

    // Set up dispatcher for TuiHost service
    let tui_host_arc = Arc::new(tui_host);
    rpc_session.set_dispatcher(create_tui_dispatcher(tui_host_arc));

    // Spawn the RPC session demux loop
    let session_runner = rpc_session.clone();
    tokio::spawn(async move {
        if let Err(e) = session_runner.run().await {
            tracing::error!("TUI RPC session error: {:?}", e);
        }
    });

    // Wait for TUI process to exit or shutdown signal
    loop {
        tokio::select! {
            status = child.wait() => {
                match status {
                    Ok(s) => tracing::debug!("TUI plugin exited with status: {}", s),
                    Err(e) => tracing::error!("TUI plugin wait error: {:?}", e),
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
                tracing::info!("Shutdown signal received, stopping TUI plugin");
                let _ = child.kill().await;
                break;
            }
        }
    }

    // Cleanup: remove SHM file
    let _ = std::fs::remove_file(&shm_path);

    Ok(())
}
