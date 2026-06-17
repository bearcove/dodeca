//! Dodeca code execution cell (cell-code-execution)
//!
//! This cell handles extracting and executing code samples from markdown.

#[cfg(feature = "dynamic-cell")]
use cell_code_execution_proto::CodeExecutorDispatcher;

// Include implementation code directly
include!("impl.rs");

#[cfg(feature = "dynamic-cell")]
dodeca_cell_runtime::declare_cell!("code_execution", |_host| {
    CodeExecutorDispatcher::new(CodeExecutorImpl)
});
