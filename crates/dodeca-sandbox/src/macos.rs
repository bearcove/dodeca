//! macOS sandboxing backend using Seatbelt (sandbox-exec).
//!
//! This implementation generates SBPL (Sandbox Profile Language) profiles
//! and uses sandbox-exec to run processes with restrictions.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{self, Stdio as StdStdio};

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
            stdin: Stdio::Null,
            stdout: Stdio::Piped,
            stderr: Stdio::Piped,
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
    /// Discard (redirect to /dev/null).
    Null,
}

impl From<Stdio> for StdStdio {
    fn from(stdio: Stdio) -> Self {
        match stdio {
            Stdio::Inherit => StdStdio::inherit(),
            Stdio::Piped => StdStdio::piped(),
            Stdio::Null => StdStdio::null(),
        }
    }
}

/// A command to be executed in a sandbox.
pub struct Command<'a> {
    config: &'a SandboxConfig,
    program: PathBuf,
    args: Vec<String>,
    env: HashMap<String, String>,
    working_dir: Option<PathBuf>,
    stdin: Stdio,
    stdout: Stdio,
    stderr: Stdio,
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
    pub fn stdin(mut self, stdin: Stdio) -> Self {
        self.stdin = stdin;
        self
    }

    /// Configure stdout.
    pub fn stdout(mut self, stdout: Stdio) -> Self {
        self.stdout = stdout;
        self
    }

    /// Configure stderr.
    pub fn stderr(mut self, stderr: Stdio) -> Self {
        self.stderr = stderr;
        self
    }

    /// Generate the SBPL profile for this configuration.
    fn generate_profile(&self) -> String {
        let mut profile = String::new();

        // Version declaration
        profile.push_str("(version 1)\n");

        // Default deny
        profile.push_str("(deny default)\n");

        // Import macOS system profile - provides essential low-level permissions for:
        // - locale info
        // - system libraries (/System/Library, /usr/lib, etc)
        // - basic tools (/etc, /dev/urandom, etc)
        // - Apple services (com.apple.system, com.apple.dyld, etc)
        profile.push_str("(import \"/System/Library/Sandbox/Profiles/system.sb\")\n");

        // Debug logging for denied operations (useful for debugging)
        profile.push_str("(debug deny)\n");

        // Allow process execution
        profile.push_str("(allow process-fork)\n");

        // /tmp access
        if self.config.allow_tmp {
            profile.push_str("(allow file-read* file-write* (subpath \"/tmp\"))\n");
            profile.push_str("(allow file-read* file-write* (subpath \"/private/tmp\"))\n");
            // Also allow var folders for temp files
            profile.push_str(
                "(allow file-read* file-write* (regex #\"^/private/var/folders/.*\"))\n",
            );
        }

        // Add configured paths
        for (path, access) in &self.config.paths {
            let path_str = path.to_string_lossy();
            // Escape special characters for SBPL
            let escaped_path = escape_sbpl_path(&path_str);

            match access {
                PathAccess::Read => {
                    profile.push_str(&format!(
                        "(allow file-read* (subpath \"{escaped_path}\"))\n"
                    ));
                }
                PathAccess::ReadWrite => {
                    profile.push_str(&format!(
                        "(allow file-read* file-write* (subpath \"{escaped_path}\"))\n"
                    ));
                }
                PathAccess::Execute => {
                    profile.push_str(&format!(
                        "(allow file-read* process-exec (literal \"{escaped_path}\"))\n"
                    ));
                }
                PathAccess::ReadExecute => {
                    profile.push_str(&format!(
                        "(allow file-read* (subpath \"{escaped_path}\"))\n"
                    ));
                    profile.push_str(&format!(
                        "(allow process-exec (subpath \"{escaped_path}\"))\n"
                    ));
                }
                PathAccess::Full => {
                    profile.push_str(&format!(
                        "(allow file-read* file-write* (subpath \"{escaped_path}\"))\n"
                    ));
                    profile.push_str(&format!(
                        "(allow process-exec (subpath \"{escaped_path}\"))\n"
                    ));
                }
            }
        }

        // Network configuration
        if self.config.network.allow_outbound {
            profile.push_str("(allow network-outbound)\n");
        }
        if self.config.network.allow_inbound {
            profile.push_str("(allow network-inbound)\n");
        }

        // Allow POSIX shared memory and semaphores (needed by many programs)
        profile.push_str("(allow ipc-posix-shm-read-data)\n");
        profile.push_str("(allow ipc-posix-shm-write-data)\n");

        profile
    }

    /// Execute the command and wait for it to complete, capturing output.
    pub fn output(self) -> Result<Output, Error> {
        // Generate the sandbox profile
        let profile = self.generate_profile();

        // Create a temporary file for the profile
        let profile_path = std::env::temp_dir().join(format!("sandbox-{}.sb", process::id()));
        std::fs::write(&profile_path, &profile)
            .map_err(|e| Error::ProfileGeneration(format!("failed to write profile: {e}")))?;

        // Build the sandbox-exec command
        let mut cmd = process::Command::new("/usr/bin/sandbox-exec");
        cmd.arg("-f").arg(&profile_path);
        cmd.arg(&self.program);
        cmd.args(&self.args);

        // Set up environment
        cmd.env_clear();

        // Inherit configured vars
        for key in &self.config.inherit_env {
            if let Ok(value) = std::env::var(key) {
                cmd.env(key, value);
            }
        }

        // Add config-level env vars
        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        // Add command-level env vars
        for (key, value) in &self.env {
            cmd.env(key, value);
        }

        // Set working directory
        if let Some(ref dir) = self.working_dir {
            cmd.current_dir(dir);
        }

        // Configure I/O
        cmd.stdin(self.stdin);
        cmd.stdout(self.stdout);
        cmd.stderr(self.stderr);

        // Execute with optional timeout
        let std_output = if let Some(timeout) = self.config.timeout {
            execute_with_timeout(&mut cmd, timeout)?
        } else {
            cmd.output()
                .map_err(|e| Error::Spawn(format!("failed to execute: {e}")))?
        };

        // Clean up the profile file
        let _ = std::fs::remove_file(&profile_path);

        Ok(Output {
            status: ExitStatus {
                code: std_output.status.code().unwrap_or(-1),
                success: std_output.status.success(),
            },
            stdout: std_output.stdout,
            stderr: std_output.stderr,
        })
    }

    /// Execute the command and wait for it to complete.
    pub fn status(self) -> Result<ExitStatus, Error> {
        let output = self.output()?;
        Ok(output.status)
    }
}

/// Execute a command with a timeout.
fn execute_with_timeout(
    cmd: &mut process::Command,
    timeout: std::time::Duration,
) -> Result<process::Output, Error> {
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Spawn(format!("failed to spawn: {e}")))?;

    let start = std::time::Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Process finished
                let stdout = child
                    .stdout
                    .take()
                    .map(|mut s| {
                        let mut buf = Vec::new();
                        std::io::Read::read_to_end(&mut s, &mut buf).ok();
                        buf
                    })
                    .unwrap_or_default();
                let stderr = child
                    .stderr
                    .take()
                    .map(|mut s| {
                        let mut buf = Vec::new();
                        std::io::Read::read_to_end(&mut s, &mut buf).ok();
                        buf
                    })
                    .unwrap_or_default();

                return Ok(process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                // Still running
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return Err(Error::Timeout(timeout.as_secs()));
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                return Err(Error::Spawn(format!("failed to wait: {e}")));
            }
        }
    }
}

/// Escape a path for use in SBPL.
fn escape_sbpl_path(path: &str) -> String {
    path.replace('\\', "\\\\").replace('"', "\\\"")
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

    #[test]
    fn test_simple_command() {
        let config = SandboxConfig::new()
            .allow_read("/usr")
            .allow_read_execute("/bin");

        let sandbox = Sandbox::new(config).unwrap();
        let output = sandbox.command("/bin/echo").arg("hello").output().unwrap();

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }

    #[test]
    fn test_profile_generation() {
        let config = SandboxConfig::new()
            .allow_read("/usr")
            .allow_read_write("/tmp/test")
            .allow_execute("/usr/bin/cargo")
            .deny_network();

        let sandbox = Sandbox::new(config).unwrap();
        let cmd = sandbox.command("/bin/echo");
        let profile = cmd.generate_profile();

        assert!(profile.contains("(version 1)"));
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(allow file-read* (subpath \"/usr\"))"));
        assert!(profile.contains("(allow file-read* file-write* (subpath \"/tmp/test\"))"));
        assert!(!profile.contains("network-outbound"));
    }

    #[test]
    fn test_write_blocked() {
        let config = SandboxConfig::new()
            .allow_read("/var") // read-only!
            .allow_read_execute("/usr")
            .allow_read_execute("/bin");

        let sandbox = Sandbox::new(config).unwrap();

        // Try to write to a read-only location
        let output = sandbox
            .command("/usr/bin/touch")
            .arg("/var/test-file")
            .output()
            .unwrap();

        // Should fail because /var is read-only
        assert!(!output.status.success());
    }

    #[test]
    fn test_env_inheritance() {
        // SAFETY: This test runs in isolation and we're setting a test-specific variable
        unsafe { std::env::set_var("TEST_SANDBOX_VAR", "test_value") };

        let config = SandboxConfig::new()
            .allow_read("/usr")
            .allow_read_execute("/bin")
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
