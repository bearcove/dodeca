//! Dodeca code execution cell (cell-code-execution)
//!
//! This cell handles extracting and executing code samples from markdown.

use cell_code_execution_proto::{CodeExecutionResult, CodeExecutor, CodeExecutorDispatcher};

// Include implementation code directly
include!("impl.rs");

dodeca_cell_runtime::declare_cell!("code_execution", |_host| {
    CodeExecutorDispatcher::new(CodeExecutorImpl)
});
