//! Sandbox configuration types.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Access level for a path in the sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathAccess {
    /// Read-only access to files.
    Read,
    /// Read and write access to files.
    ReadWrite,
    /// Execute access (for binaries).
    Execute,
    /// Read and execute access.
    ReadExecute,
    /// Full access (read, write, execute).
    Full,
}

/// Network access configuration.
#[derive(Debug, Clone, Default)]
pub struct NetworkConfig {
    /// Allow outbound network connections.
    pub allow_outbound: bool,
    /// Allow inbound network connections.
    pub allow_inbound: bool,
    /// Specific hosts/ports to allow (if not allowing all).
    pub allowed_endpoints: Vec<String>,
}

/// Configuration for a sandboxed process.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Paths and their access levels.
    pub(crate) paths: Vec<(PathBuf, PathAccess)>,

    /// Environment variables to set.
    pub(crate) env: HashMap<String, String>,

    /// Environment variables to inherit from the parent process.
    pub(crate) inherit_env: Vec<String>,

    /// Network configuration.
    pub(crate) network: NetworkConfig,

    /// Process timeout.
    pub(crate) timeout: Option<Duration>,

    /// Working directory for the process.
    pub(crate) working_dir: Option<PathBuf>,

    /// Allow access to /dev/null, /dev/zero, /dev/urandom.
    pub(crate) allow_dev_basics: bool,

    /// Allow access to /proc (read-only).
    pub(crate) allow_proc: bool,

    /// Allow access to /tmp (creates isolated tmpfs on Linux).
    pub(crate) allow_tmp: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl SandboxConfig {
    /// Create a new sandbox configuration with sensible defaults.
    ///
    /// By default:
    /// - No paths are accessible
    /// - No network access
    /// - Basic devices (/dev/null, /dev/zero, /dev/urandom) are allowed
    /// - /proc is allowed (read-only)
    /// - An isolated /tmp is provided
    pub fn new() -> Self {
        Self {
            paths: Vec::new(),
            env: HashMap::new(),
            inherit_env: Vec::new(),
            network: NetworkConfig::default(),
            timeout: None,
            working_dir: None,
            allow_dev_basics: true,
            allow_proc: true,
            allow_tmp: true,
        }
    }

    /// Allow read-only access to a path and all its contents.
    pub fn allow_read(mut self, path: impl AsRef<Path>) -> Self {
        self.paths.push((path.as_ref().to_path_buf(), PathAccess::Read));
        self
    }

    /// Allow read-write access to a path and all its contents.
    pub fn allow_read_write(mut self, path: impl AsRef<Path>) -> Self {
        self.paths.push((path.as_ref().to_path_buf(), PathAccess::ReadWrite));
        self
    }

    /// Allow execute access to a path (typically a binary).
    pub fn allow_execute(mut self, path: impl AsRef<Path>) -> Self {
        self.paths.push((path.as_ref().to_path_buf(), PathAccess::Execute));
        self
    }

    /// Allow read and execute access to a path.
    pub fn allow_read_execute(mut self, path: impl AsRef<Path>) -> Self {
        self.paths.push((path.as_ref().to_path_buf(), PathAccess::ReadExecute));
        self
    }

    /// Allow full access (read, write, execute) to a path.
    pub fn allow_full(mut self, path: impl AsRef<Path>) -> Self {
        self.paths.push((path.as_ref().to_path_buf(), PathAccess::Full));
        self
    }

    /// Add multiple read-only paths at once.
    pub fn allow_read_many(mut self, paths: impl IntoIterator<Item = impl AsRef<Path>>) -> Self {
        for path in paths {
            self.paths.push((path.as_ref().to_path_buf(), PathAccess::Read));
        }
        self
    }

    /// Set an environment variable for the sandboxed process.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Inherit an environment variable from the parent process.
    pub fn inherit_env(mut self, key: impl Into<String>) -> Self {
        self.inherit_env.push(key.into());
        self
    }

    /// Inherit multiple environment variables from the parent process.
    pub fn inherit_env_many(mut self, keys: impl IntoIterator<Item = impl Into<String>>) -> Self {
        for key in keys {
            self.inherit_env.push(key.into());
        }
        self
    }

    /// Allow outbound network connections.
    pub fn allow_network_outbound(mut self) -> Self {
        self.network.allow_outbound = true;
        self
    }

    /// Allow inbound network connections.
    pub fn allow_network_inbound(mut self) -> Self {
        self.network.allow_inbound = true;
        self
    }

    /// Allow all network access.
    pub fn allow_network(mut self) -> Self {
        self.network.allow_outbound = true;
        self.network.allow_inbound = true;
        self
    }

    /// Deny all network access (default).
    pub fn deny_network(mut self) -> Self {
        self.network.allow_outbound = false;
        self.network.allow_inbound = false;
        self.network.allowed_endpoints.clear();
        self
    }

    /// Set a timeout for the sandboxed process.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the working directory for the sandboxed process.
    pub fn working_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.working_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Control access to basic devices (/dev/null, /dev/zero, /dev/urandom).
    pub fn allow_dev_basics(mut self, allow: bool) -> Self {
        self.allow_dev_basics = allow;
        self
    }

    /// Control access to /proc.
    pub fn allow_proc(mut self, allow: bool) -> Self {
        self.allow_proc = allow;
        self
    }

    /// Control access to /tmp (on Linux, this creates an isolated tmpfs).
    pub fn allow_tmp(mut self, allow: bool) -> Self {
        self.allow_tmp = allow;
        self
    }

    /// Convenience method to configure for building Rust projects.
    ///
    /// This sets up common paths needed for cargo/rustc:
    /// - Read access to /usr, /lib, /lib64, /bin, /etc
    /// - Inherits HOME, USER, PATH, RUSTUP_HOME, CARGO_HOME
    pub fn for_rust_build(self) -> Self {
        self.allow_read("/usr")
            .allow_read("/lib")
            .allow_read("/lib64")
            .allow_read("/bin")
            .allow_read("/etc")
            .inherit_env("HOME")
            .inherit_env("USER")
            .inherit_env("PATH")
            .inherit_env("RUSTUP_HOME")
            .inherit_env("CARGO_HOME")
    }
}
