//! Dodeca SVGO cell (cell-svgo)
//!
//! This cell handles SVG optimization.

use cell_svgo_proto::{SvgoOptimizer, SvgoOptimizerServer, SvgoResult};

/// SVGO optimizer implementation
pub struct SvgoOptimizerImpl;

impl SvgoOptimizer for SvgoOptimizerImpl {
    async fn optimize_svg(&self, svg: String) -> SvgoResult {
        match svag::minify(&svg) {
            Ok(optimized) => SvgoResult::Success { svg: optimized },
            Err(e) => SvgoResult::Error {
                message: format!("SVG optimization failed: {}", e),
            },
        }
    }
}

use std::sync::Arc;

struct CellService(Arc<SvgoOptimizerServer<SvgoOptimizerImpl>>);

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
    let server = SvgoOptimizerServer::new(SvgoOptimizerImpl);
    rapace_cell::run(CellService(Arc::new(server))).await?;
    Ok(())
}
