//! Dodeca code execution cell (cell-code-execution)
//!
//! This cell handles extracting and executing code samples from markdown.

use dodeca_cell_runtime::run_cell;

use cell_code_execution_proto::{CodeExecutionResult, CodeExecutor, CodeExecutorDispatcher};

// Include implementation code directly
include!("impl.rs");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!(
        "code_execution",
        CodeExecutorDispatcher::new(CodeExecutorImpl)
    )
}
