//! Build tasks for dodeca
//!
//! Usage:
//!   cargo xtask build [--release]
//!   cargo xtask run [--release] [-- <ddc args>]
//!   cargo xtask wasm

use std::env;
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
