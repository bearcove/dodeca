//! Build tasks for dodeca

mod ci;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::SystemTime;

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

/// CI GitHub command - generate GitHub workflow
#[derive(Facet, Debug)]
struct CiGithubArgs {
    /// Check that generated files are up to date (don't write)
    #[facet(args::named)]
    check: bool,
}

/// CI Forgejo command - generate Forgejo workflow
#[derive(Facet, Debug)]
struct CiForgejoArgs {
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
    /// Generate GitHub workflow
    CiGithub(CiGithubArgs),
    /// Generate Forgejo workflow
    CiForgejo(CiForgejoArgs),
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
        XtaskCommand::CiGithub(args) => {
            let repo_root =
                Utf8PathBuf::from_path_buf(env::current_dir().expect("get current directory"))
                    .expect("current dir is valid UTF-8");
            match ci::generate_github(&repo_root, args.check) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{}: {e}", "error".red().bold());
                    ExitCode::FAILURE
                }
            }
        }
        XtaskCommand::CiForgejo(args) => {
            let repo_root =
                Utf8PathBuf::from_path_buf(env::current_dir().expect("get current directory"))
                    .expect("current dir is valid UTF-8");
            match ci::generate_forgejo(&repo_root, args.check) {
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
    // Build WASM first
    if !build_wasm() {
        return false;
    }

    // Build everything else (dodeca + all cells in workspace)
    eprintln!(
        "Building dodeca + all cells{}...",
        if release { " (release)" } else { "" }
    );

    let mut cmd = Command::new("cargo");
    cmd.arg("build");
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

fn build_wasm() -> bool {
    eprintln!("Building dodeca-devtools WASM...");

    // wasm-pack doesn't respect CARGO_TARGET_DIR by default, so we pass it explicitly
    // This ensures it uses the workspace target/ directory that we cache
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "--package",
            "dodeca-devtools",
            "--verbose",
        ])
        .status();

    match status {
        Ok(s) if s.success() => {
            // Now run wasm-bindgen to generate the JS bindings
            eprintln!("Running wasm-bindgen...");
            let bindgen_status = Command::new("wasm-bindgen")
                .args([
                    "--target",
                    "web",
                    "--out-dir",
                    "crates/dodeca-devtools/pkg",
                    "target/wasm32-unknown-unknown/release/dodeca_devtools.wasm",
                ])
                .status();

            match bindgen_status {
                Ok(s) if s.success() => {
                    eprintln!("WASM build complete");
                    true
                }
                Ok(s) => {
                    eprintln!("wasm-bindgen failed with status: {s}");
                    false
                }
                Err(e) => {
                    eprintln!("Failed to run wasm-bindgen: {e}");
                    eprintln!("Install with: cargo install wasm-bindgen-cli");
                    false
                }
            }
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

    let release_dir = PathBuf::from("target/release");

    // Copy ddc binary (remove first to avoid "text file busy" on Linux)
    let ddc_src = release_dir.join("ddc");
    let ddc_dst = cargo_bin.join("ddc");
    let _ = fs::remove_file(&ddc_dst);
    if let Err(e) = fs::copy(&ddc_src, &ddc_dst) {
        eprintln!("Failed to copy ddc: {e}");
        return false;
    }
    eprintln!("  Installed ddc");

    // Copy all ddc-cell-* binaries
    if let Ok(entries) = fs::read_dir(&release_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            // Copy cell binaries (ddc-cell-*)
            if name.starts_with("ddc-cell-") && !name.contains('.') {
                let dst = cargo_bin.join(name);
                let _ = fs::remove_file(&dst);
                if let Err(e) = fs::copy(&path, &dst) {
                    eprintln!("Failed to copy {name}: {e}");
                    return false;
                }
                eprintln!("  Installed {name}");
            }
        }
    }

    eprintln!("Installation complete!");
    true
}

fn run_integration_tests(no_build: bool, extra_args: &[&str]) -> bool {
    use std::env;

    // Determine which profile to use for integration tests.
    // Priority:
    // 1. DODECA_INTEGRATION_PROFILE env var (for explicit CI control)
    // 2. Auto-detect from xtask's own build profile
    // 3. Default to release
    let release = if let Ok(profile) = env::var("DODECA_INTEGRATION_PROFILE") {
        match profile.as_str() {
            "debug" | "dev" => {
                eprintln!(
                    "Using debug profile (DODECA_INTEGRATION_PROFILE={})",
                    profile
                );
                false
            }
            "release" => {
                eprintln!(
                    "Using release profile (DODECA_INTEGRATION_PROFILE={})",
                    profile
                );
                true
            }
            _ => {
                eprintln!(
                    "{}: invalid DODECA_INTEGRATION_PROFILE={}, expected 'debug' or 'release'",
                    "warning".yellow().bold(),
                    profile
                );
                true
            }
        }
    } else if cfg!(debug_assertions) {
        // xtask itself was built in debug mode, so look for debug integration-tests
        eprintln!("Auto-detected debug profile (xtask built with debug_assertions)");
        false
    } else {
        // xtask was built in release mode
        eprintln!("Auto-detected release profile");
        true
    };

    let target_dir = if release {
        PathBuf::from("target/release")
    } else {
        PathBuf::from("target/debug")
    };
    let integration_bin = target_dir.join("integration-tests");

    if !no_build {
        let profile_name = if release { "release" } else { "debug" };
        eprintln!(
            "Building {} binaries for integration tests...",
            profile_name
        );
        // build_all builds everything: WASM, dodeca, all cells, and integration-tests
        if !build_all(release) {
            return false;
        }
    } else {
        eprintln!("Skipping build (--no-build), assuming binaries are already built");

        // Strictness: refuse to run if the integration test harness looks stale.
        //
        // A stale harness tends to produce misleading failures and wastes time.
        let bin_mtime = match fs::metadata(&integration_bin).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => {
                eprintln!(
                    "{}: integration-tests binary not found at {}",
                    "error".red().bold(),
                    integration_bin.display()
                );
                let build_cmd = if release {
                    "cargo build --release -p integration-tests"
                } else {
                    "cargo build -p integration-tests"
                };
                eprintln!("Rebuild it with: {}", build_cmd);
                return false;
            }
        };

        fn src_mtime(path: &str) -> Option<SystemTime> {
            std::fs::metadata(path).ok()?.modified().ok()
        }

        let watched_sources = [
            "crates/integration-tests/src/harness.rs",
            "crates/integration-tests/src/main.rs",
            "crates/integration-tests/Cargo.toml",
        ];

        for src in watched_sources {
            if let Some(src_mtime) = src_mtime(src)
                && src_mtime > bin_mtime
            {
                eprintln!(
                    "{}: integration-tests binary is older than {}",
                    "error".red().bold(),
                    src
                );
                let build_cmd = if release {
                    "cargo build --release -p integration-tests"
                } else {
                    "cargo build -p integration-tests"
                };
                eprintln!("Rebuild it with: {}", build_cmd);
                return false;
            }
        }
    }

    // Verify integration-tests binary exists
    if !integration_bin.exists() {
        eprintln!(
            "{}: integration-tests binary not found at {}",
            "error".red().bold(),
            integration_bin.display()
        );
        return false;
    }

    // Run the integration-tests binary
    eprintln!("Running integration tests...");

    let mut cmd = Command::new(&integration_bin);
    cmd.args(extra_args);

    // Check if DODECA_BIN and DODECA_CELL_PATH are already set in the environment
    // If so, pass them through. Otherwise, set them to our built binaries.
    let ddc_bin_path = if let Ok(env_bin) = env::var("DODECA_BIN") {
        eprintln!("Using DODECA_BIN from environment: {}", env_bin);
        PathBuf::from(env_bin)
    } else {
        let ddc_bin = target_dir.join("ddc");
        if !ddc_bin.exists() {
            eprintln!(
                "{}: ddc binary not found at {}",
                "error".red().bold(),
                ddc_bin.display()
            );
            eprintln!("Run without --no-build to build it, or set DODECA_BIN environment variable");
            return false;
        }
        ddc_bin
    };

    let cell_path = if let Ok(env_path) = env::var("DODECA_CELL_PATH") {
        eprintln!("Using DODECA_CELL_PATH from environment: {}", env_path);
        PathBuf::from(env_path)
    } else {
        target_dir.clone()
    };

    // Set environment variables for the test harness
    let ddc_bin_abs = ddc_bin_path.canonicalize().unwrap_or(ddc_bin_path);
    let cell_path_abs = cell_path.canonicalize().unwrap_or(cell_path);

    cmd.env("DODECA_BIN", &ddc_bin_abs);
    cmd.env("DODECA_CELL_PATH", &cell_path_abs);

    eprintln!("  DODECA_BIN={}", ddc_bin_abs.display());
    eprintln!("  DODECA_CELL_PATH={}", cell_path_abs.display());

    // Make fixture resolution independent of where `integration-tests` was built.
    // Use runtime path resolution instead of compile-time CARGO_MANIFEST_DIR to support
    // CI environments where binaries are built in temp directories and then moved.
    let fixtures_root = env::current_dir()
        .expect("get current directory")
        .join("crates")
        .join("integration-tests")
        .join("fixtures");
    if !fixtures_root.is_dir() {
        eprintln!(
            "{}: fixtures directory not found at {}",
            "error".red().bold(),
            fixtures_root.display()
        );
        eprintln!(
            "Current directory: {}",
            env::current_dir().unwrap_or_default().display()
        );
        eprintln!("Expected to be run from the repository root");
        return false;
    }
    cmd.env("DODECA_TEST_FIXTURES_DIR", &fixtures_root);
    eprintln!("  DODECA_TEST_FIXTURES_DIR={}", fixtures_root.display());

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
