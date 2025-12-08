//! Linux sandboxing backend using hakoniwa.
//!
//! This implementation uses Linux namespaces, seccomp, and landlock
//! via the hakoniwa crate to provide process isolation.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use hakoniwa::{Container, Namespace, Stdio as HakoStdio};

use crate::config::{PathAccess, SandboxConfig};
use crate::error::Error;

/// A configured sandbox ready to run commands.
pub struct Sandbox {
    config: SandboxConfig,
}

impl Sandbox {
    /// Create a new sandbox from the given configuration.
    pub fn new(config: SandboxConfig) -> Result<Self, Error> {
        Ok(Self { config })
    }

    /// Create a command to run in this sandbox.
    pub fn command(&self, program: impl AsRef<Path>) -> Command<'_> {
        Command {
            config: &self.config,
            program: program.as_ref().to_path_buf(),
            args: Vec::new(),
            env: HashMap::new(),
            working_dir: self.config.working_dir.clone(),
            stdin: StdioConfig::Null,
            stdout: StdioConfig::Piped,
            stderr: StdioConfig::Piped,
        }
    }
}

/// Stdio configuration for our API.
#[derive(Debug, Clone, Copy)]
pub enum StdioConfig {
    /// Inherit from parent.
    Inherit,
    /// Capture to a pipe.
    Piped,
    /// Discard (not directly supported by hakoniwa, we use piped and discard).
    Null,
}

/// Re-export for compatibility.
pub type Stdio = StdioConfig;

/// A command to be executed in a sandbox.
pub struct Command<'a> {
    config: &'a SandboxConfig,
    program: PathBuf,
    args: Vec<String>,
    env: HashMap<String, String>,
    working_dir: Option<PathBuf>,
    stdin: StdioConfig,
    stdout: StdioConfig,
    stderr: StdioConfig,
}

impl<'a> Command<'a> {
    /// Add an argument to the command.
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_string_lossy().into_owned());
        self
    }

    /// Add multiple arguments to the command.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self.args.push(arg.as_ref().to_string_lossy().into_owned());
        }
        self
    }

    /// Set an environment variable.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set the working directory.
    pub fn current_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.working_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Configure stdin.
    pub fn stdin(mut self, stdin: StdioConfig) -> Self {
        self.stdin = stdin;
        self
    }

    /// Configure stdout.
    pub fn stdout(mut self, stdout: StdioConfig) -> Self {
        self.stdout = stdout;
        self
    }

    /// Configure stderr.
    pub fn stderr(mut self, stderr: StdioConfig) -> Self {
        self.stderr = stderr;
        self
    }

    /// Paths that are already mounted by rootfs("/")
    const ROOTFS_PATHS: &'static [&'static str] =
        &["/bin", "/etc", "/lib", "/lib64", "/lib32", "/sbin", "/usr"];

    /// Check if a path is already provided by rootfs
    fn is_rootfs_path(path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        Self::ROOTFS_PATHS
            .iter()
            .any(|&p| path_str == p || path_str.starts_with(&format!("{p}/")))
    }

    /// Build the hakoniwa container from our config.
    fn build_container(&self) -> Result<Container, Error> {
        let mut container = Container::new();

        // Set up the root filesystem with standard system directories
        // This automatically mounts /bin, /etc, /lib, /lib64, /lib32, /sbin, /usr as read-only
        container
            .rootfs("/")
            .map_err(|e| Error::Creation(format!("failed to set rootfs: {e}")))?;

        // Mount /tmp as tmpfs if allowed
        if self.config.allow_tmp {
            container.tmpfsmount("/tmp");
        }

        // Mount /proc if allowed
        if self.config.allow_proc {
            container.procfsmount("/proc");
        }

        // Mount /dev basics if allowed
        if self.config.allow_dev_basics {
            container.devfsmount("/dev");
        }

        // Add configured paths (skip paths already mounted by rootfs)
        for (path, access) in &self.config.paths {
            // Skip paths that are already provided by rootfs("/")
            if Self::is_rootfs_path(path) {
                // If it's a read-only access, it's already satisfied
                // If it's read-write, we need to re-mount it
                match access {
                    PathAccess::Read | PathAccess::Execute | PathAccess::ReadExecute => {
                        // Already mounted as read-only by rootfs, skip
                        continue;
                    }
                    PathAccess::ReadWrite | PathAccess::Full => {
                        // Need to re-mount as read-write (not ideal, but necessary)
                        let path_str = path.to_string_lossy();
                        container.bindmount_rw(&path_str, &path_str);
                    }
                }
            } else {
                let path_str = path.to_string_lossy();
                match access {
                    PathAccess::Read | PathAccess::Execute | PathAccess::ReadExecute => {
                        container.bindmount_ro(&path_str, &path_str);
                    }
                    PathAccess::ReadWrite | PathAccess::Full => {
                        container.bindmount_rw(&path_str, &path_str);
                    }
                }
            }
        }

        // Configure network isolation
        if !self.config.network.allow_outbound && !self.config.network.allow_inbound {
            container.unshare(Namespace::Network);
        }

        Ok(container)
    }

    /// Convert our stdio config to hakoniwa's.
    fn to_hako_stdio(config: StdioConfig) -> HakoStdio {
        match config {
            StdioConfig::Inherit => HakoStdio::inherit(),
            // For Null and Piped, we use piped() and just discard Null output
            StdioConfig::Piped | StdioConfig::Null => HakoStdio::piped(),
        }
    }

    /// Execute the command and wait for it to complete, capturing output.
    pub fn output(self) -> Result<Output, Error> {
        let container = self.build_container()?;

        let program_str = self.program.to_string_lossy();
        let mut cmd = container.command(&program_str);

        // Add arguments
        for arg in &self.args {
            cmd.arg(arg);
        }

        // Set up environment - start with inherited vars
        for key in &self.config.inherit_env {
            if let Ok(value) = std::env::var(key) {
                cmd.env(key, &value);
            }
        }

        // Add config-level env vars
        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        // Add command-level env vars (override config)
        for (key, value) in &self.env {
            cmd.env(key, value);
        }

        // Set working directory
        if let Some(ref dir) = self.working_dir {
            cmd.current_dir(dir);
        }

        // Configure I/O
        cmd.stdin(Self::to_hako_stdio(self.stdin));
        cmd.stdout(Self::to_hako_stdio(self.stdout));
        cmd.stderr(Self::to_hako_stdio(self.stderr));

        // Set timeout if configured
        if let Some(timeout) = self.config.timeout {
            cmd.wait_timeout(timeout.as_secs());
        }

        // Execute and capture output
        let hako_output = cmd
            .output()
            .map_err(|e| Error::Spawn(format!("failed to execute: {e}")))?;

        Ok(Output {
            status: ExitStatus {
                code: hako_output.status.code,
                success: hako_output.status.success(),
            },
            stdout: hako_output.stdout,
            stderr: hako_output.stderr,
        })
    }

    /// Execute the command and wait for it to complete.
    pub fn status(self) -> Result<ExitStatus, Error> {
        let output = self.output()?;
        Ok(output.status)
    }
}

/// Exit status from a sandboxed command.
#[derive(Debug, Clone)]
pub struct ExitStatus {
    /// The exit code.
    pub code: i32,
    /// Whether the process exited successfully.
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
    /// The exit status of the process.
    pub status: ExitStatus,
    /// The stdout output.
    pub stdout: Vec<u8>,
    /// The stderr output.
    pub stderr: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SandboxConfig;
    use std::time::Duration;

    #[test]
    fn test_simple_command() {
        let config = SandboxConfig::new()
            .allow_read("/usr")
            .allow_read("/lib")
            .allow_read("/lib64")
            .allow_read("/bin");

        let sandbox = Sandbox::new(config).unwrap();
        let output = sandbox.command("/bin/echo").arg("hello").output().unwrap();

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }

    #[test]
    fn test_network_blocked() {
        let config = SandboxConfig::new()
            .allow_read("/usr")
            .allow_read("/lib")
            .allow_read("/lib64")
            .allow_read("/bin")
            .deny_network()
            .timeout(Duration::from_secs(5));

        let sandbox = Sandbox::new(config).unwrap();

        // This should fail because network is blocked
        let output = sandbox
            .command("/bin/ping")
            .args(["-c", "1", "127.0.0.1"])
            .output();

        // ping should fail without network access
        if let Ok(output) = output {
            assert!(!output.status.success());
        }
    }

    #[test]
    fn test_write_blocked() {
        let config = SandboxConfig::new()
            .allow_read("/usr")
            .allow_read("/lib")
            .allow_read("/lib64")
            .allow_read("/bin")
            .allow_read("/home"); // read-only!

        let sandbox = Sandbox::new(config).unwrap();

        // Try to write to a read-only location
        let output = sandbox
            .command("/bin/touch")
            .arg("/home/test-file")
            .output()
            .unwrap();

        // Should fail because /home is read-only
        assert!(!output.status.success());
    }

    #[test]
    fn test_env_inheritance() {
        // SAFETY: This test runs in isolation and we're setting a test-specific variable
        unsafe { std::env::set_var("TEST_SANDBOX_VAR", "test_value") };

        let config = SandboxConfig::new()
            .allow_read("/usr")
            .allow_read("/lib")
            .allow_read("/lib64")
            .allow_read("/bin")
            .inherit_env("TEST_SANDBOX_VAR");

        let sandbox = Sandbox::new(config).unwrap();
        let output = sandbox
            .command("/bin/sh")
            .args(["-c", "echo $TEST_SANDBOX_VAR"])
            .output()
            .unwrap();

        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "test_value"
        );
    }
}
