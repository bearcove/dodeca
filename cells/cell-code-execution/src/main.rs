//! Dodeca code execution cell (cell-code-execution)
//!
//! This cell handles extracting and executing code samples from markdown.

use cell_code_execution_proto::{CodeExecutionResult, CodeExecutor, CodeExecutorServer};

// Include implementation code directly
include!("impl.rs");

use std::sync::Arc;

struct CellService(Arc<CodeExecutorServer<CodeExecutorImpl>>);

impl rapace_cell::ServiceDispatch for CellService {
    fn dispatch(
        &self,
        method_id: u32,
        payload: &[u8],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<rapace::Frame, rapace::RpcError>>
                + Send
                + 'static,
        >,
    > {
        let server = self.0.clone();
        let payload = payload.to_vec();
        Box::pin(async move { server.dispatch(method_id, &payload).await })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = CodeExecutorServer::new(CodeExecutorImpl);
    rapace_cell::run(CellService(Arc::new(server))).await?;
    Ok(())
}
