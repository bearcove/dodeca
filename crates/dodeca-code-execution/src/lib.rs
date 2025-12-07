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
    /// Cache directory for execution results
    pub cache_dir: String,
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
            languages: vec!["rust".to_string()],
            dependencies: default_rust_dependencies(),
            language_config: HashMap::from([("rust".to_string(), LanguageConfig::rust())]),
        }
    }
}

impl CodeExecutionConfig {
    /// Create from KDL config, applying defaults for unspecified values
    pub fn from_kdl_config(kdl: &KdlCodeExecutionConfig) -> Self {
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
            args: kdl.rust.args.clone().unwrap_or_else(|| vec!["run".to_string()]),
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
            args: vec!["run".to_string()],
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

    // Ensure cache directory exists
    if let Err(e) = fs::create_dir_all(&input.config.cache_dir) {
        return PlugResult::Err(format!("Failed to create cache directory: {}", e));
    }

    for sample in input.samples {
        let result = if !sample.executable {
            ExecutionResult {
                success: true,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: 0,
                error: None,
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
                }
            } else {
                execute_single_sample(&sample, &input.config)
            }
        };

        results.push((sample, result));
    }

    PlugResult::Ok(ExecuteSamplesOutput { results })
}

/// Execute a single code sample
fn execute_single_sample(sample: &CodeSample, config: &CodeExecutionConfig) -> ExecutionResult {
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
            };
        }
    };

    // For Rust, create a temporary Cargo project
    if sample.language == "rust" || sample.language == "rs" {
        execute_rust_sample(sample, lang_config, config, start_time)
    } else {
        ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: start_time.elapsed().as_millis() as u64,
            error: Some(format!("Unsupported language: {}", sample.language)),
        }
    }
}

/// Execute Rust code sample using Cargo
fn execute_rust_sample(
    sample: &CodeSample,
    lang_config: &LanguageConfig,
    config: &CodeExecutionConfig,
    start_time: std::time::Instant,
) -> ExecutionResult {
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
        };
    }

    // Generate Cargo.toml
    let cargo_toml = generate_cargo_toml(&config.dependencies);
    if let Err(e) = fs::write(project_dir.join("Cargo.toml"), cargo_toml) {
        return ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: start_time.elapsed().as_millis() as u64,
            error: Some(format!("Failed to write Cargo.toml: {}", e)),
        };
    }

    // Create src directory and write main.rs
    let src_dir = project_dir.join("src");
    if let Err(e) = fs::create_dir_all(&src_dir) {
        return ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: start_time.elapsed().as_millis() as u64,
            error: Some(format!("Failed to create src dir: {}", e)),
        };
    }

    // Prepare code with auto-imports
    let prepared_code = if lang_config.prepare_code {
        prepare_rust_code(&sample.code, &lang_config.auto_imports)
    } else {
        sample.code.clone()
    };

    if let Err(e) = fs::write(src_dir.join("main.rs"), prepared_code) {
        return ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: start_time.elapsed().as_millis() as u64,
            error: Some(format!("Failed to write main.rs: {}", e)),
        };
    }

    // Execute with cargo - inherit streams so output is visible during build
    let mut cmd = Command::new(&lang_config.command);
    cmd.args(&lang_config.args);
    cmd.current_dir(&project_dir);
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    // Print header with full command so user knows what's being executed
    eprintln!(
        "\n=== Executing {}:{} ===\n$ cd {:?} && {} {}",
        sample.source_path,
        sample.line,
        project_dir,
        lang_config.command,
        lang_config.args.join(" ")
    );

    let status = match execute_with_timeout_inherit(&mut cmd, config.timeout_secs) {
        Ok(status) => status,
        Err(e) => {
            // Clean up
            let _ = fs::remove_dir_all(&project_dir);
            return ExecutionResult {
                success: false,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: start_time.elapsed().as_millis() as u64,
                error: Some(e),
            };
        }
    };

    let success = status.success();

    // With inherited streams, we can't check expected_errors against stderr
    // For now, just use the exit status
    let final_success = success;

    let result = ExecutionResult {
        success: final_success,
        exit_code: status.code(),
        stdout: String::new(), // Not captured with inherit
        stderr: String::new(), // Not captured with inherit
        duration_ms: start_time.elapsed().as_millis() as u64,
        error: if final_success {
            None
        } else {
            Some(format!(
                "Process exited with code: {:?}",
                status.code()
            ))
        },
    };

    // Clean up
    let _ = fs::remove_dir_all(&project_dir);
    result
}

/// Generate Cargo.toml with dependencies
fn generate_cargo_toml(dependencies: &[DependencySpec]) -> String {
    let mut lines = vec![
        "[package]".to_string(),
        "name = \"dodeca-code-sample\"".to_string(),
        "version = \"0.1.0\"".to_string(),
        "edition = \"2021\"".to_string(),
        "".to_string(),
        "[dependencies]".to_string(),
    ];

    for dep in dependencies {
        lines.push(dep.to_cargo_toml_line());
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

/// Execute a command with timeout (inherited streams, returns ExitStatus)
fn execute_with_timeout_inherit(
    cmd: &mut Command,
    timeout_secs: u64,
) -> Result<std::process::ExitStatus, String> {
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
                return Ok(status);
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
