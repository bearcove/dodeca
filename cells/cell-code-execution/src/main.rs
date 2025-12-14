//! Dodeca code execution plugin (dodeca-mod-code-execution)
//!
//! This plugin handles extracting and executing code samples from markdown.

use cell_code_execution_proto::{CodeExecutor, CodeExecutionResult, CodeExecutorServer};

// Include implementation code directly
include!("impl.rs");

dodeca_cell_runtime::cell_service!(
    CodeExecutorServer<CodeExecutorImpl>,
    CodeExecutorImpl
);

dodeca_cell_runtime::run_cell!(CodeExecutorImpl);
