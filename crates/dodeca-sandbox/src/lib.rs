//! Cross-platform process sandboxing.
//!
//! This crate provides a unified API for running processes in a sandboxed environment
//! across different operating systems:
//!
//! - **Linux**: Uses [hakoniwa](https://docs.rs/hakoniwa) (namespaces, seccomp, landlock)
//! - **macOS**: Uses Seatbelt (sandbox-exec with SBPL profiles)
//!
//! # Example
//!
//! ```no_run
//! use dodeca_sandbox::{Sandbox, SandboxConfig};
//!
//! let config = SandboxConfig::new()
//!     .allow_read("/usr")
//!     .allow_read("/lib")
//!     .allow_read("/lib64")
//!     .allow_read_write("/tmp/my-project")
//!     .allow_execute("/usr/bin/cargo");
//!
//! let sandbox = Sandbox::new(config)?;
//! let output = sandbox
//!     .command("/usr/bin/cargo")
//!     .args(["build", "--release"])
//!     .current_dir("/tmp/my-project")
//!     .output()?;
//! # Ok::<(), dodeca_sandbox::Error>(())
//! ```

mod config;
mod error;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod unsupported;

pub use config::{SandboxConfig, PathAccess};
pub use error::Error;

#[cfg(target_os = "linux")]
pub use linux::{Sandbox, Command, Output, ExitStatus, Stdio};

#[cfg(target_os = "macos")]
pub use macos::{Sandbox, Command, Output, ExitStatus, Stdio};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use unsupported::{Sandbox, Command, Output, ExitStatus, Stdio};

/// Result type for sandbox operations.
pub type Result<T> = std::result::Result<T, Error>;
