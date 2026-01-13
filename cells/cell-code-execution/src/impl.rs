use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

use cell_code_execution_proto::*;

/// Code executor implementation
#[derive(Clone)]
pub struct CodeExecutorImpl;

impl CodeExecutor for CodeExecutorImpl {
    async fn extract_code_samples(&self, input: ExtractSamplesInput) -> CodeExecutionResult {
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
                Event::End(pulldown_cmark::TagEnd::CodeBlock) => {
                    if in_code_block {
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

        CodeExecutionResult::ExtractSuccess {
            output: ExtractSamplesOutput { samples },
        }
    }

    async fn execute_code_samples(&self, input: ExecuteSamplesInput) -> CodeExecutionResult {
        let mut results = Vec::new();

        if !input.config.is_enabled() {
            return CodeExecutionResult::ExecuteSuccess {
                output: ExecuteSamplesOutput { results },
            };
        }

        // Simplified execution logic
        for sample in input.samples {
            let result = if !sample.executable {
                ExecutionResult {
                    status: ExecutionStatus::Skipped,
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration_ms: 0,
                    error: None,
                    metadata: None,
                }
            } else {
                execute_code_sample(&sample, &input.config).await
            };
            results.push((sample, result));
        }

        CodeExecutionResult::ExecuteSuccess {
            output: ExecuteSamplesOutput { results },
        }
    }
}

/// Parse info string into base language and attributes.
/// e.g., "rust,test" -> ("rust", ["test"])
fn parse_info_string(info: &str) -> (&str, Vec<&str>) {
    let mut parts = info.split(',');
    let lang = parts.next().unwrap_or("");
    let attrs: Vec<&str> = parts.collect();
    (lang, attrs)
}

fn should_execute(language: &str) -> bool {
    // Disable all code execution with DODECA_NO_CODE_EXEC=1
    if std::env::var("DODECA_NO_CODE_EXEC").is_ok() {
        return false;
    }

    let (lang, attrs) = parse_info_string(language);

    // Must be a Rust code block
    if !matches!(lang.to_lowercase().as_str(), "rust" | "rs") {
        return false;
    }

    // Rust code execution is opt-in: requires the `test` attribute
    attrs.contains(&"test")
}

/// Progress reporting interval
const PROGRESS_INTERVAL_SECS: u64 = 15;

/// Maximum output size (10MB)
const MAX_OUTPUT_SIZE: usize = 10 * 1024 * 1024;

/// Execution timeout (5 minutes)
const EXECUTION_TIMEOUT_SECS: u64 = 300;

/// Check if we're inside a ddc build (reentrancy guard)
fn is_reentrant_build() -> bool {
    std::env::var("DODECA_BUILD_ACTIVE").is_ok()
}

async fn execute_code_sample(
    sample: &CodeSample,
    _config: &CodeExecutionConfig,
) -> ExecutionResult {
    use tokio::io::AsyncReadExt;

    // Reentrancy guard: refuse to execute if we're inside a ddc build
    if is_reentrant_build() {
        tracing::warn!(
            "[code-exec] BLOCKED: refusing to execute code inside ddc build (reentrancy guard) - {}:{}",
            sample.source_path,
            sample.line
        );
        return ExecutionResult {
            status: ExecutionStatus::Skipped,
            exit_code: None,
            stdout: String::new(),
            stderr: "Code execution blocked: cannot run code samples during ddc build (reentrancy guard)".to_string(),
            duration_ms: 0,
            error: Some("Reentrancy guard: code execution disabled during ddc build".to_string()),
            metadata: None,
        };
    }

    let start_time = std::time::Instant::now();
    let source_info = format!("{}:{}", sample.source_path, sample.line);

    // Only Rust with `test` attribute is supported
    let (lang, _attrs) = parse_info_string(&sample.language);
    if !matches!(lang.to_lowercase().as_str(), "rust" | "rs") {
        return ExecutionResult {
            status: ExecutionStatus::Skipped,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("Unsupported language: {}", sample.language),
            duration_ms: 0,
            error: Some(format!("Unsupported language: {}", sample.language)),
            metadata: None,
        };
    }

    // Create a temporary directory for the Rust project
    let temp_dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(e) => {
            return ExecutionResult {
                status: ExecutionStatus::Failed,
                exit_code: None,
                stdout: String::new(),
                stderr: format!("Failed to create temp directory: {}", e),
                duration_ms: start_time
                    .elapsed()
                    .as_millis()
                    .try_into()
                    .unwrap_or(u64::MAX),
                error: Some(format!("Failed to create temp directory: {}", e)),
                metadata: None,
            };
        }
    };

    let project_dir = temp_dir.path();

    // Write Cargo.toml
    let cargo_toml = r#"[package]
name = "code-sample"
version = "0.1.0"
edition = "2021"

[dependencies]
"#;

    if let Err(e) = std::fs::write(project_dir.join("Cargo.toml"), cargo_toml) {
        return ExecutionResult {
            status: ExecutionStatus::Failed,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("Failed to write Cargo.toml: {}", e),
            duration_ms: start_time
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            error: Some(format!("Failed to write Cargo.toml: {}", e)),
            metadata: None,
        };
    }

    // Create src directory
    let src_dir = project_dir.join("src");
    if let Err(e) = std::fs::create_dir(&src_dir) {
        return ExecutionResult {
            status: ExecutionStatus::Failed,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("Failed to create src directory: {}", e),
            duration_ms: start_time
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            error: Some(format!("Failed to create src directory: {}", e)),
            metadata: None,
        };
    }

    // Process hidden lines (doctest-style # prefix) for compilation
    let code = prepare_code_for_execution(&sample.code);

    // Determine if code needs to be wrapped in main()
    let main_code = if code.contains("fn main()") {
        code
    } else {
        format!("fn main() {{\n{}\n}}", code)
    };

    // Write main.rs
    if let Err(e) = std::fs::write(src_dir.join("main.rs"), &main_code) {
        return ExecutionResult {
            status: ExecutionStatus::Failed,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("Failed to write main.rs: {}", e),
            duration_ms: start_time
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            error: Some(format!("Failed to write main.rs: {}", e)),
            metadata: None,
        };
    }

    let command = "cargo";
    let args = ["run", "--release"];

    // Reuse build artifacts across executions to make repeated code samples much faster (especially
    // in `ddc serve` + the integration test suite). This is intentionally opt-in overridable.
    let shared_target_dir: PathBuf = std::env::var_os("DDC_CODE_EXEC_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("dodeca-code-exec-target"));
    if let Err(e) = std::fs::create_dir_all(&shared_target_dir) {
        tracing::warn!("Failed to create shared target dir: {e}");
    }

    tracing::debug!(
        "[code-exec] Starting: {} {} ({})",
        command,
        args.join(" "),
        source_info
    );

    // Spawn the process with piped stdout/stderr in the temp project directory
    let mut child = match Command::new(command)
        .args(args)
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", &shared_target_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return ExecutionResult {
                status: ExecutionStatus::Failed,
                exit_code: None,
                stdout: String::new(),
                stderr: format!("Failed to execute {}: {}", command, e),
                duration_ms: 0,
                error: Some(format!("Failed to execute {}: {}", command, e)),
                metadata: None,
            };
        }
    };

    let mut stdout_handle = child.stdout.take().unwrap();
    let mut stderr_handle = child.stderr.take().unwrap();

    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    let mut last_output_time = std::time::Instant::now();
    let mut last_progress_report = std::time::Instant::now();
    let mut stdout_eof = false;
    let mut stderr_eof = false;

    // Read output with progress reporting and timeout
    let timeout = std::time::Duration::from_secs(EXECUTION_TIMEOUT_SECS);
    let progress_interval = std::time::Duration::from_secs(PROGRESS_INTERVAL_SECS);

    loop {
        let elapsed = start_time.elapsed();

        // Check timeout
        if elapsed > timeout {
            let _ = child.kill().await;
            tracing::warn!(
                "[code-exec] TIMEOUT after {}s: {} ({})",
                elapsed.as_secs(),
                command,
                source_info
            );
            return ExecutionResult {
                status: ExecutionStatus::Failed,
                exit_code: None,
                stdout: String::from_utf8_lossy(&stdout_buf).to_string(),
                stderr: String::from_utf8_lossy(&stderr_buf).to_string(),
                duration_ms: elapsed.as_millis().try_into().unwrap_or(u64::MAX),
                error: Some(format!(
                    "Execution timed out after {}s",
                    EXECUTION_TIMEOUT_SECS
                )),
                metadata: None,
            };
        }

        // Progress report every PROGRESS_INTERVAL_SECS
        if last_progress_report.elapsed() >= progress_interval {
            let since_output = last_output_time.elapsed().as_secs();
            tracing::debug!(
                "[code-exec] Running {}s, stdout={}B, stderr={}B, last_output={}s ago: {} ({})",
                elapsed.as_secs(),
                stdout_buf.len(),
                stderr_buf.len(),
                since_output,
                command,
                source_info
            );
            last_progress_report = std::time::Instant::now();
        }

        // Check output size limits
        if stdout_buf.len() + stderr_buf.len() > MAX_OUTPUT_SIZE {
            let _ = child.kill().await;
            tracing::warn!(
                "[code-exec] OUTPUT TOO LARGE ({}B): {} ({})",
                stdout_buf.len() + stderr_buf.len(),
                command,
                source_info
            );
            return ExecutionResult {
                status: ExecutionStatus::Failed,
                exit_code: None,
                stdout: String::from_utf8_lossy(&stdout_buf).to_string(),
                stderr: String::from_utf8_lossy(&stderr_buf).to_string(),
                duration_ms: elapsed.as_millis().try_into().unwrap_or(u64::MAX),
                error: Some(format!(
                    "Output exceeded {}MB limit",
                    MAX_OUTPUT_SIZE / 1024 / 1024
                )),
                metadata: None,
            };
        }

        // Try to read some output (non-blocking with short timeout)
        let mut stdout_tmp = [0u8; 4096];
        let mut stderr_tmp = [0u8; 4096];
        tokio::select! {
            // NOTE: once a pipe hits EOF, further reads resolve immediately with Ok(0).
            // If we keep selecting on it, it can starve `child.wait()` and hang forever.
            result = stdout_handle.read(&mut stdout_tmp), if !stdout_eof => {
                match result {
                    Ok(0) => stdout_eof = true,
                    Ok(n) => {
                        stdout_buf.extend_from_slice(&stdout_tmp[..n]);
                        last_output_time = std::time::Instant::now();
                    }
                    Err(_) => {}
                }
            }
            result = stderr_handle.read(&mut stderr_tmp), if !stderr_eof => {
                match result {
                    Ok(0) => stderr_eof = true,
                    Ok(n) => {
                        stderr_buf.extend_from_slice(&stderr_tmp[..n]);
                        last_output_time = std::time::Instant::now();
                    }
                    Err(_) => {}
                }
            }
            result = child.wait() => {
                // Process exited - drain remaining output
                if let Err(e) = stdout_handle.read_to_end(&mut stdout_buf).await {
                    tracing::debug!("Failed to drain stdout: {e}");
                }
                if let Err(e) = stderr_handle.read_to_end(&mut stderr_buf).await {
                    tracing::debug!("Failed to drain stderr: {e}");
                }

                let duration_ms = start_time.elapsed().as_millis();
                let proc_status = result.ok();
                let exit_code = proc_status.and_then(|s| s.code());
                let success = proc_status.map(|s| s.success()).unwrap_or(false);

                tracing::debug!(
                    "[code-exec] Finished in {}ms, exit={:?}, stdout={}B, stderr={}B: {} ({})",
                    duration_ms,
                    exit_code,
                    stdout_buf.len(),
                    stderr_buf.len(),
                    command,
                    source_info
                );

                return ExecutionResult {
                    status: if success {
                        ExecutionStatus::Success
                    } else {
                        ExecutionStatus::Failed
                    },
                    exit_code,
                    stdout: String::from_utf8_lossy(&stdout_buf).to_string(),
                    stderr: String::from_utf8_lossy(&stderr_buf).to_string(),
                    duration_ms: duration_ms.try_into().unwrap_or(u64::MAX),
                    error: if success {
                        None
                    } else {
                        Some(format!("Process exited with code {:?}", exit_code))
                    },
                    metadata: None,
                };
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                // Small sleep to prevent busy loop
            }
        }
    }
}

/// Prepare code for execution by processing doctest-style hidden line markers.
///
/// Lines starting with `#` have the prefix stripped before compilation:
/// - `# code` -> `code` (hidden setup code)
/// - `#[attr]` -> `[attr]` (attributes - but we actually want `#[attr]`!)
/// - `##` -> `#` (escape sequence)
///
/// Wait, that's wrong for attributes. Let me re-read rustdoc behavior:
/// In rustdoc, `# ` (hash + space) hides the line but includes it.
/// `#[attr]` is NOT hidden - it's a normal attribute.
///
/// So the actual rule is:
/// - `# ` (hash + space) -> strip `# ` prefix, include in compilation
/// - `##` -> becomes `#` in output (escape for displaying a #)
/// - `#foo` where foo doesn't start with space -> normal code, not hidden
fn prepare_code_for_execution(code: &str) -> String {
    code.lines()
        .map(|line| {
            if let Some(rest) = line.strip_prefix("# ") {
                // `# code` -> `code` (hidden line, include in compilation)
                rest.to_string()
            } else if let Some(rest) = line.strip_prefix("##") {
                // `##` -> `#` (escape sequence)
                format!("#{}", rest)
            } else {
                // Normal line (including #[attr] which should stay as-is)
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
