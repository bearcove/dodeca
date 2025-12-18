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

rapace_cell::cell_service!(SvgoOptimizerServer<SvgoOptimizerImpl>, SvgoOptimizerImpl);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run(CellService::from(SvgoOptimizerImpl)).await?;
    Ok(())
}
