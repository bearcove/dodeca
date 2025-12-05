use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;
use color_eyre::{Result, eyre::eyre};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use serde::{Deserialize, Serialize};
use crate::types::SourcePath;

/// Configuration for code sample execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeExecutionConfig {
    /// Enable code sample execution
    pub enabled: bool,
    /// Fail build on execution errors (vs just warnings in dev)
    pub fail_on_error: bool,
    /// Timeout for code execution (seconds)
    pub timeout_secs: u64,
    /// Languages to execute (empty = all supported)
    pub languages: Vec<String>,
    /// Per-language configuration
    pub language_config: HashMap<String, LanguageConfig>,
}

impl Default for CodeExecutionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fail_on_error: true,
            timeout_secs: 30,
            languages: vec!["rust".to_string(), "bash".to_string(), "javascript".to_string()],
            language_config: HashMap::from([
                ("rust".to_string(), LanguageConfig::rust()),
                ("bash".to_string(), LanguageConfig::bash()),
                ("javascript".to_string(), LanguageConfig::javascript()),
            ]),
        }
    }
}

/// Per-language execution configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageConfig {
    /// Command to run for this language
    pub command: String,
    /// Arguments to pass to the command
    pub args: Vec<String>,
    /// File extension for temporary files
    pub extension: String,
    /// Prepare code before execution (e.g., add main function)
    pub prepare_code: bool,
}

impl LanguageConfig {
    fn rust() -> Self {
        Self {
            command: "rustc".to_string(),
            args: vec!["--edition".to_string(), "2021".to_string(), "-o".to_string()],
            extension: "rs".to_string(),
            prepare_code: true,
        }
    }

    fn bash() -> Self {
        Self {
            command: "bash".to_string(),
            args: vec![],
            extension: "sh".to_string(),
            prepare_code: false,
        }
    }

    fn javascript() -> Self {
        Self {
            command: "node".to_string(),
            args: vec![],
            extension: "js".to_string(),
            prepare_code: false,
        }
    }
}

/// A code sample extracted from markdown
#[derive(Debug, Clone)]
pub struct CodeSample {
    /// The source file this came from
    pub source_path: SourcePath,
    /// Line number in the source file
    pub line: usize,
    /// Programming language
    pub language: String,
    /// The raw code content
    pub code: String,
    /// Whether this sample should be executed
    pub executable: bool,
}

/// Result of executing a code sample
#[derive(Debug, Clone)]
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

/// Extract code samples from markdown content
pub fn extract_code_samples(source_path: &SourcePath, content: &str) -> Vec<CodeSample> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_HEADING_ATTRIBUTES;

    let parser = Parser::new_ext(content, options);
    let mut samples = Vec::new();
    let mut current_line = 1;
    let mut in_code_block = false;
    let mut current_language = String::new();
    let mut current_code = String::new();
    let mut code_start_line = 0;

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                if let CodeBlockKind::Fenced(lang) = kind {
                    current_language = lang.to_string();
                    in_code_block = true;
                    code_start_line = current_line;
                    current_code.clear();
                }
            }
            Event::End(Tag::CodeBlock(_)) => {
                if in_code_block {
                    // Check if this code block should be executed
                    let executable = should_execute(&current_language);
                    
                    samples.push(CodeSample {
                        source_path: source_path.clone(),
                        line: code_start_line,
                        language: current_language.clone(),
                        code: current_code.clone(),
                        executable,
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

    samples
}

/// Determine if a code block should be executed based on language
fn should_execute(language: &str) -> bool {
    let supported_langs = ["rust", "rs", "bash", "sh", "javascript", "js"];
    supported_langs.contains(&language.to_lowercase().as_str())
}

/// Execute a code sample
pub fn execute_code_sample(
    sample: &CodeSample,
    config: &CodeExecutionConfig,
) -> Result<ExecutionResult> {
    if !sample.executable {
        return Ok(ExecutionResult {
            success: true,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: 0,
            error: None,
        });
    }

    // Check if this language is enabled
    if !config.languages.is_empty() && !config.languages.contains(&sample.language) {
        return Ok(ExecutionResult {
            success: true,
            exit_code: Some(0),
            stdout: format!("Skipped execution for language: {}", sample.language),
            stderr: String::new(),
            duration_ms: 0,
            error: None,
        });
    }

    let lang_config = config.language_config.get(&sample.language)
        .ok_or_else(|| eyre!("No configuration for language: {}", sample.language))?;

    let start_time = std::time::Instant::now();

    // Prepare the code if needed
    let prepared_code = if lang_config.prepare_code {
        prepare_code_for_language(&sample.language, &sample.code)?
    } else {
        sample.code.clone()
    };

    // Create temporary file
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(format!("dodeca_sample_{}.{}", 
        std::process::id(), 
        lang_config.extension
    ));

    // Write code to temporary file
    std::fs::write(&temp_file, &prepared_code)
        .map_err(|e| eyre!("Failed to write temporary file: {}", e))?;

    let result = match sample.language.as_str() {
        "rust" | "rs" => execute_rust_code(&temp_file, lang_config, config.timeout_secs),
        "bash" | "sh" => execute_script(&temp_file, lang_config, config.timeout_secs),
        "javascript" | "js" => execute_script(&temp_file, lang_config, config.timeout_secs),
        _ => Err(eyre!("Unsupported language: {}", sample.language)),
    };

    // Clean up temporary files
    let _ = std::fs::remove_file(&temp_file);
    if sample.language == "rust" || sample.language == "rs" {
        let executable_path = temp_file.with_extension("");
        let _ = std::fs::remove_file(&executable_path);
    }

    let duration = start_time.elapsed();

    match result {
        Ok(output) => {
            let success = output.status.success();
            Ok(ExecutionResult {
                success,
                exit_code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                duration_ms: duration.as_millis() as u64,
                error: if success { None } else { 
                    Some(format!("Process exited with code: {:?}", output.status.code())) 
                },
            })
        }
        Err(e) => Ok(ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: duration.as_millis() as u64,
            error: Some(e.to_string()),
        }),
    }
}

/// Prepare code for execution by adding necessary boilerplate
fn prepare_code_for_language(language: &str, code: &str) -> Result<String> {
    match language {
        "rust" | "rs" => {
            // Check if code already has a main function
            if code.contains("fn main(") {
                Ok(code.to_string())
            } else {
                // Wrap in main function
                Ok(format!(
                    r#"fn main() {{
{}}}
"#,
                    indent_lines(code, "    ")
                ))
            }
        }
        _ => Ok(code.to_string()),
    }
}

/// Indent all lines of a string
fn indent_lines(text: &str, indent: &str) -> String {
    text.lines()
        .map(|line| if line.trim().is_empty() { line.to_string() } else { format!("{}{}", indent, line) })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Execute Rust code by compiling and running
fn execute_rust_code(
    source_file: &Path,
    config: &LanguageConfig,
    timeout_secs: u64,
) -> Result<Output> {
    let executable_path = source_file.with_extension("");
    
    // Compile
    let mut compile_cmd = Command::new(&config.command);
    compile_cmd.args(&config.args);
    compile_cmd.arg(&executable_path);
    compile_cmd.arg(source_file);
    
    let compile_output = compile_cmd.output()?;
    if !compile_output.status.success() {
        return Ok(compile_output);
    }

    // Run the compiled binary
    let mut run_cmd = Command::new(&executable_path);
    run_cmd.stdout(Stdio::piped());
    run_cmd.stderr(Stdio::piped());

    execute_with_timeout(&mut run_cmd, timeout_secs)
}

/// Execute a script file
fn execute_script(
    script_file: &Path,
    config: &LanguageConfig,
    timeout_secs: u64,
) -> Result<Output> {
    let mut cmd = Command::new(&config.command);
    cmd.args(&config.args);
    cmd.arg(script_file);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    execute_with_timeout(&mut cmd, timeout_secs)
}

/// Execute a command with timeout
fn execute_with_timeout(cmd: &mut Command, timeout_secs: u64) -> Result<Output> {
    use std::thread;

    let mut child = cmd.spawn()?;
    
    let timeout = Duration::from_secs(timeout_secs);
    
    // Wait for completion with timeout
    match thread::sleep(timeout) {
        // Sleep completed, check if process is still running
        _ => {
            match child.try_wait()? {
                Some(status) => {
                    // Process finished, get output
                    let output = child.wait_with_output()?;
                    Ok(output)
                }
                None => {
                    // Process still running, kill it
                    child.kill()?;
                    return Err(eyre!("Process timed out after {} seconds", timeout_secs));
                }
            }
        }
    }
}

/// Execute all code samples from a source file
pub fn execute_samples_from_source(
    source_path: &SourcePath,
    content: &str,
    config: &CodeExecutionConfig,
) -> Vec<(CodeSample, ExecutionResult)> {
    let samples = extract_code_samples(source_path, content);
    let mut results = Vec::new();

    for sample in samples {
        if config.enabled && sample.executable {
            match execute_code_sample(&sample, config) {
                Ok(result) => results.push((sample, result)),
                Err(e) => results.push((sample, ExecutionResult {
                    success: false,
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration_ms: 0,
                    error: Some(e.to_string()),
                })),
            }
        } else {
            // Skip execution
            results.push((sample, ExecutionResult {
                success: true,
                exit_code: Some(0),
                stdout: "Skipped".to_string(),
                stderr: String::new(),
                duration_ms: 0,
                error: None,
            }));
        }
    }

    results
}

/// Format execution results for display
pub fn format_execution_error(sample: &CodeSample, result: &ExecutionResult) -> String {
    format!(
        "Code sample execution failed in {}:{}\n\
         Language: {}\n\
         Error: {}\n\
         --- Code ---\n{}\n\
         --- Stderr ---\n{}\n\
         --- Stdout ---\n{}",
        sample.source_path.as_str(),
        sample.line,
        sample.language,
        result.error.as_deref().unwrap_or("Unknown error"),
        sample.code,
        result.stderr,
        result.stdout
    )
}