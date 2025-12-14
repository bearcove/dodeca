use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use tokio::process::Command;
use std::process::Stdio;

use cell_code_execution_proto::*;

/// Code executor implementation
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

        if !input.config.enabled {
            return CodeExecutionResult::ExecuteSuccess {
                output: ExecuteSamplesOutput { results },
            };
        }

        // Simplified execution logic
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
                execute_code_sample(&sample, &input.config).await
            };
            results.push((sample, result));
        }

        CodeExecutionResult::ExecuteSuccess {
            output: ExecuteSamplesOutput { results },
        }
    }
}

fn should_execute(language: &str) -> bool {
    // Disable all code execution with DODECA_NO_CODE_EXEC=1
    if std::env::var("DODECA_NO_CODE_EXEC").is_ok() {
        return false;
    }

    matches!(language.to_lowercase().as_str(), "rust" | "rs")
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

async fn execute_code_sample(sample: &CodeSample, _config: &CodeExecutionConfig) -> ExecutionResult {
    use tokio::io::AsyncReadExt;

    // Reentrancy guard: refuse to execute if we're inside a ddc build
    if is_reentrant_build() {
        eprintln!(
            "[code-exec] BLOCKED: refusing to execute code inside ddc build (reentrancy guard) - {}:{}",
            sample.source_path, sample.line
        );
        return ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: "Code execution blocked: cannot run code samples during ddc build (reentrancy guard)".to_string(),
            duration_ms: 0,
            error: Some("Reentrancy guard: code execution disabled during ddc build".to_string()),
            metadata: None,
            skipped: true,
        };
    }

    // Simple execution for now - this would need the full language config logic
    let (command, args): (String, Vec<&str>) = match sample.language.to_lowercase().as_str() {
        "rust" | "rs" => ("cargo".to_string(), vec!["run", "--release", "--bin", "sample"]),
        _ => return ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("Unsupported language: {}", sample.language),
            duration_ms: 0,
            error: Some(format!("Unsupported language: {}", sample.language)),
            metadata: None,
            skipped: false,
        },
    };

    let start_time = std::time::Instant::now();
    let source_info = format!("{}:{}", sample.source_path, sample.line);

    eprintln!(
        "[code-exec] Starting: {} {} ({})",
        command,
        args.join(" "),
        source_info
    );

    // Spawn the process with piped stdout/stderr
    let mut child = match Command::new(&command)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return ExecutionResult {
                success: false,
                exit_code: None,
                stdout: String::new(),
                stderr: format!("Failed to execute {}: {}", command, e),
                duration_ms: 0,
                error: Some(format!("Failed to execute {}: {}", command, e)),
                metadata: None,
                skipped: false,
            };
        }
    };

    let mut stdout_handle = child.stdout.take().unwrap();
    let mut stderr_handle = child.stderr.take().unwrap();

    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    let mut last_output_time = std::time::Instant::now();
    let mut last_progress_report = std::time::Instant::now();

    // Read output with progress reporting and timeout
    let timeout = std::time::Duration::from_secs(EXECUTION_TIMEOUT_SECS);
    let progress_interval = std::time::Duration::from_secs(PROGRESS_INTERVAL_SECS);

    loop {
        let elapsed = start_time.elapsed();

        // Check timeout
        if elapsed > timeout {
            let _ = child.kill().await;
            eprintln!(
                "[code-exec] TIMEOUT after {}s: {} ({})",
                elapsed.as_secs(),
                command,
                source_info
            );
            return ExecutionResult {
                success: false,
                exit_code: None,
                stdout: String::from_utf8_lossy(&stdout_buf).to_string(),
                stderr: String::from_utf8_lossy(&stderr_buf).to_string(),
                duration_ms: elapsed.as_millis().try_into().unwrap_or(u64::MAX),
                error: Some(format!("Execution timed out after {}s", EXECUTION_TIMEOUT_SECS)),
                metadata: None,
                skipped: false,
            };
        }

        // Progress report every PROGRESS_INTERVAL_SECS
        if last_progress_report.elapsed() >= progress_interval {
            let since_output = last_output_time.elapsed().as_secs();
            eprintln!(
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
            eprintln!(
                "[code-exec] OUTPUT TOO LARGE ({}B): {} ({})",
                stdout_buf.len() + stderr_buf.len(),
                command,
                source_info
            );
            return ExecutionResult {
                success: false,
                exit_code: None,
                stdout: String::from_utf8_lossy(&stdout_buf).to_string(),
                stderr: String::from_utf8_lossy(&stderr_buf).to_string(),
                duration_ms: elapsed.as_millis().try_into().unwrap_or(u64::MAX),
                error: Some(format!("Output exceeded {}MB limit", MAX_OUTPUT_SIZE / 1024 / 1024)),
                metadata: None,
                skipped: false,
            };
        }

        // Try to read some output (non-blocking with short timeout)
        let mut stdout_tmp = [0u8; 4096];
        let mut stderr_tmp = [0u8; 4096];
        tokio::select! {
            result = stdout_handle.read(&mut stdout_tmp) => {
                match result {
                    Ok(0) => {} // EOF
                    Ok(n) => {
                        stdout_buf.extend_from_slice(&stdout_tmp[..n]);
                        last_output_time = std::time::Instant::now();
                    }
                    Err(_) => {}
                }
            }
            result = stderr_handle.read(&mut stderr_tmp) => {
                match result {
                    Ok(0) => {} // EOF
                    Ok(n) => {
                        stderr_buf.extend_from_slice(&stderr_tmp[..n]);
                        last_output_time = std::time::Instant::now();
                    }
                    Err(_) => {}
                }
            }
            result = child.wait() => {
                // Process exited - drain remaining output
                let _ = stdout_handle.read_to_end(&mut stdout_buf).await;
                let _ = stderr_handle.read_to_end(&mut stderr_buf).await;

                let duration_ms = start_time.elapsed().as_millis();
                let status = result.ok();
                let exit_code = status.and_then(|s| s.code());
                let success = status.map(|s| s.success()).unwrap_or(false);

                eprintln!(
                    "[code-exec] Finished in {}ms, exit={:?}, stdout={}B, stderr={}B: {} ({})",
                    duration_ms,
                    exit_code,
                    stdout_buf.len(),
                    stderr_buf.len(),
                    command,
                    source_info
                );

                return ExecutionResult {
                    success,
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
                    skipped: false,
                };
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                // Small sleep to prevent busy loop
            }
        }
    }
}