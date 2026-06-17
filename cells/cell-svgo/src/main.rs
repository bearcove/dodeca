//! Dodeca SVGO cell (cell-svgo)
//!
//! This cell handles SVG optimization.

use cell_svgo_proto::{SvgoOptimizer, SvgoResult};

/// SVGO optimizer implementation
#[derive(Clone)]
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
