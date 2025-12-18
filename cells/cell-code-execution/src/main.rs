//! Dodeca code execution cell (cell-code-execution)
//!
//! This cell handles extracting and executing code samples from markdown.

use cell_code_execution_proto::{CodeExecutionResult, CodeExecutor, CodeExecutorServer};

// Include implementation code directly
include!("impl.rs");

rapace_cell::cell_service!(CodeExecutorServer<CodeExecutorImpl>, CodeExecutorImpl);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run(CellService::from(CodeExecutorImpl)).await?;
    Ok(())
}
