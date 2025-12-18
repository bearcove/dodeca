//! Build tasks for dodeca

mod ci;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use camino::Utf8PathBuf;
use facet::Facet;
use facet_args as args;
use owo_colors::OwoColorize;

/// Build command - build WASM + plugins + dodeca
#[derive(Facet, Debug)]
struct BuildArgs {
    /// Build in release mode
    #[facet(args::named, args::short = 'r')]
    release: bool,
}

/// Run command - build all, then run ddc
#[derive(Facet, Debug)]
struct RunArgs {
    /// Build in release mode
    #[facet(args::named, args::short = 'r')]
    release: bool,

    /// Arguments to pass to ddc
    #[facet(args::positional, default)]
    ddc_args: Vec<String>,
}

/// Install command - build release & install to ~/.cargo/bin
#[derive(Facet, Debug)]
struct InstallArgs {}

/// WASM command - build WASM only
#[derive(Facet, Debug)]
struct WasmArgs {}

/// CI command - generate release workflow
#[derive(Facet, Debug)]
struct CiArgs {
    /// Check that generated files are up to date (don't write)
    #[facet(args::named)]
    check: bool,
}

/// Generate PowerShell installer
#[derive(Facet, Debug)]
struct GeneratePs1InstallerArgs {
    /// Output path for the installer script
    #[facet(args::positional)]
    output_path: String,
}

/// Integration tests command
#[derive(Facet, Debug)]
struct IntegrationArgs {
    /// Skip building binaries (assume they're already built)
    #[facet(args::named)]
    no_build: bool,

    /// Arguments to pass to integration-tests binary
    #[facet(args::positional, default)]
    extra_args: Vec<String>,
}

#[derive(Facet, Debug)]
#[repr(u8)]
enum XtaskCommand {
    /// Build WASM + plugins + dodeca
    Build(BuildArgs),
    /// Build all, then run ddc
    Run(RunArgs),
    /// Build release & install to ~/.cargo/bin
    Install(InstallArgs),
    /// Build WASM only
    Wasm(WasmArgs),
    /// Generate release workflow
    Ci(CiArgs),
    /// Generate PowerShell installer
    GeneratePs1Installer(GeneratePs1InstallerArgs),
    /// Run integration tests
    Integration(IntegrationArgs),
}

#[derive(Facet, Debug)]
struct XtaskArgs {
    #[facet(args::subcommand)]
    command: XtaskCommand,
}

fn parse_args() -> Result<XtaskCommand, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let parsed: XtaskArgs = facet_args::from_slice(&args_refs).map_err(|e| {
        eprintln!("{:?}", miette::Report::new(e));
        "Failed to parse arguments".to_string()
    })?;

    Ok(parsed.command)
}

fn main() -> ExitCode {
    // Set up miette for nice error formatting
    miette::set_hook(Box::new(|_| {
        Box::new(
            miette::MietteHandlerOpts::new()
                .terminal_links(true)
                .unicode(true)
                .build(),
        )
    }))
    .ok();

    let cmd = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {}", "error".red().bold(), e);
            return ExitCode::FAILURE;
        }
    };

    match cmd {
        XtaskCommand::Build(args) => {
            if !build_all(args.release) {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        XtaskCommand::Run(args) => {
            if !build_all(args.release) {
                return ExitCode::FAILURE;
            }
            let ddc_args: Vec<&str> = args.ddc_args.iter().map(|s| s.as_str()).collect();
            if !run_ddc(args.release, &ddc_args) {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        XtaskCommand::Install(_) => {
            if !install_dev() {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        XtaskCommand::Wasm(_) => {
            if build_wasm() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        XtaskCommand::Ci(args) => {
            let repo_root = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_owned();
            match ci::generate(&repo_root, args.check) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{}: {e}", "error".red().bold());
                    ExitCode::FAILURE
                }
            }
        }
        XtaskCommand::GeneratePs1Installer(args) => {
            let content = ci::generate_powershell_installer();
            if let Err(e) = fs::write(&args.output_path, content) {
                eprintln!(
                    "{}: writing PowerShell installer: {e}",
                    "error".red().bold()
                );
                return ExitCode::FAILURE;
            }
            eprintln!("Generated PowerShell installer: {}", args.output_path);
            ExitCode::SUCCESS
        }
        XtaskCommand::Integration(args) => {
            let extra_args: Vec<&str> = args.extra_args.iter().map(|s| s.as_str()).collect();
            if !run_integration_tests(args.no_build, &extra_args) {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
    }
}

fn build_all(release: bool) -> bool {
    if !build_wasm() {
        return false;
    }
    if !build_cdylib_plugins(release) {
        return false;
    }
    if !build_dodeca_and_rapace_plugins(release) {
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

/// Discover cdylib plugin crates by looking for dodeca-* directories with cdylib crate type
fn discover_cdylib_plugins() -> Vec<String> {
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

/// Discover rapace plugins by looking for mod-* directories with `[[bin]]` in Cargo.toml
/// Returns (package_name, binary_name) pairs
fn discover_rapace_plugins() -> Vec<(String, String)> {
    let cells_dir = PathBuf::from("cells");
    let mut plugins = Vec::new();

    if let Ok(entries) = fs::read_dir(&cells_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip proto crates
            if !name.starts_with("mod-") || name.ends_with("-proto") {
                continue;
            }

            // Check if it has a [[bin]] section
            let cargo_toml = path.join("Cargo.toml");
            if let Ok(content) = fs::read_to_string(&cargo_toml)
                && content.contains("[[bin]]")
            {
                let bin_name = format!("dodeca-{}", name);
                plugins.push((name.to_string(), bin_name));
            }
        }
    }

    plugins.sort();
    plugins
}

fn build_cdylib_plugins(release: bool) -> bool {
    let plugins = discover_cdylib_plugins();
    if plugins.is_empty() {
        eprintln!("No cdylib plugins found to build");
        return true;
    }

    eprintln!(
        "Building {} cdylib plugins{}...",
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
            eprintln!("cdylib plugins built: {}", plugins.join(", "));
            true
        }
        Ok(s) => {
            eprintln!("cdylib plugin build failed with status: {s}");
            false
        }
        Err(e) => {
            eprintln!("Failed to run cargo: {e}");
            false
        }
    }
}

fn build_dodeca_and_rapace_plugins(release: bool) -> bool {
    let rapace_plugins = discover_rapace_plugins();

    eprintln!(
        "Building dodeca + {} rapace plugins{}...",
        rapace_plugins.len(),
        if release { " (release)" } else { "" }
    );

    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--package", "dodeca"]);

    // Add all rapace plugins
    for (pkg, bin) in &rapace_plugins {
        cmd.args(["--package", pkg, "--bin", bin]);
    }

    if release {
        cmd.arg("--release");
    }

    match cmd.status() {
        Ok(s) if s.success() => {
            let bins: Vec<_> = rapace_plugins.iter().map(|(_, b)| b.as_str()).collect();
            eprintln!("Built: ddc, {}", bins.join(", "));
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

    // Copy all rapace plugin binaries
    let rapace_plugins = discover_rapace_plugins();
    for (_, bin_name) in &rapace_plugins {
        let src = PathBuf::from(format!("target/release/{bin_name}"));
        let dst = cargo_bin.join(bin_name);
        if src.exists() {
            let _ = fs::remove_file(&dst);
            if let Err(e) = fs::copy(&src, &dst) {
                eprintln!("Failed to copy {bin_name}: {e}");
                return false;
            }
            eprintln!("  Installed {bin_name}");
        } else {
            eprintln!("  Warning: {bin_name} not found, skipping");
        }
    }

    // Copy cdylib plugins
    let plugin_ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };

    let cdylib_plugins = discover_cdylib_plugins();
    for plugin in &cdylib_plugins {
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

/// Discover cell binaries (cell-* crates that produce binaries)
fn discover_cell_binaries() -> Vec<String> {
    let cells_dir = PathBuf::from("cells");
    let mut bins = Vec::new();

    if let Ok(entries) = fs::read_dir(&cells_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip proto crates
            if !name.starts_with("cell-") || name.ends_with("-proto") {
                continue;
            }

            // Check if it has a src/main.rs (binary crate)
            let main_rs = path.join("src/main.rs");
            if main_rs.exists() {
                bins.push(name.to_string());
            }
        }
    }

    bins.sort();
    bins
}

fn run_integration_tests(no_build: bool, extra_args: &[&str]) -> bool {
    // Always use release mode for integration tests
    let release = true;
    let target_dir = PathBuf::from("target/release");
    let ddc_bin = target_dir.join("ddc");
    let integration_bin = target_dir.join("integration-tests");

    if !no_build {
        eprintln!("Building release binaries for integration tests...");
        if !build_all(release) {
            return false;
        }

        // Build all cell binaries
        let cell_bins = discover_cell_binaries();
        if !cell_bins.is_empty() {
            eprintln!("Building {} cell binaries...", cell_bins.len());
            let mut cmd = Command::new("cargo");
            cmd.arg("build").arg("--release");
            for bin in &cell_bins {
                cmd.args(["-p", bin]);
            }

            match cmd.status() {
                Ok(s) if s.success() => {
                    eprintln!("Cell binaries built: {}", cell_bins.join(", "));
                }
                Ok(s) => {
                    eprintln!("Cell binary build failed with status: {s}");
                    return false;
                }
                Err(e) => {
                    eprintln!("Failed to run cargo: {e}");
                    return false;
                }
            }
        }

        // Build the integration-tests binary
        eprintln!("Building integration-tests binary...");
        let mut cmd = Command::new("cargo");
        cmd.args(["build", "--release", "-p", "integration-tests"]);

        match cmd.status() {
            Ok(s) if s.success() => {
                eprintln!("integration-tests built");
            }
            Ok(s) => {
                eprintln!("integration-tests build failed with status: {s}");
                return false;
            }
            Err(e) => {
                eprintln!("Failed to run cargo: {e}");
                return false;
            }
        }
    } else {
        eprintln!("Skipping build (--no-build), assuming binaries are already built");
    }

    // Verify binaries exist
    if !ddc_bin.exists() {
        eprintln!(
            "{}: ddc binary not found at {}",
            "error".red().bold(),
            ddc_bin.display()
        );
        eprintln!("Run without --no-build to build it, or ensure it was built separately");
        return false;
    }

    if !integration_bin.exists() {
        eprintln!(
            "{}: integration-tests binary not found at {}",
            "error".red().bold(),
            integration_bin.display()
        );
        eprintln!("Run without --no-build to build it, or ensure it was built separately");
        return false;
    }

    // Run the integration-tests binary
    eprintln!("Running integration tests...");

    let mut cmd = Command::new(&integration_bin);
    cmd.args(extra_args);

    // Set environment variables for the test harness
    let ddc_bin_abs = ddc_bin.canonicalize().unwrap_or(ddc_bin);
    let target_dir_abs = target_dir.canonicalize().unwrap_or(target_dir);

    cmd.env("DODECA_BIN", &ddc_bin_abs);
    cmd.env("DODECA_CELL_PATH", &target_dir_abs);

    eprintln!("  DODECA_BIN={}", ddc_bin_abs.display());
    eprintln!("  DODECA_CELL_PATH={}", target_dir_abs.display());

    match cmd.status() {
        Ok(s) if s.success() => {
            eprintln!("Integration tests passed!");
            true
        }
        Ok(s) => {
            eprintln!("Integration tests failed with status: {s}");
            false
        }
        Err(e) => {
            eprintln!("Failed to run integration-tests: {e}");
            false
        }
    }
}
