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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cargo_build() {
        use std::process::Command;

        // Create temp directory outside of /tmp to avoid conflict with
        // the sandbox's isolated tmpfs mount on Linux
        let home_dir = std::env::var("HOME").expect("HOME not set");
        let temp_dir = tempfile::tempdir_in(&home_dir).unwrap();
        let project_dir = temp_dir.path();

        // Create a minimal Rust project
        std::fs::write(
            project_dir.join("Cargo.toml"),
            r#"[package]
name = "sandbox-test-project"
version = "0.1.0"
edition = "2021"

[dependencies]
"#,
        )
        .unwrap();

        std::fs::create_dir(project_dir.join("src")).unwrap();
        std::fs::write(
            project_dir.join("src/main.rs"),
            r#"fn main() {
    println!("Hello from sandboxed build!");
}
"#,
        )
        .unwrap();

        // Run cargo fetch outside the sandbox to download dependencies
        // (the project has no deps, but this ensures the index is ready)
        let fetch_status = Command::new("cargo")
            .args(["fetch"])
            .current_dir(project_dir)
            .status()
            .expect("failed to run cargo fetch");
        assert!(fetch_status.success(), "cargo fetch failed");

        // Get paths from environment
        let rustup_home =
            std::env::var("RUSTUP_HOME").unwrap_or_else(|_| format!("{home_dir}/.rustup"));
        let cargo_home =
            std::env::var("CARGO_HOME").unwrap_or_else(|_| format!("{home_dir}/.cargo"));

        // Get the active toolchain's bin directory
        let rustc_path = Command::new("rustup")
            .args(["which", "rustc"])
            .output()
            .expect("failed to run rustup which rustc");
        let rustc_path = String::from_utf8_lossy(&rustc_path.stdout);
        let toolchain_bin = std::path::Path::new(rustc_path.trim())
            .parent()
            .expect("rustc path has no parent")
            .to_string_lossy()
            .to_string();

        // Get cargo path
        let cargo_path_output = Command::new("rustup")
            .args(["which", "cargo"])
            .output()
            .expect("failed to run rustup which cargo");
        let cargo_path = String::from_utf8_lossy(&cargo_path_output.stdout)
            .trim()
            .to_string();

        // Configure sandbox for cargo build - platform specific paths
        let config = SandboxConfig::new()
            // Rust toolchain (read + execute)
            .allow_read_execute(&rustup_home)
            .allow_read_execute(&cargo_home)
            // Project directory (read + write for build artifacts)
            .allow_full(project_dir)
            // Deny network (we've already fetched dependencies)
            .deny_network();

        // Platform-specific paths
        #[cfg(target_os = "macos")]
        let (config, extra_env) = {
            // Get SDK path for linker
            let sdk_path = Command::new("xcrun")
                .args(["--sdk", "macosx", "--show-sdk-path"])
                .output()
                .expect("failed to run xcrun --show-sdk-path");
            let sdk_path = String::from_utf8_lossy(&sdk_path.stdout).trim().to_string();

            // Get developer dir
            let developer_dir = Command::new("xcode-select")
                .args(["-p"])
                .output()
                .expect("failed to run xcode-select -p");
            let developer_dir = String::from_utf8_lossy(&developer_dir.stdout)
                .trim()
                .to_string();

            let config = config
                // System paths needed for compilation
                .allow_read_execute("/usr")
                .allow_read_execute("/bin")
                // SSL/TLS configuration
                .allow_read("/private/etc")
                // xcode-select symlinks
                .allow_read("/private/var/select")
                // Homebrew (for linker, etc.)
                .allow_read_execute("/opt/homebrew")
                // Library paths
                .allow_read("/Library")
                // Xcode / developer tools (linker, SDK)
                .allow_read_execute("/Applications/Xcode.app")
                .allow_read_execute("/Applications/Xcode-beta.app")
                .allow_read_execute("/Library/Developer");

            let extra_env = vec![
                ("SDKROOT".to_string(), sdk_path),
                ("DEVELOPER_DIR".to_string(), developer_dir),
            ];
            (config, extra_env)
        };

        #[cfg(target_os = "linux")]
        let (config, extra_env) = {
            let config = config
                // System paths needed for compilation
                .allow_read_execute("/usr")
                .allow_read_execute("/bin")
                .allow_read_execute("/lib")
                .allow_read_execute("/lib64")
                // SSL/TLS configuration
                .allow_read("/etc")
                // Proc filesystem (needed by some tools)
                .allow_read("/proc");

            let extra_env: Vec<(String, String)> = vec![];
            (config, extra_env)
        };

        let sandbox = Sandbox::new(config).unwrap();
        let mut cmd = sandbox
            .command(&cargo_path)
            .args(["build", "--release"])
            .current_dir(project_dir)
            .env("RUSTUP_HOME", &rustup_home)
            .env("CARGO_HOME", &cargo_home)
            .env("HOME", &home_dir)
            .env(
                "PATH",
                format!("{cargo_home}/bin:{toolchain_bin}:/usr/bin:/bin"),
            );

        // Add platform-specific env vars
        for (key, value) in extra_env {
            cmd = cmd.env(&key, &value);
        }

        let output = cmd.output().unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "cargo build failed:\nstdout: {stdout}\nstderr: {stderr}"
        );

        // Verify the binary was created
        let binary_path = project_dir.join("target/release/sandbox-test-project");
        assert!(
            binary_path.exists(),
            "binary was not created at {binary_path:?}"
        );
    }
}
