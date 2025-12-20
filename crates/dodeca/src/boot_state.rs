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
    Booting { since: Instant, phase: BootPhase },
    /// Server is ready to handle requests
    Ready { since: Instant, generation: u64 },
    /// Fatal startup error - server will serve HTTP 500s
    Fatal {
        since: Instant,
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
    /// Building initial revision
    BuildingRevision,
}

/// Error kind for fatal boot failures
#[derive(Clone, Debug)]
pub enum ErrorKind {
    /// Required cell binary not found or not executable
    MissingCell,
    /// Cell failed to start or communicate
    CellStartupFailed,
    /// Initial revision build failed
    RevisionBuildFailed,
}

impl BootState {
    /// Create initial booting state
    pub fn booting() -> Self {
        Self::Booting {
            since: Instant::now(),
            phase: BootPhase::LoadingCells,
        }
    }

    /// Transition to ready state
    pub fn ready(generation: u64) -> Self {
        Self::Ready {
            since: Instant::now(),
            generation,
        }
    }

    /// Transition to fatal error state
    pub fn fatal(error_kind: ErrorKind, message: impl Into<String>) -> Self {
        Self::Fatal {
            since: Instant::now(),
            error_kind,
            message: message.into(),
        }
    }

    /// Check if the server is ready
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    /// Check if the server has a fatal error
    pub fn is_fatal(&self) -> bool {
        matches!(self, Self::Fatal { .. })
    }

    /// Get elapsed time since this state was entered
    pub fn elapsed(&self) -> std::time::Duration {
        match self {
            Self::Booting { since, .. } | Self::Ready { since, .. } | Self::Fatal { since, .. } => {
                since.elapsed()
            }
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
        let _ = self.tx.send(BootState::Booting {
            since: Instant::now(),
            phase,
        });
    }

    /// Mark the server as ready
    pub fn set_ready(&self, generation: u64) {
        let elapsed_ms = self.start_time.elapsed().as_millis();
        tracing::info!(elapsed_ms, generation, "Boot complete - server ready");
        let _ = self.tx.send(BootState::ready(generation));
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

    /// Get the current boot state
    pub fn current(&self) -> BootState {
        self.rx.borrow().clone()
    }
}

impl Default for BootStateManager {
    fn default() -> Self {
        Self::new()
    }
}
