//! Error types for sandboxing operations.

use std::path::PathBuf;

/// Errors that can occur during sandbox operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The sandbox backend is not available on this platform.
    #[error("sandboxing is not supported on this platform")]
    Unsupported,

    /// Failed to create the sandbox environment.
    #[error("failed to create sandbox: {0}")]
    Creation(String),

    /// Failed to spawn the sandboxed process.
    #[error("failed to spawn process: {0}")]
    Spawn(String),

    /// The process timed out.
    #[error("process timed out after {0} seconds")]
    Timeout(u64),

    /// I/O error during sandbox operations.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A required path does not exist.
    #[error("path does not exist: {}", .0.display())]
    PathNotFound(PathBuf),

    /// Failed to generate sandbox profile (macOS).
    #[error("failed to generate sandbox profile: {0}")]
    ProfileGeneration(String),

    /// The sandbox denied an operation.
    #[error("sandbox denied operation: {0}")]
    Denied(String),

    /// Invalid configuration.
    #[error("invalid sandbox configuration: {0}")]
    InvalidConfig(String),
}
