//! Vite dev server management
//!
//! Spawns and manages a Vite dev server process for seamless frontend development.

use eyre::{Result, WrapErr};
use owo_colors::OwoColorize;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

/// Log a status message, routing through tracing in TUI mode
macro_rules! status {
    ($($arg:tt)*) => {
        if crate::host::Host::get().is_tui_mode() {
            tracing::info!(target: "vite", $($arg)*);
        } else {
            eprintln!($($arg)*);
        }
    };
}

/// Check if pnpm is available in PATH
fn check_pnpm_available() -> Result<()> {
    match std::process::Command::new("pnpm")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => eyre::bail!(
            "pnpm is installed but returned an error\n\
             Try running 'pnpm --version' to diagnose"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => eyre::bail!(
            "pnpm is not installed\n\
             \n\
             Vite integration requires pnpm. Install it with:\n\
             \n\
             \x20 curl -fsSL https://get.pnpm.io/install.sh | sh\n\
             \n\
             Or see: https://pnpm.io/installation"
        ),
        Err(e) => eyre::bail!("Failed to run pnpm: {}", e),
    }
}

/// Validate package.json exists and has required scripts
fn validate_package_json(project_dir: &Path, script: &str) -> Result<()> {
    let package_json_path = project_dir.join("package.json");

    if !package_json_path.exists() {
        eyre::bail!(
            "No package.json found in {}\n\
             \n\
             Vite integration requires a package.json with a '{}' script.\n\
             Create one with:\n\
             \n\
             \x20 pnpm init\n\
             \x20 pnpm add -D vite",
            project_dir.display(),
            script
        );
    }

    let content = fs::read_to_string(&package_json_path)
        .wrap_err_with(|| format!("Failed to read {}", package_json_path.display()))?;

    // Simple check for script presence (avoid pulling in a JSON parser just for this)
    let script_pattern = format!("\"{}\"", script);
    if !content.contains(&script_pattern) {
        eyre::bail!(
            "package.json missing '{}' script\n\
             \n\
             Add a '{}' script to your package.json:\n\
             \n\
             \x20 {{\n\
             \x20   \"scripts\": {{\n\
             \x20     \"{}\": \"vite{}\"\n\
             \x20   }}\n\
             \x20 }}",
            script,
            script,
            script,
            if script == "build" { " build" } else { "" }
        );
    }

    Ok(())
}

/// Information about a running Vite dev server
pub struct ViteServer {
    /// The port Vite is listening on
    pub port: u16,
    /// Handle to the child process (killed on drop)
    _child: Child,
}

impl ViteServer {
    /// Start a Vite dev server in the given directory
    pub async fn start(project_dir: &Path) -> Result<Self> {
        // Validate setup before attempting to start
        check_pnpm_available()?;
        validate_package_json(project_dir, "dev")?;

        status!(
            "   {} Vite dev server in {}",
            "Starting".blue().bold(),
            project_dir.display()
        );

        // Run pnpm install first to ensure dependencies are installed
        let install_status = Command::new("pnpm")
            .arg("install")
            .current_dir(project_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .await
            .wrap_err("Failed to run pnpm install")?;

        if !install_status.success() {
            eyre::bail!("pnpm install failed");
        }

        // Channel to receive the port from stdout parsing
        let (tx, mut rx) = mpsc::channel::<u16>(1);

        // Start vite dev server
        let mut child = Command::new("pnpm")
            .arg("run")
            .arg("dev")
            .current_dir(project_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .wrap_err("Failed to spawn Vite dev server")?;

        // Spawn tasks to read stdout/stderr and extract port
        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        let tx_clone = tx.clone();
        tokio::spawn(async move {
            relay_output(stdout, tx_clone).await;
        });

        tokio::spawn(async move {
            relay_output(stderr, tx).await;
        });

        // Wait for port with timeout
        let port = tokio::time::timeout(std::time::Duration::from_secs(30), rx.recv())
            .await
            .wrap_err("Timeout waiting for Vite to start")?
            .ok_or_else(|| eyre::eyre!("Vite process exited before reporting port"))?;

        status!(
            "   {} Vite dev server running on port {}",
            "OK".green().bold(),
            port
        );

        Ok(ViteServer {
            port,
            _child: child,
        })
    }
}

/// Read lines from a reader and extract the Vite port, forwarding other output
async fn relay_output<R: tokio::io::AsyncRead + Unpin>(reader: R, tx: mpsc::Sender<u16>) {
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        // Try to extract port from Vite's "Local: http://localhost:PORT/" output
        if let Some(port) = extract_vite_port(&line) {
            let _ = tx.send(port).await;
            // Don't print the localhost line - we'll print our own message
            continue;
        }

        // Skip empty lines
        if !line.trim().is_empty() {
            status!("   {} {}", "[vite]".dimmed(), line);
        }
    }
}

/// Extract the port from a Vite server output line
///
/// Vite outputs lines like:
///   ➜  Local:   http://localhost:5173/
/// possibly with ANSI escape codes
fn extract_vite_port(line: &str) -> Option<u16> {
    // Strip ANSI escape codes
    let stripped = strip_ansi_escapes(line);

    // Look for localhost URL pattern
    // Match "http://localhost:" or "http://127.0.0.1:" followed by port
    for pattern in &["http://localhost:", "http://127.0.0.1:"] {
        if let Some(idx) = stripped.find(pattern) {
            let after_pattern = &stripped[idx + pattern.len()..];
            // Extract digits until non-digit
            let port_str: String = after_pattern
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(port) = port_str.parse::<u16>() {
                return Some(port);
            }
        }
    }

    None
}

/// Simple ANSI escape code stripper
fn strip_ansi_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip escape sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                // Skip until we hit a letter (the command)
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Check if a directory has a Vite configuration file
pub fn has_vite_config(dir: &Path) -> bool {
    dir.join("vite.config.ts").exists()
        || dir.join("vite.config.js").exists()
        || dir.join("vite.config.mts").exists()
        || dir.join("vite.config.mjs").exists()
}

/// Ensure dist/ is in the .gitignore file
fn ensure_dist_gitignored(project_dir: &Path) {
    let gitignore_path = project_dir.join(".gitignore");
    let dist_entry = "dist";

    // Check if dist/ is already in the gitignore
    let needs_update = if gitignore_path.exists() {
        match fs::read_to_string(&gitignore_path) {
            Ok(content) => !content.lines().any(|line| {
                let trimmed = line.trim();
                trimmed == dist_entry || trimmed == "dist/"
            }),
            Err(_) => true,
        }
    } else {
        // No .gitignore - only create one if we're in a git repo
        !project_dir.join(".git").exists()
    };

    if !needs_update {
        return;
    }

    // Don't create .gitignore if not in a git repo
    if !gitignore_path.exists() && !project_dir.join(".git").exists() {
        return;
    }

    // Append dist to gitignore
    let entry = if gitignore_path.exists() {
        let content = fs::read_to_string(&gitignore_path).unwrap_or_default();
        if content.ends_with('\n') || content.is_empty() {
            format!("{}\n", dist_entry)
        } else {
            format!("\n{}\n", dist_entry)
        }
    } else {
        format!("{}\n", dist_entry)
    };

    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&gitignore_path)
    {
        let _ = file.write_all(entry.as_bytes());
        status!(
            "   {} Added {} to .gitignore",
            "OK".green().bold(),
            dist_entry
        );
    }
}

/// Run Vite production build if configured.
///
/// Returns Ok(true) if Vite build ran successfully, Ok(false) if no Vite config found.
pub async fn maybe_run_vite_build(project_dir: &Path) -> Result<bool> {
    if !has_vite_config(project_dir) {
        return Ok(false);
    }

    // Validate setup before attempting to build
    check_pnpm_available()?;
    validate_package_json(project_dir, "build")?;

    status!(
        "   {} Vite production build in {}",
        "Running".blue().bold(),
        project_dir.display()
    );

    // Run pnpm install first
    let install_status = Command::new("pnpm")
        .arg("install")
        .current_dir(project_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .await
        .wrap_err("Failed to run pnpm install")?;

    if !install_status.success() {
        eyre::bail!("pnpm install failed");
    }

    // Run pnpm build
    let build_status = Command::new("pnpm")
        .arg("run")
        .arg("build")
        .current_dir(project_dir)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .wrap_err("Failed to run pnpm build")?;

    if !build_status.success() {
        eyre::bail!("Vite build failed");
    }

    status!(
        "   {} Vite production build complete",
        "OK".green().bold()
    );

    ensure_dist_gitignored(project_dir);

    Ok(true)
}

/// Start Vite dev server if configured, and register port with Host.
///
/// Returns the ViteServer handle (which keeps the process alive) or None.
/// The port is registered with Host::get().provide_vite_port() regardless.
pub async fn maybe_start_vite(project_dir: &Path) -> Option<ViteServer> {
    if !has_vite_config(project_dir) {
        crate::host::Host::get().provide_vite_port(None);
        return None;
    }

    match ViteServer::start(project_dir).await {
        Ok(server) => {
            crate::host::Host::get().provide_vite_port(Some(server.port));
            ensure_dist_gitignored(project_dir);
            Some(server)
        }
        Err(e) => {
            tracing::warn!("Failed to start Vite dev server: {}", e);
            crate::host::Host::get().provide_vite_port(None);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_vite_port() {
        // Plain output
        assert_eq!(
            extract_vite_port("  ➜  Local:   http://localhost:5173/"),
            Some(5173)
        );

        // With different formatting
        assert_eq!(
            extract_vite_port("Local: http://localhost:3000/"),
            Some(3000)
        );

        // 127.0.0.1 variant
        assert_eq!(
            extract_vite_port("  ➜  Local:   http://127.0.0.1:5174/"),
            Some(5174)
        );

        // No match
        assert_eq!(extract_vite_port("Some other output"), None);
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi_escapes("\x1b[32m➜\x1b[0m  Local"), "➜  Local");
    }
}
