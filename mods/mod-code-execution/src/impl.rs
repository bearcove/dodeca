use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use std::process::{Command, Stdio};

use mod_code_execution_proto::*;

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
                execute_code_sample(&sample, &input.config)
            };
            results.push((sample, result));
        }

        CodeExecutionResult::ExecuteSuccess {
            output: ExecuteSamplesOutput { results },
        }
    }
}

fn should_execute(language: &str) -> bool {
    match language.to_lowercase().as_str() {
        "rust" | "rs" | "javascript" | "js" | "python" | "py" | "bash" | "sh" => true,
        _ => false,
    }
}

fn execute_code_sample(sample: &CodeSample, _config: &CodeExecutionConfig) -> ExecutionResult {
    // Simple execution for now - this would need the full language config logic
    let (command, args) = match sample.language.to_lowercase().as_str() {
        "rust" | "rs" => ("cargo".to_string(), vec!["run", "--release", "--bin", "sample"]),
        "javascript" | "js" => ("node".to_string(), vec!["-e", &sample.code]),
        "python" | "py" => ("python".to_string(), vec!["-c", &sample.code]),
        "bash" | "sh" => ("bash".to_string(), vec!["-c", &sample.code]),
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

    match Command::new(&command)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) => {
            let duration_ms = start_time.elapsed().as_millis();
            ExecutionResult {
                success: output.status.success(),
                exit_code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                duration_ms: duration_ms.try_into().unwrap(),
                error: if output.status.success() {
                    None
                } else {
                    Some(format!("Process exited with code {:?}", output.status.code()))
                },
                metadata: None,
                skipped: false,
            }
        }
        Err(e) => ExecutionResult {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("Failed to execute {}: {}", command, e),
            duration_ms: 0,
            error: Some(format!("Failed to execute {}: {}", command, e)),
            metadata: None,
            skipped: false,
        },
    }
}