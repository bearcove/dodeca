//! Dodeca vite cell (cell-vite)
//!
//! Manages Vite dev server and production builds.

use cell_vite_proto::{RunBuildResult, StartDevServerResult, ViteManager, ViteManagerDispatcher};
use dodeca_cell_runtime::run_cell;
use eyre::WrapErr;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// Vite manager implementation
#[derive(Clone)]
pub struct ViteManagerImpl;

impl ViteManager for ViteManagerImpl {
    async fn start_dev_server(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        project_dir: String,
    ) -> StartDevServerResult {
        match start_dev_server_inner(Path::new(&project_dir)).await {
            Ok(port) => StartDevServerResult::Success { port },
            Err(e) => StartDevServerResult::Error {
                message: format!("{:#}", e),
            },
        }
    }

    async fn run_build(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        project_dir: String,
    ) -> RunBuildResult {
        match run_build_inner(Path::new(&project_dir)).await {
            Ok(()) => RunBuildResult::Success,
            Err(e) => RunBuildResult::Error {
                message: format!("{:#}", e),
            },
        }
    }
}

/// Check if pnpm is available in PATH
fn check_pnpm_available() -> eyre::Result<()> {
    match std::process::Command::new("pnpm")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => eyre::bail!("pnpm is installed but returned an error"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eyre::bail!("pnpm is not installed")
        }
        Err(e) => eyre::bail!("Failed to run pnpm: {}", e),
    }
}

/// Validate package.json exists and has required scripts
fn validate_package_json(project_dir: &Path, script: &str) -> eyre::Result<()> {
    let package_json_path = project_dir.join("package.json");

    if !package_json_path.exists() {
        eyre::bail!("No package.json found in {}", project_dir.display());
    }

    let content = std::fs::read_to_string(&package_json_path)
        .wrap_err_with(|| format!("Failed to read {}", package_json_path.display()))?;

    let script_pattern = format!("\"{}\"", script);
    if !content.contains(&script_pattern) {
        eyre::bail!("package.json missing '{}' script", script);
    }

    Ok(())
}

/// Start a Vite dev server
async fn start_dev_server_inner(project_dir: &Path) -> eyre::Result<u16> {
    check_pnpm_available()?;
    validate_package_json(project_dir, "dev")?;

    // Run pnpm install
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

    // Channel to receive the port
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

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");

    let tx_clone = tx.clone();
    tokio::spawn(async move {
        relay_output_for_port(stdout, tx_clone).await;
    });

    tokio::spawn(async move {
        relay_output_for_port(stderr, tx).await;
    });

    // Keep child alive by spawning a task that holds it
    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    // Wait for port with timeout
    let port = tokio::time::timeout(std::time::Duration::from_secs(30), rx.recv())
        .await
        .wrap_err("Timeout waiting for Vite to start")?
        .ok_or_else(|| eyre::eyre!("Vite process exited before reporting port"))?;

    Ok(port)
}

/// Run a Vite production build
async fn run_build_inner(project_dir: &Path) -> eyre::Result<()> {
    check_pnpm_available()?;
    validate_package_json(project_dir, "build")?;

    // Run pnpm install
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

    Ok(())
}

/// Read lines from a reader and extract the Vite port
async fn relay_output_for_port<R: tokio::io::AsyncRead + Unpin>(reader: R, tx: mpsc::Sender<u16>) {
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(port) = extract_vite_port(&line) {
            let _ = tx.send(port).await;
        }
    }
}

/// Extract the port from a Vite server output line
fn extract_vite_port(line: &str) -> Option<u16> {
    let stripped = strip_ansi_escapes(line);

    for pattern in &["http://localhost:", "http://127.0.0.1:"] {
        if let Some(idx) = stripped.find(pattern) {
            let after_pattern = &stripped[idx + pattern.len()..];
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
            if chars.peek() == Some(&'[') {
                chars.next();
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("vite", |_handle| ViteManagerDispatcher::new(
        ViteManagerImpl
    ))
}
