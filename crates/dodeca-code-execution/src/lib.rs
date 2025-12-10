//! Code sample execution plugin for dodeca
//!
//! This plugin extracts and executes code samples from markdown content
//! to ensure they work correctly during the build process.

use plugcard::{PlugResult, plugcard};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use std::fs;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::Duration;

// Re-export all types from the types crate
pub use dodeca_code_execution_types::*;

plugcard::export_plugin!();

/// Extract code samples from markdown content
#[plugcard]
pub fn extract_code_samples(input: ExtractSamplesInput) -> PlugResult<ExtractSamplesOutput> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_HEADING_ATTRIBUTES;

    let parser = Parser::new_ext(&input.content, options);
    let mut samples = Vec::new();
    let mut current_line = 1;
    let mut in_code_block = false;
    let mut current_language = String::new();
    let mut current_code = String::new();
    let mut code_start_line = 0;

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                current_language = lang.to_string();
                in_code_block = true;
                code_start_line = current_line;
                current_code.clear();
            }
            Event::Start(Tag::CodeBlock(_)) => {}
            Event::End(pulldown_cmark::TagEnd::CodeBlock) => {
                if in_code_block {
                    // Check if this code block should be executed
                    let executable = should_execute(&current_language);

                    samples.push(CodeSample {
                        source_path: input.source_path.clone(),
                        line: code_start_line,
                        language: current_language.clone(),
                        code: current_code.clone(),
                        executable,
                        expected_errors: vec![],
                    });

                    in_code_block = false;
                    current_language.clear();
                    current_code.clear();
                }
            }
            Event::Text(text) => {
                if in_code_block {
                    current_code.push_str(&text);
                }
                // Count newlines for line tracking
                current_line += text.matches('\n').count();
            }
            Event::Code(code) => {
                // Inline code - count newlines
                current_line += code.matches('\n').count();
            }
            Event::SoftBreak | Event::HardBreak => {
                current_line += 1;
            }
            _ => {}
        }
    }

    Ok(ExtractSamplesOutput { samples }).into()
}

/// Execute code samples
#[plugcard]
pub fn execute_code_samples(input: ExecuteSamplesInput) -> PlugResult<ExecuteSamplesOutput> {
    let mut results = Vec::new();

    if !input.config.enabled {
        return PlugResult::Ok(ExecuteSamplesOutput { results });
    }

    // Determine project root for path dependency resolution
    let project_root = input.config.project_root.as_ref().map(std::path::Path::new);

    // Ensure cache directory exists
    let cache_dir = if let Some(root) = project_root {
        root.join(&input.config.cache_dir)
    } else {
        std::path::PathBuf::from(&input.config.cache_dir)
    };

    if let Err(e) = fs::create_dir_all(&cache_dir) {
        return PlugResult::Err(format!("Failed to create cache directory: {}", e));
    }

    // Check if we have any Rust samples to execute
    let has_rust_samples = input
        .samples
        .iter()
        .any(|s| s.executable && (s.language == "rust" || s.language == "rs"));

    // Prepare the shared target directory with all deps pre-compiled
    // All samples will share this target dir via CARGO_TARGET_DIR
    let shared_target_info = if has_rust_samples {
        match prepare_rust_shared_target(
            &input.config.dependencies,
            project_root,
            &cache_dir,
            input.config.timeout_secs,
        ) {
            Ok(info) => Some(info),
            Err(e) => {
                plugcard::log_warn!("Failed to prepare shared target: {}", e);
                None
            }
        }
    } else {
        None
    };

    for sample in input.samples {
        let result = if !sample.executable {
            ExecutionResult {
                success: true,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: 0,
                error: None,
                metadata: None,
                skipped: true,
            }
        } else {
            // Check if this language is enabled
            if !input.config.languages.is_empty()
                && !input.config.languages.contains(&sample.language)
            {
                ExecutionResult {
                    success: true,
                    exit_code: Some(0),
                    stdout: format!("Skipped execution for language: {}", sample.language),
                    stderr: String::new(),
                    duration_ms: 0,
                    error: None,
                    metadata: None,
                    skipped: true,
                }
            } else {
                execute_single_sample(&sample, &input.config, shared_target_info.as_ref())
            }
        };

        results.push((sample, result));
    }

    PlugResult::Ok(ExecuteSamplesOutput { results })
}

/// Nightly cargo flags for shared target support
const NIGHTLY_FLAGS: &[&str] = &[
    "-Zbuild-dir-new-layout", // New layout that enables artifact sharing across projects
    "-Zchecksum-freshness",   // Use checksums instead of mtimes for freshness
    "-Zbinary-dep-depinfo",   // Track binary deps (proc-macros) for proper rebuilds
    "-Zgc",                   // Enable garbage collection for cargo cache
    "-Zno-index-update",      // Skip registry index updates (deps already cached)
];

/// Result of preparing the shared target directory
struct SharedTargetInfo {
    /// Path to the shared target directory
    path: std::path::PathBuf,
    /// Build metadata captured during preparation
    metadata: BuildMetadata,
}

/// Prepare a shared target directory with all dependencies pre-compiled
fn prepare_rust_shared_target(
    dependencies: &[DependencySpec],
    project_root: Option<&std::path::Path>,
    cache_dir: &std::path::Path,
    timeout_secs: u64,
) -> Result<SharedTargetInfo, String> {
    // Canonicalize cache_dir to get absolute paths - this is critical because
    // cargo interprets CARGO_TARGET_DIR relative to the project directory
    let cache_dir = cache_dir
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize cache dir: {}", e))?;
    let base_project_dir = cache_dir.join("base_project");
    let shared_target_dir = cache_dir.join("target");

    // Check if we need to rebuild by comparing Cargo.toml content
    let cargo_toml_content = generate_cargo_toml(dependencies, project_root);
    let cargo_toml_path = base_project_dir.join("Cargo.toml");

    let needs_rebuild = if cargo_toml_path.exists() {
        match fs::read_to_string(&cargo_toml_path) {
            Ok(existing) => existing != cargo_toml_content,
            Err(_) => true,
        }
    } else {
        true
    };

    if !needs_rebuild && shared_target_dir.join("release").exists() {
        plugcard::log_info!("=== Using cached shared target (deps already compiled) ===");
        // Capture metadata for cached build (cache_hit = true)
        let metadata = capture_build_metadata(&cache_dir, true);
        return Ok(SharedTargetInfo {
            path: shared_target_dir,
            metadata,
        });
    }

    plugcard::log_info!("=== Preparing shared target (compiling dependencies) ===");

    // Create base project directory
    fs::create_dir_all(&base_project_dir)
        .map_err(|e| format!("Failed to create base project dir: {}", e))?;

    // Write Cargo.toml
    fs::write(&cargo_toml_path, &cargo_toml_content)
        .map_err(|e| format!("Failed to write base Cargo.toml: {}", e))?;

    // Create minimal src/main.rs that references all deps to ensure they're compiled
    let src_dir = base_project_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| format!("Failed to create base src dir: {}", e))?;

    // Generate main.rs that imports all dependencies to force compilation
    let main_rs = generate_base_main_rs(dependencies);
    fs::write(src_dir.join("main.rs"), main_rs)
        .map_err(|e| format!("Failed to write base main.rs: {}", e))?;

    // Build the base project to compile all dependencies
    // Use nightly cargo with special flags for shared target support
    let mut cmd = Command::new("cargo");
    cmd.args(["+nightly"]);
    cmd.args(NIGHTLY_FLAGS);
    cmd.args(["build", "--release"]);
    cmd.current_dir(&base_project_dir);
    cmd.env("CARGO_TARGET_DIR", &shared_target_dir);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    plugcard::log_info!("=== Building shared target dependencies ===");

    let output = execute_with_timeout_capture(&mut cmd, timeout_secs)
        .map_err(|e| format!("Failed to build base project: {}", e))?;

    // Log captured output
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        plugcard::log_info!("{}", stdout);
    }
    if !stderr.is_empty() {
        plugcard::log_info!("{}", stderr);
    }

    if !output.status.success() {
        return Err(format!(
            "Base project build failed with code: {:?}",
            output.status.code()
        ));
    }

    // Capture metadata for fresh build (cache_hit = false)
    let metadata = capture_build_metadata(&cache_dir, false);

    Ok(SharedTargetInfo {
        path: shared_target_dir,
        metadata,
    })
}

/// Generate a main.rs that imports all dependencies to ensure they're compiled
fn generate_base_main_rs(dependencies: &[DependencySpec]) -> String {
    let mut lines = Vec::new();

    // Add extern crate for each dependency (handles crate name normalization)
    for dep in dependencies {
        // Convert crate name with hyphens to underscores for Rust
        let crate_name = dep.name.replace('-', "_");
        lines.push(format!("use {}; // force compilation", crate_name));
    }

    lines.push(String::new());
    lines.push("fn main() {}".to_string());

    lines.join("\n")
}

/// Execute a single code sample
fn execute_single_sample(
    sample: &CodeSample,
    config: &CodeExecutionConfig,
    shared_target_info: Option<&SharedTargetInfo>,
) -> ExecutionResult {
    let start_time = std::time::Instant::now();

    let lang_config = match config.language_config.get(&sample.language) {
        Some(config) => config,
        None => {
            return ExecutionResult {
                success: false,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: 0,
                error: Some(format!(
                    "No configuration for language: {}",
                    sample.language
                )),
                metadata: None,
                skipped: false,
            };
        }
    };

    // For Rust, create a temporary Cargo project
    if sample.language == "rust" || sample.language == "rs" {
        execute_rust_sample(sample, lang_config, config, shared_target_info, start_time)
    } else {
        ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: start_time.elapsed().as_millis() as u64,
            error: Some(format!("Unsupported language: {}", sample.language)),
            metadata: None,
            skipped: false,
        }
    }
}

/// Execute Rust code sample using Cargo
fn execute_rust_sample(
    sample: &CodeSample,
    lang_config: &LanguageConfig,
    config: &CodeExecutionConfig,
    shared_target_info: Option<&SharedTargetInfo>,
    start_time: std::time::Instant,
) -> ExecutionResult {
    // Determine project root for path dependency resolution
    let project_root = config.project_root.as_ref().map(std::path::Path::new);

    // Extract metadata from shared target info (will be cloned into result)
    let metadata = shared_target_info.map(|info| info.metadata.clone());

    // Create temporary Cargo project
    let temp_dir = std::env::temp_dir();
    let project_name = format!("dodeca_sample_{}", std::process::id());
    let project_dir = temp_dir.join(&project_name);

    if let Err(e) = fs::create_dir_all(&project_dir) {
        return ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: start_time.elapsed().as_millis() as u64,
            error: Some(format!("Failed to create temp project: {}", e)),
            metadata,
            skipped: false,
        };
    }

    // Generate Cargo.toml with resolved path dependencies
    let cargo_toml = generate_cargo_toml(&config.dependencies, project_root);
    if let Err(e) = fs::write(project_dir.join("Cargo.toml"), cargo_toml) {
        let _ = fs::remove_dir_all(&project_dir);
        return ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: start_time.elapsed().as_millis() as u64,
            error: Some(format!("Failed to write Cargo.toml: {}", e)),
            metadata,
            skipped: false,
        };
    }

    // Create src directory and write main.rs
    let src_dir = project_dir.join("src");
    if let Err(e) = fs::create_dir_all(&src_dir) {
        let _ = fs::remove_dir_all(&project_dir);
        return ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: start_time.elapsed().as_millis() as u64,
            error: Some(format!("Failed to create src dir: {}", e)),
            metadata,
            skipped: false,
        };
    }

    // Prepare code with auto-imports
    let prepared_code = if lang_config.prepare_code {
        prepare_rust_code(&sample.code, &lang_config.auto_imports)
    } else {
        sample.code.clone()
    };

    if let Err(e) = fs::write(src_dir.join("main.rs"), prepared_code) {
        let _ = fs::remove_dir_all(&project_dir);
        return ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: start_time.elapsed().as_millis() as u64,
            error: Some(format!("Failed to write main.rs: {}", e)),
            metadata,
            skipped: false,
        };
    }

    // Execute with cargo - capture output for TUI compatibility
    // Use nightly with special flags if we have a shared target dir
    let mut cmd = Command::new(&lang_config.command);
    if shared_target_info.is_some() {
        cmd.args(["+nightly"]);
        cmd.args(NIGHTLY_FLAGS);
    }
    cmd.args(&lang_config.args);
    cmd.current_dir(&project_dir);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // Use shared target directory if available (deps pre-compiled there)
    if let Some(info) = shared_target_info {
        cmd.env("CARGO_TARGET_DIR", &info.path);
    }

    // Log header with full command so user knows what's being executed
    plugcard::log_info!(
        "\n=== Executing {}:{} ===\n$ cd {:?} && {} {}",
        sample.source_path,
        sample.line,
        project_dir,
        lang_config.command,
        lang_config.args.join(" ")
    );

    let output = match execute_with_timeout_capture(&mut cmd, config.timeout_secs) {
        Ok(output) => output,
        Err(e) => {
            let _ = fs::remove_dir_all(&project_dir);
            return ExecutionResult {
                success: false,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: start_time.elapsed().as_millis() as u64,
                error: Some(e),
                metadata,
                skipped: false,
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Log captured output for visibility
    if !stdout.is_empty() {
        plugcard::log_info!("{}", stdout);
    }
    if !stderr.is_empty() {
        plugcard::log_info!("{}", stderr);
    }

    let success = output.status.success();

    let result = ExecutionResult {
        success,
        exit_code: output.status.code(),
        stdout,
        stderr,
        duration_ms: start_time.elapsed().as_millis() as u64,
        error: if success {
            None
        } else {
            Some(format!(
                "Process exited with code: {:?}",
                output.status.code()
            ))
        },
        metadata,
        skipped: false,
    };

    // Clean up the project dir (but not shared target - that's the cache!)
    let _ = fs::remove_dir_all(&project_dir);
    result
}

/// Generate Cargo.toml with dependencies
fn generate_cargo_toml(
    dependencies: &[DependencySpec],
    project_root: Option<&std::path::Path>,
) -> String {
    let mut lines = vec![
        "[package]".to_string(),
        "name = \"dodeca-code-sample\"".to_string(),
        "version = \"0.1.0\"".to_string(),
        "edition = \"2021\"".to_string(),
        "".to_string(),
        "# Prevent this from being part of any parent workspace".to_string(),
        "[workspace]".to_string(),
        "".to_string(),
        "[dependencies]".to_string(),
    ];

    for dep in dependencies {
        lines.push(dep.to_cargo_toml_line_with_root(project_root));
    }

    lines.join("\n")
}

/// Prepare Rust code with auto-imports and main function
fn prepare_rust_code(code: &str, auto_imports: &[String]) -> String {
    let mut result = String::new();

    // Add auto-imports
    for import in auto_imports {
        result.push_str(import);
        result.push('\n');
    }

    if !auto_imports.is_empty() {
        result.push('\n');
    }

    // Check if code already has a main function
    if code.contains("fn main(") {
        result.push_str(code);
    } else {
        result.push_str("fn main() {\n");
        for line in code.lines() {
            result.push_str("    ");
            result.push_str(line);
            result.push('\n');
        }
        result.push_str("}\n");
    }

    result
}

/// Determine if a code block should be executed based on language
/// Handles comma-separated attributes like "rust,noexec" or "rust,skip"
fn should_execute(language: &str) -> bool {
    // Extract the base language (before any comma)
    let base_language = language.split(',').next().unwrap_or("").trim();

    // Check if base language is Rust
    if !matches!(base_language.to_lowercase().as_str(), "rust" | "rs") {
        return false;
    }

    // Check if any attributes indicate skipping
    let attributes = language.split(',').skip(1).map(|s| s.trim());
    for attr in attributes {
        if matches!(attr.to_lowercase().as_str(), "noexec" | "skip" | "no-verify") {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_execute_basic_rust() {
        assert!(should_execute("rust"));
        assert!(should_execute("rs"));
    }

    #[test]
    fn test_should_execute_with_noexec() {
        assert!(!should_execute("rust,noexec"));
        assert!(!should_execute("rust, noexec")); // with space
        assert!(!should_execute("rs,noexec"));
    }

    #[test]
    fn test_should_execute_with_skip() {
        assert!(!should_execute("rust,skip"));
        assert!(!should_execute("rust, skip")); // with space
    }

    #[test]
    fn test_should_execute_with_no_verify() {
        assert!(!should_execute("rust,no-verify"));
        assert!(!should_execute("rust, no-verify")); // with space
    }

    #[test]
    fn test_should_execute_non_rust_languages() {
        assert!(!should_execute("python"));
        assert!(!should_execute("javascript"));
        assert!(!should_execute(""));
    }

    #[test]
    fn test_should_execute_case_insensitive() {
        assert!(should_execute("Rust"));
        assert!(should_execute("RUST"));
        assert!(!should_execute("rust,NOEXEC"));
        assert!(!should_execute("RUST,NoExec"));
    }

    #[test]
    fn test_should_execute_multiple_attributes() {
        // Only the skip/noexec attribute matters
        assert!(!should_execute("rust,noexec,other"));
        assert!(should_execute("rust,other")); // other non-skip attributes don't affect execution
    }
}

/// Execute a command with timeout (captures output)
#[allow(dead_code)] // May be useful later for captured mode
fn _execute_with_timeout(cmd: &mut Command, timeout_secs: u64) -> Result<Output, String> {
    use std::time::Instant;

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start process: {}", e))?;

    let timeout = Duration::from_secs(timeout_secs);
    let start = Instant::now();

    // Poll for completion with timeout
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                // Process finished, get output
                return child
                    .wait_with_output()
                    .map_err(|e| format!("Failed to get process output: {}", e));
            }
            Ok(None) => {
                // Process still running, check timeout
                if start.elapsed() > timeout {
                    if let Err(e) = child.kill() {
                        return Err(format!("Failed to kill process: {}", e));
                    }
                    return Err(format!("Process timed out after {} seconds", timeout_secs));
                }
                // Wait a bit before checking again
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("Failed to check process status: {}", e)),
        }
    }
}

/// Execute a command with timeout (piped streams, returns Output with stdout/stderr)
fn execute_with_timeout_capture(cmd: &mut Command, timeout_secs: u64) -> Result<Output, String> {
    use std::io::Read;
    use std::time::Instant;

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start process: {}", e))?;

    let timeout = Duration::from_secs(timeout_secs);
    let start = Instant::now();

    // Poll for completion with timeout
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Process finished, collect output
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();

                if let Some(mut stdout_pipe) = child.stdout.take() {
                    let _ = stdout_pipe.read_to_end(&mut stdout);
                }
                if let Some(mut stderr_pipe) = child.stderr.take() {
                    let _ = stderr_pipe.read_to_end(&mut stderr);
                }

                return Ok(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                // Process still running, check timeout
                if start.elapsed() > timeout {
                    if let Err(e) = child.kill() {
                        return Err(format!("Failed to kill process: {}", e));
                    }
                    return Err(format!("Process timed out after {} seconds", timeout_secs));
                }
                // Wait a bit before checking again
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("Failed to check process status: {}", e)),
        }
    }
}

// ============================================================================
// Build Metadata Capture
// ============================================================================

/// Capture rustc version information
fn capture_rustc_version() -> String {
    Command::new("rustc")
        .args(["--version", "--verbose"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_else(|e| format!("Failed to get rustc version: {}", e))
}

/// Capture cargo version information
fn capture_cargo_version() -> String {
    Command::new("cargo")
        .args(["--version"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|e| format!("Failed to get cargo version: {}", e))
}

/// Extract target triple from rustc verbose output
fn extract_target_from_rustc(rustc_version: &str) -> String {
    for line in rustc_version.lines() {
        if let Some(target) = line.strip_prefix("host: ") {
            return target.trim().to_string();
        }
    }
    // Fallback: try to detect from current platform
    format!(
        "{}-unknown-{}-gnu",
        std::env::consts::ARCH,
        std::env::consts::OS
    )
}

/// Get current timestamp in ISO 8601 format
fn get_iso_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    // Format as ISO 8601 (basic format without external deps)
    let secs = duration.as_secs();
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;

    // Calculate year, month, day (simplified, assumes no leap seconds)
    let mut year = 1970;
    let mut days_remaining = days_since_epoch as i64;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days_remaining < days_in_year {
            break;
        }
        days_remaining -= days_in_year;
        year += 1;
    }

    let mut month = 1;
    let days_in_months = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    for days_in_month in days_in_months {
        if days_remaining < days_in_month as i64 {
            break;
        }
        days_remaining -= days_in_month as i64;
        month += 1;
    }

    let day = days_remaining + 1;
    let hour = time_of_day / 3600;
    let minute = (time_of_day % 3600) / 60;
    let second = time_of_day % 60;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Parse Cargo.lock to extract resolved dependencies
fn parse_cargo_lock(lock_path: &std::path::Path) -> Vec<ResolvedDependency> {
    let content = match fs::read_to_string(lock_path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut deps = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_version: Option<String> = None;
    let mut current_source: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();

        if line == "[[package]]" {
            // Save previous package if complete
            if let (Some(name), Some(version)) = (current_name.take(), current_version.take()) {
                let source = parse_dependency_source(current_source.take());
                deps.push(ResolvedDependency {
                    name,
                    version,
                    source,
                });
            }
            current_source = None;
        } else if let Some(name) = line.strip_prefix("name = ") {
            current_name = Some(unquote(name));
        } else if let Some(version) = line.strip_prefix("version = ") {
            current_version = Some(unquote(version));
        } else if let Some(source) = line.strip_prefix("source = ") {
            current_source = Some(unquote(source));
        }
    }

    // Don't forget the last package
    if let (Some(name), Some(version)) = (current_name, current_version) {
        let source = parse_dependency_source(current_source);
        deps.push(ResolvedDependency {
            name,
            version,
            source,
        });
    }

    deps
}

/// Remove quotes from a TOML string value
fn unquote(s: &str) -> String {
    s.trim_matches('"').to_string()
}

/// Parse a Cargo.lock source string into DependencySource
fn parse_dependency_source(source: Option<String>) -> DependencySource {
    match source {
        None => DependencySource::CratesIo,
        Some(s) if s.starts_with("registry+") => DependencySource::CratesIo,
        Some(s) if s.starts_with("git+") => {
            // Format: git+https://github.com/user/repo?branch=main#commitsha
            // or: git+https://github.com/user/repo#commitsha
            let without_prefix = s.strip_prefix("git+").unwrap_or(&s);

            let (url_part, commit) = if let Some(hash_pos) = without_prefix.rfind('#') {
                let (url, commit) = without_prefix.split_at(hash_pos);
                (url, commit.trim_start_matches('#').to_string())
            } else {
                (without_prefix, "unknown".to_string())
            };

            // Remove query params (branch, rev, etc.) from URL
            let url = url_part.split('?').next().unwrap_or(url_part).to_string();

            DependencySource::Git { url, commit }
        }
        Some(s) if s.starts_with("path+") => {
            let path = s.strip_prefix("path+").unwrap_or(&s).to_string();
            DependencySource::Path { path }
        }
        Some(s) => {
            // Unknown source format, treat as crates.io
            plugcard::log_warn!("Unknown dependency source format: {}", s);
            DependencySource::CratesIo
        }
    }
}

/// Capture complete build metadata
fn capture_build_metadata(cache_dir: &std::path::Path, cache_hit: bool) -> BuildMetadata {
    let rustc_version = capture_rustc_version();
    let cargo_version = capture_cargo_version();
    let target = extract_target_from_rustc(&rustc_version);

    // Parse Cargo.lock from the base project
    let lock_path = cache_dir.join("base_project").join("Cargo.lock");
    let dependencies = parse_cargo_lock(&lock_path);

    BuildMetadata {
        rustc_version,
        cargo_version,
        target,
        timestamp: get_iso_timestamp(),
        cache_hit,
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        dependencies,
    }
}
