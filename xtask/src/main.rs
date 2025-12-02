//! Build tasks for dodeca
//!
//! Usage:
//!   cargo xtask build [--release]
//!   cargo xtask run [--release] [-- <ddc args>]
//!   cargo xtask install
//!   cargo xtask wasm

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

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
        _ => {
            eprintln!("Usage:");
            eprintln!("  cargo xtask build [--release]        Build WASM + plugins + dodeca");
            eprintln!("  cargo xtask run [--release] [-- ..]  Build all, then run ddc");
            eprintln!("  cargo xtask install                  Build release & install to ~/.cargo/bin");
            eprintln!("  cargo xtask wasm                     Build WASM only");
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
    eprintln!("Building livereload-client WASM...");

    let status = Command::new("wasm-pack")
        .args(["build", "--target", "web", "crates/livereload-client"])
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

fn build_plugins(release: bool) -> bool {
    eprintln!(
        "Building plugins{}...",
        if release { " (release)" } else { "" }
    );

    let mut cmd = Command::new("cargo");
    cmd.args(["build", "-p", "dodeca-webp", "-p", "dodeca-jxl"]);
    if release {
        cmd.arg("--release");
    }

    match cmd.status() {
        Ok(s) if s.success() => {
            eprintln!("Plugins built");
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
        "Building dodeca{}...",
        if release { " (release)" } else { "" }
    );

    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--package", "dodeca"]);
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
            if let Some(home) = env::var("HOME").ok() {
                PathBuf::from(home).join(".cargo").join("bin")
            } else {
                eprintln!("Could not determine cargo bin directory");
                return false;
            }
        }
    };

    if !cargo_bin.exists() {
        eprintln!("Cargo bin directory does not exist: {}", cargo_bin.display());
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

    // Copy plugins
    let plugin_ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };

    let plugins = ["libdodeca_webp", "libdodeca_jxl"];
    for plugin in plugins {
        let src = PathBuf::from(format!("target/release/{plugin}.{plugin_ext}"));
        let dst = cargo_bin.join(format!("{plugin}.{plugin_ext}"));
        if src.exists() {
            let _ = fs::remove_file(&dst); // Remove first to avoid "text file busy"
            if let Err(e) = fs::copy(&src, &dst) {
                eprintln!("Failed to copy {plugin}: {e}");
                return false;
            }
            eprintln!("  Installed {plugin}.{plugin_ext}");
        } else {
            eprintln!("  Warning: {plugin}.{plugin_ext} not found, skipping");
        }
    }

    eprintln!("Installation complete!");
    true
}
