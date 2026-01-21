//! Dodeca SVGO cell (cell-svgo)
//!
//! This cell handles SVG optimization.

use cell_svgo_proto::{SvgoOptimizer, SvgoOptimizerDispatcher, SvgoResult};
use dodeca_cell_runtime::run_cell;

/// SVGO optimizer implementation
#[derive(Clone)]
pub struct SvgoOptimizerImpl;

impl SvgoOptimizer for SvgoOptimizerImpl {
    async fn optimize_svg(&self, _cx: &dodeca_cell_runtime::Context, svg: String) -> SvgoResult {
        match svag::minify(&svg) {
            Ok(optimized) => SvgoResult::Success { svg: optimized },
            Err(e) => SvgoResult::Error {
                message: format!("SVG optimization failed: {}", e),
            },
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("svgo", |_handle| SvgoOptimizerDispatcher::new(
        SvgoOptimizerImpl
    ))
}
