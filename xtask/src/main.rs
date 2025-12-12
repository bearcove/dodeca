//! Build tasks for dodeca
//!
//! Usage:
//!   cargo xtask build [--release]
//!   cargo xtask run [--release] [-- <ddc args>]
//!   cargo xtask install
//!   cargo xtask wasm
//!   cargo xtask ci [--check]

mod ci;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use camino::Utf8PathBuf;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    match args.first().map(|s| s.as_str()) {
        Some("build") => {
            let release = args.iter().any(|a| a == "--release" || a == "-r");
            if !build_all(release) {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Some("run") => {
            let release = args.iter().any(|a| a == "--release" || a == "-r");
            // Find args after "--" to pass to ddc
            let ddc_args: Vec<&str> = args
                .iter()
                .skip_while(|a| *a != "--")
                .skip(1)
                .map(|s| s.as_str())
                .collect();

            if !build_all(release) {
                return ExitCode::FAILURE;
            }
            if !run_ddc(release, &ddc_args) {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Some("install") => {
            if !install_dev() {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Some("wasm") => {
            if build_wasm() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Some("ci") => {
            let check = args.iter().any(|a| a == "--check");
            let repo_root = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_owned();
            match ci::generate(&repo_root, check) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("Error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("generate-ps1-installer") => {
            // Generate PowerShell installer to specified path
            if let Some(output_path) = args.get(1) {
                let content = ci::generate_powershell_installer();
                if let Err(e) = fs::write(output_path, content) {
                    eprintln!("Error writing PowerShell installer: {e}");
                    return ExitCode::FAILURE;
                }
                eprintln!("Generated PowerShell installer: {output_path}");
                ExitCode::SUCCESS
            } else {
                eprintln!("Usage: cargo xtask generate-ps1-installer <output-path>");
                ExitCode::FAILURE
            }
        }
        _ => {
            eprintln!("Usage:");
            eprintln!("  cargo xtask build [--release]        Build WASM + plugins + dodeca");
            eprintln!("  cargo xtask run [--release] [-- ..]  Build all, then run ddc");
            eprintln!(
                "  cargo xtask install                  Build release & install to ~/.cargo/bin"
            );
            eprintln!("  cargo xtask wasm                     Build WASM only");
            eprintln!("  cargo xtask ci [--check]             Generate release workflow");
            ExitCode::FAILURE
        }
    }
}

fn build_all(release: bool) -> bool {
    if !build_wasm() {
        return false;
    }
    if !build_plugins(release) {
        return false;
    }
    if !build_dodeca(release) {
        return false;
    }
    true
}

fn build_wasm() -> bool {
    eprintln!("Building dodeca-devtools WASM...");

    let status = Command::new("wasm-pack")
        .args(["build", "--target", "web", "crates/dodeca-devtools"])
        .status();

    match status {
        Ok(s) if s.success() => {
            eprintln!("WASM build complete");
            true
        }
        Ok(s) => {
            eprintln!("wasm-pack failed with status: {s}");
            false
        }
        Err(e) => {
            eprintln!("Failed to run wasm-pack: {e}");
            eprintln!("Install with: cargo install wasm-pack");
            false
        }
    }
}

/// Discover plugin crates by looking for dodeca-* directories with cdylib crate type
fn discover_plugins() -> Vec<String> {
    let crates_dir = PathBuf::from("crates");
    let mut plugins = Vec::new();

    if let Ok(entries) = fs::read_dir(&crates_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with("dodeca-") {
                continue;
            }

            // Check if it's a cdylib (plugin)
            let cargo_toml = path.join("Cargo.toml");
            if let Ok(content) = fs::read_to_string(&cargo_toml)
                && content.contains("cdylib")
            {
                plugins.push(name.to_string());
            }
        }
    }

    plugins.sort();
    plugins
}

fn build_plugins(release: bool) -> bool {
    let plugins = discover_plugins();
    if plugins.is_empty() {
        eprintln!("No plugins found to build");
        return true;
    }

    eprintln!(
        "Building {} plugins{}...",
        plugins.len(),
        if release { " (release)" } else { "" }
    );

    let mut cmd = Command::new("cargo");
    cmd.arg("build");
    for plugin in &plugins {
        cmd.args(["-p", plugin]);
    }
    if release {
        cmd.arg("--release");
    }

    match cmd.status() {
        Ok(s) if s.success() => {
            eprintln!("Plugins built: {}", plugins.join(", "));
            true
        }
        Ok(s) => {
            eprintln!("Plugin build failed with status: {s}");
            false
        }
        Err(e) => {
            eprintln!("Failed to run cargo: {e}");
            false
        }
    }
}

fn build_dodeca(release: bool) -> bool {
    eprintln!(
        "Building dodeca + dodeca-mod-http{}...",
        if release { " (release)" } else { "" }
    );

    let mut cmd = Command::new("cargo");
    cmd.args([
        "build",
        "--package",
        "dodeca",
        "--package",
        "mod-http",
        "--bin",
        "dodeca-mod-http",
    ]);
    if release {
        cmd.arg("--release");
    }

    match cmd.status() {
        Ok(s) if s.success() => {
            eprintln!("Build complete");
            true
        }
        Ok(s) => {
            eprintln!("cargo build failed with status: {s}");
            false
        }
        Err(e) => {
            eprintln!("Failed to run cargo: {e}");
            false
        }
    }
}

fn run_ddc(release: bool, args: &[&str]) -> bool {
    let binary = if release {
        "target/release/ddc"
    } else {
        "target/debug/ddc"
    };

    eprintln!("Running: {} {}", binary, args.join(" "));

    let mut cmd = Command::new(binary);
    cmd.args(args);

    match cmd.status() {
        Ok(s) if s.success() => true,
        Ok(s) => {
            eprintln!("ddc exited with status: {s}");
            false
        }
        Err(e) => {
            eprintln!("Failed to run ddc: {e}");
            false
        }
    }
}

fn install_dev() -> bool {
    // Build everything in release mode
    if !build_all(true) {
        return false;
    }

    // Get cargo bin directory
    let cargo_bin = match env::var("CARGO_HOME") {
        Ok(home) => PathBuf::from(home).join("bin"),
        Err(_) => {
            if let Ok(home) = env::var("HOME") {
                PathBuf::from(home).join(".cargo").join("bin")
            } else {
                eprintln!("Could not determine cargo bin directory");
                return false;
            }
        }
    };

    if !cargo_bin.exists() {
        eprintln!(
            "Cargo bin directory does not exist: {}",
            cargo_bin.display()
        );
        return false;
    }

    eprintln!("Installing to {}...", cargo_bin.display());

    // Copy ddc binary (remove first to avoid "text file busy" on Linux)
    let ddc_src = PathBuf::from("target/release/ddc");
    let ddc_dst = cargo_bin.join("ddc");
    let _ = fs::remove_file(&ddc_dst); // Ignore error if file doesn't exist
    if let Err(e) = fs::copy(&ddc_src, &ddc_dst) {
        eprintln!("Failed to copy ddc: {e}");
        return false;
    }
    eprintln!("  Installed ddc");

    // Copy dodeca-mod-http binary
    let mod_http_src = PathBuf::from("target/release/dodeca-mod-http");
    let mod_http_dst = cargo_bin.join("dodeca-mod-http");
    let _ = fs::remove_file(&mod_http_dst);
    if let Err(e) = fs::copy(&mod_http_src, &mod_http_dst) {
        eprintln!("Failed to copy dodeca-mod-http: {e}");
        return false;
    }
    eprintln!("  Installed dodeca-mod-http");

    // Copy plugins
    let plugin_ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };

    // Discover and install all plugins
    let plugins = discover_plugins();
    for plugin in &plugins {
        // Convert crate name (dodeca-webp) to lib name (libdodeca_webp)
        let lib_name = format!("lib{}", plugin.replace('-', "_"));
        let src = PathBuf::from(format!("target/release/{lib_name}.{plugin_ext}"));
        let dst = cargo_bin.join(format!("{lib_name}.{plugin_ext}"));
        if src.exists() {
            let _ = fs::remove_file(&dst); // Remove first to avoid "text file busy"
            if let Err(e) = fs::copy(&src, &dst) {
                eprintln!("Failed to copy {lib_name}: {e}");
                return false;
            }
            eprintln!("  Installed {lib_name}.{plugin_ext}");
        } else {
            eprintln!("  Warning: {lib_name}.{plugin_ext} not found, skipping");
        }
    }

    eprintln!("Installation complete!");
    true
}
