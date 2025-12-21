//! Boot state machine for tracking server readiness
//!
//! This module provides a state machine that tracks the server's boot lifecycle,
//! allowing connection handlers to wait for readiness instead of refusing/resetting
//! connections during startup.

use std::time::Instant;
use tokio::sync::watch;

/// Boot state of the server
#[derive(Clone, Debug)]
pub enum BootState {
    /// Server is booting - cells loading, revision building
    Booting { phase: BootPhase },
    /// Server is ready to handle requests
    Ready,
    /// Fatal startup error - server will serve HTTP 500s
    Fatal {
        error_kind: ErrorKind,
        message: String,
    },
}

/// Boot phase during startup
#[derive(Clone, Debug)]
pub enum BootPhase {
    /// Loading cell binaries
    LoadingCells,
    /// Waiting for cells to become ready
    WaitingCellsReady,
}

/// Error kind for fatal boot failures
#[derive(Clone, Debug)]
pub enum ErrorKind {
    /// Required cell binary not found or not executable
    MissingCell,
    /// Cell failed to start or communicate
    CellStartupFailed,
}

impl BootState {
    /// Create initial booting state
    pub fn booting() -> Self {
        Self::Booting {
            phase: BootPhase::LoadingCells,
        }
    }

    /// Transition to ready state
    pub fn ready() -> Self {
        Self::Ready
    }

    /// Transition to fatal error state
    pub fn fatal(error_kind: ErrorKind, message: impl Into<String>) -> Self {
        Self::Fatal {
            error_kind,
            message: message.into(),
        }
    }
}

/// Boot state manager - tracks and broadcasts boot state transitions
pub struct BootStateManager {
    tx: watch::Sender<BootState>,
    rx: watch::Receiver<BootState>,
    start_time: Instant,
}

impl BootStateManager {
    /// Create a new boot state manager
    pub fn new() -> Self {
        let start_time = Instant::now();
        let (tx, rx) = watch::channel(BootState::booting());
        Self { tx, rx, start_time }
    }

    /// Get a receiver to watch boot state
    pub fn subscribe(&self) -> watch::Receiver<BootState> {
        self.rx.clone()
    }

    /// Update the boot phase
    pub fn set_phase(&self, phase: BootPhase) {
        let elapsed_ms = self.start_time.elapsed().as_millis();
        tracing::info!(
            elapsed_ms,
            phase = ?phase,
            "Boot phase transition"
        );
        let _ = self.tx.send(BootState::Booting { phase });
    }

    /// Mark the server as ready
    pub fn set_ready(&self) {
        let elapsed_ms = self.start_time.elapsed().as_millis();
        tracing::info!(elapsed_ms, "Boot complete - server ready");
        let _ = self.tx.send(BootState::ready());
    }

    /// Mark the server as fatally failed
    pub fn set_fatal(&self, error_kind: ErrorKind, message: impl Into<String>) {
        let elapsed_ms = self.start_time.elapsed().as_millis();
        let message = message.into();
        tracing::error!(
            elapsed_ms,
            error_kind = ?error_kind,
            message = %message,
            "Boot failed fatally"
        );
        let _ = self.tx.send(BootState::fatal(error_kind, message));
    }
}

impl Default for BootStateManager {
    fn default() -> Self {
        Self::new()
    }
}
