//! Fallback for unsupported platforms.
//!
//! This module provides stub implementations that return errors on platforms
//! where sandboxing is not supported.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::config::SandboxConfig;
use crate::error::Error;

/// A configured sandbox (not supported on this platform).
pub struct Sandbox {
    _config: SandboxConfig,
}

impl Sandbox {
    /// Create a new sandbox from the given configuration.
    ///
    /// This always returns an error on unsupported platforms.
    pub fn new(_config: SandboxConfig) -> Result<Self, Error> {
        Err(Error::Unsupported)
    }

    /// Create a command to run in this sandbox.
    pub fn command(&self, program: impl AsRef<Path>) -> Command {
        Command {
            program: program.as_ref().to_path_buf(),
        }
    }
}

/// Stdio configuration.
#[derive(Debug, Clone, Copy)]
pub enum Stdio {
    /// Inherit from parent.
    Inherit,
    /// Capture to a pipe.
    Piped,
    /// Discard.
    Null,
}

/// A command (not supported on this platform).
pub struct Command {
    program: PathBuf,
}

impl Command {
    pub fn arg(self, _arg: impl AsRef<OsStr>) -> Self {
        self
    }

    pub fn args<I, S>(self, _args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self
    }

    pub fn env(self, _key: impl Into<String>, _value: impl Into<String>) -> Self {
        self
    }

    pub fn current_dir(self, _dir: impl AsRef<Path>) -> Self {
        self
    }

    pub fn stdin(self, _stdin: Stdio) -> Self {
        self
    }

    pub fn stdout(self, _stdout: Stdio) -> Self {
        self
    }

    pub fn stderr(self, _stderr: Stdio) -> Self {
        self
    }

    pub fn output(self) -> Result<Output, Error> {
        Err(Error::Unsupported)
    }

    pub fn status(self) -> Result<ExitStatus, Error> {
        Err(Error::Unsupported)
    }
}

/// Exit status from a sandboxed command.
#[derive(Debug, Clone)]
pub struct ExitStatus {
    /// The exit code.
    pub code: i32,
    success: bool,
}

impl ExitStatus {
    /// Returns true if the process exited successfully (code 0).
    pub fn success(&self) -> bool {
        self.success
    }

    /// Returns the exit code if the process exited normally.
    pub fn code(&self) -> Option<i32> {
        Some(self.code)
    }
}

/// Output from a sandboxed command.
#[derive(Debug)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
