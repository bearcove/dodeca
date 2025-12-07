//! Code sample execution plugin for dodeca
//!
//! This plugin extracts and executes code samples from markdown content
//! to ensure they work correctly during the build process.

use facet::Facet;
use plugcard::{PlugResult, plugcard};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use std::collections::HashMap;
use std::fs;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::Duration;

// Re-export shared config types
pub use dodeca_code_execution_config::{
    CodeExecutionConfig as KdlCodeExecutionConfig,
    DependencySpec, DependenciesConfig, RustConfig,
    default_rust_dependencies,
};

plugcard::export_plugin!();

/// Runtime configuration for code sample execution (used by the plugin)
#[derive(Facet, Debug, Clone, PartialEq, Eq)]
pub struct CodeExecutionConfig {
    /// Enable code sample execution
    pub enabled: bool,
    /// Fail build on execution errors (vs just warnings in dev)
    pub fail_on_error: bool,
    /// Timeout for code execution (seconds)
    pub timeout_secs: u64,
    /// Cache directory for execution results (relative to project root)
    pub cache_dir: String,
    /// Project root directory (for resolving path dependencies)
    pub project_root: Option<String>,
    /// Languages to execute (empty = all supported)
    pub languages: Vec<String>,
    /// Dependencies available to all code samples
    pub dependencies: Vec<DependencySpec>,
    /// Per-language configuration
    pub language_config: HashMap<String, LanguageConfig>,
}

impl Default for CodeExecutionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fail_on_error: true,
            timeout_secs: 30,
            cache_dir: ".cache/code-execution".to_string(),
            project_root: None,
            languages: vec!["rust".to_string()],
            dependencies: default_rust_dependencies(),
            language_config: HashMap::from([("rust".to_string(), LanguageConfig::rust())]),
        }
    }
}

impl CodeExecutionConfig {
    /// Create from KDL config, applying defaults for unspecified values
    pub fn from_kdl_config(kdl: &KdlCodeExecutionConfig) -> Self {
        Self::from_kdl_config_with_root(kdl, None)
    }

    /// Create from KDL config with a project root for resolving path dependencies
    pub fn from_kdl_config_with_root(kdl: &KdlCodeExecutionConfig, project_root: Option<String>) -> Self {
        let defaults = Self::default();

        // Use user-specified deps if any, otherwise use defaults
        let dependencies = if kdl.dependencies.deps.is_empty() {
            defaults.dependencies
        } else {
            kdl.dependencies.deps.clone()
        };

        // Build language config from rust settings
        let mut language_config = HashMap::new();
        let rust_config = LanguageConfig {
            command: kdl.rust.command.clone().unwrap_or_else(|| "cargo".to_string()),
            args: kdl.rust.args.clone().unwrap_or_else(|| vec!["run".to_string(), "--release".to_string()]),
            extension: kdl.rust.extension.clone().unwrap_or_else(|| "rs".to_string()),
            prepare_code: kdl.rust.prepare_code.unwrap_or(true),
            auto_imports: kdl.rust.auto_imports.clone().unwrap_or_else(|| {
                vec![
                    "use std::collections::HashMap;".to_string(),
                    "use facet::Facet;".to_string(),
                ]
            }),
            show_output: kdl.rust.show_output.unwrap_or(true),
            expected_compile_errors: vec![],
        };
        language_config.insert("rust".to_string(), rust_config);

        Self {
            enabled: kdl.enabled.unwrap_or(true),
            fail_on_error: kdl.fail_on_error.unwrap_or(true),
            timeout_secs: kdl.timeout_secs.unwrap_or(30),
            cache_dir: kdl.cache_dir.clone().unwrap_or_else(|| ".cache/code-execution".to_string()),
            project_root,
            languages: vec!["rust".to_string()],
            dependencies,
            language_config,
        }
    }
}

/// Per-language execution configuration
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LanguageConfig {
    /// Command to run for this language
    pub command: String,
    /// Arguments to pass to the command
    pub args: Vec<String>,
    /// File extension for temporary files
    pub extension: String,
    /// Prepare code before execution (e.g., add main function)
    pub prepare_code: bool,
    /// Auto-imports to add to every code sample
    pub auto_imports: Vec<String>,
    /// Show output even on success
    pub show_output: bool,
    /// Expected compilation errors (regex patterns)
    pub expected_compile_errors: Vec<String>,
}

impl LanguageConfig {
    fn rust() -> Self {
        Self {
            command: "cargo".to_string(),
            args: vec!["run".to_string(), "--release".to_string()],
            extension: "rs".to_string(),
            prepare_code: true,
            auto_imports: vec![
                "use std::collections::HashMap;".to_string(),
                "use facet::Facet;".to_string(),
            ],
            show_output: true,
            expected_compile_errors: vec![],
        }
    }
}

/// A code sample extracted from markdown
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CodeSample {
    /// The source file this came from
    pub source_path: String,
    /// Line number in the source file
    pub line: usize,
    /// Programming language
    pub language: String,
    /// The raw code content
    pub code: String,
    /// Whether this sample should be executed
    pub executable: bool,
    /// Expected compilation errors (from code block metadata)
    pub expected_errors: Vec<String>,
}

/// Result of executing a code sample
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExecutionResult {
    /// Success status
    pub success: bool,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Execution duration
    pub duration_ms: u64,
    /// Error message if execution failed
    pub error: Option<String>,
    /// Build metadata for reproducibility
    pub metadata: Option<BuildMetadata>,
}

/// Build metadata captured for reproducibility
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct BuildMetadata {
    /// Rust compiler version (from `rustc --version --verbose`)
    pub rustc_version: String,
    /// Cargo version (from `cargo --version`)
    pub cargo_version: String,
    /// Target triple (e.g., "x86_64-unknown-linux-gnu")
    pub target: String,
    /// Build timestamp (ISO 8601 format)
    pub timestamp: String,
    /// Whether shared target cache was used (vs fresh build)
    pub cache_hit: bool,
    /// Platform (e.g., "linux", "macos", "windows")
    pub platform: String,
    /// CPU architecture (e.g., "x86_64", "aarch64")
    pub arch: String,
    /// Dependencies with exact resolved versions (from Cargo.lock)
    pub dependencies: Vec<ResolvedDependency>,
}

/// A resolved dependency with exact version info
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedDependency {
    /// Crate name
    pub name: String,
    /// Exact version
    pub version: String,
    /// Source of the dependency
    pub source: DependencySource,
}

/// Source of a resolved dependency
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DependencySource {
    /// crates.io registry
    CratesIo,
    /// Git repository with commit hash
    Git { url: String, commit: String },
    /// Local path
    Path { path: String },
}

/// Input for extracting code samples
#[derive(Facet)]
pub struct ExtractSamplesInput {
    /// Source file path
    pub source_path: String,
    /// Markdown content
    pub content: String,
}

/// Output from extracting code samples
#[derive(Facet)]
pub struct ExtractSamplesOutput {
    /// Extracted code samples
    pub samples: Vec<CodeSample>,
}

/// Input for executing code samples
#[derive(Facet)]
pub struct ExecuteSamplesInput {
    /// Code samples to execute
    pub samples: Vec<CodeSample>,
    /// Execution configuration
    pub config: CodeExecutionConfig,
}

/// Output from executing code samples
#[derive(Facet)]
pub struct ExecuteSamplesOutput {
    /// Execution results
    pub results: Vec<(CodeSample, ExecutionResult)>,
}

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
    let has_rust_samples = input.samples.iter().any(|s| {
        s.executable && (s.language == "rust" || s.language == "rs")
    });

    // Prepare the shared target directory with all deps pre-compiled
    // All samples will share this target dir via CARGO_TARGET_DIR
    let shared_target_info = if has_rust_samples {
        match prepare_rust_shared_target(&input.config.dependencies, project_root, &cache_dir, input.config.timeout_secs) {
            Ok(info) => Some(info),
            Err(e) => {
                eprintln!("Warning: Failed to prepare shared target: {}", e);
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
    "-Zbuild-dir-new-layout",  // New layout that enables artifact sharing across projects
    "-Zchecksum-freshness",    // Use checksums instead of mtimes for freshness
    "-Zbinary-dep-depinfo",    // Track binary deps (proc-macros) for proper rebuilds
    "-Zgc",                    // Enable garbage collection for cargo cache
    "-Zno-index-update",       // Skip registry index updates (deps already cached)
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
        eprintln!("=== Using cached shared target (deps already compiled) ===");
        // Capture metadata for cached build (cache_hit = true)
        let metadata = capture_build_metadata(cache_dir, true);
        return Ok(SharedTargetInfo {
            path: shared_target_dir,
            metadata,
        });
    }

    eprintln!("=== Preparing shared target (compiling dependencies) ===");

    // Create base project directory
    fs::create_dir_all(&base_project_dir)
        .map_err(|e| format!("Failed to create base project dir: {}", e))?;

    // Write Cargo.toml
    fs::write(&cargo_toml_path, &cargo_toml_content)
        .map_err(|e| format!("Failed to write base Cargo.toml: {}", e))?;

    // Create minimal src/main.rs that references all deps to ensure they're compiled
    let src_dir = base_project_dir.join("src");
    fs::create_dir_all(&src_dir)
        .map_err(|e| format!("Failed to create base src dir: {}", e))?;

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

    eprintln!("=== Building shared target dependencies ===");

    let output = execute_with_timeout_capture(&mut cmd, timeout_secs)
        .map_err(|e| format!("Failed to build base project: {}", e))?;

    // Print captured output
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        eprintln!("{}", stdout);
    }
    if !stderr.is_empty() {
        eprintln!("{}", stderr);
    }

    if !output.status.success() {
        return Err(format!("Base project build failed with code: {:?}", output.status.code()));
    }

    // Capture metadata for fresh build (cache_hit = false)
    let metadata = capture_build_metadata(cache_dir, false);

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

    // Print header with full command so user knows what's being executed
    eprintln!(
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
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Print captured output for visibility
    if !stdout.is_empty() {
        eprintln!("{}", stdout);
    }
    if !stderr.is_empty() {
        eprintln!("{}", stderr);
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
    };

    // Clean up the project dir (but not shared target - that's the cache!)
    let _ = fs::remove_dir_all(&project_dir);
    result
}

/// Generate Cargo.toml with dependencies
fn generate_cargo_toml(dependencies: &[DependencySpec], project_root: Option<&std::path::Path>) -> String {
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
fn should_execute(language: &str) -> bool {
    matches!(language.to_lowercase().as_str(), "rust" | "rs")
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
fn execute_with_timeout_capture(
    cmd: &mut Command,
    timeout_secs: u64,
) -> Result<Output, String> {
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

                return Ok(Output { status, stdout, stderr });
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
                deps.push(ResolvedDependency { name, version, source });
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
        deps.push(ResolvedDependency { name, version, source });
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
            let url = url_part
                .split('?')
                .next()
                .unwrap_or(url_part)
                .to_string();

            DependencySource::Git { url, commit }
        }
        Some(s) if s.starts_with("path+") => {
            let path = s.strip_prefix("path+").unwrap_or(&s).to_string();
            DependencySource::Path { path }
        }
        Some(s) => {
            // Unknown source format, treat as crates.io
            eprintln!("Unknown dependency source format: {}", s);
            DependencySource::CratesIo
        }
    }
}

/// Capture complete build metadata
fn capture_build_metadata(
    cache_dir: &std::path::Path,
    cache_hit: bool,
) -> BuildMetadata {
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
