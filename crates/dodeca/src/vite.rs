//! Vite dev server management
//!
//! Uses the vite cell to spawn and manage Vite dev server processes.

use cell_vite_proto::{RunBuildResult, StartDevServerResult};
use eyre::Result;
use owo_colors::OwoColorize;
use std::fs;
use std::io::Write;
use std::path::Path;

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

/// Information about a running Vite dev server
pub struct ViteServer {
    /// The port Vite is listening on
    pub port: u16,
}

impl ViteServer {
    /// Start a Vite dev server in the given directory via the vite cell
    pub async fn start(project_dir: &Path) -> Result<Self> {
        status!(
            "   {} Vite dev server in {}",
            "Starting".blue().bold(),
            project_dir.display()
        );

        let client = crate::cells::vite_cell()
            .await
            .ok_or_else(|| eyre::eyre!("Vite cell not available"))?;

        let result = client
            .start_dev_server(project_dir.to_string_lossy().to_string())
            .await;

        match result {
            Ok(StartDevServerResult::Success { port }) => {
                status!(
                    "   {} Vite dev server running on port {}",
                    "OK".green().bold(),
                    port
                );
                Ok(ViteServer { port })
            }
            Ok(StartDevServerResult::Error { message }) => {
                Err(eyre::eyre!("Failed to start Vite: {}", message))
            }
            Err(e) => Err(eyre::eyre!("RPC error starting Vite: {:?}", e)),
        }
    }
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

    let needs_update = if gitignore_path.exists() {
        match fs::read_to_string(&gitignore_path) {
            Ok(content) => !content.lines().any(|line| {
                let trimmed = line.trim();
                trimmed == dist_entry || trimmed == "dist/"
            }),
            Err(_) => true,
        }
    } else {
        !project_dir.join(".git").exists()
    };

    if !needs_update {
        return;
    }

    if !gitignore_path.exists() && !project_dir.join(".git").exists() {
        return;
    }

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

/// Run Vite production build if configured via the vite cell.
///
/// Returns Ok(true) if Vite build ran successfully, Ok(false) if no Vite config found.
pub async fn maybe_run_vite_build(project_dir: &Path) -> Result<bool> {
    if !has_vite_config(project_dir) {
        return Ok(false);
    }

    status!(
        "   {} Vite production build in {}",
        "Running".blue().bold(),
        project_dir.display()
    );

    let client = crate::cells::vite_cell()
        .await
        .ok_or_else(|| eyre::eyre!("Vite cell not available"))?;

    let result = client
        .run_build(project_dir.to_string_lossy().to_string())
        .await;

    match result {
        Ok(RunBuildResult::Success) => {
            status!("   {} Vite production build complete", "OK".green().bold());
            ensure_dist_gitignored(project_dir);
            Ok(true)
        }
        Ok(RunBuildResult::Error { message }) => Err(eyre::eyre!("Vite build failed: {}", message)),
        Err(e) => Err(eyre::eyre!("RPC error running Vite build: {:?}", e)),
    }
}

/// Start Vite dev server if configured, and register port with Host.
///
/// Returns the ViteServer handle or None.
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
    fn test_has_vite_config() {
        // Just test the function exists and doesn't panic
        let _ = has_vite_config(Path::new("/nonexistent"));
    }
}
