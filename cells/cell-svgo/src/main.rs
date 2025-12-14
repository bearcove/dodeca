//! Dodeca SVGO plugin (dodeca-mod-svgo)
//!
//! This plugin handles SVG optimization.

use cell_svgo_proto::{SvgoOptimizer, SvgoResult, SvgoOptimizerServer};

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

dodeca_cell_runtime::cell_service!(
    SvgoOptimizerServer<SvgoOptimizerImpl>,
    SvgoOptimizerImpl
);

dodeca_cell_runtime::run_cell!(SvgoOptimizerImpl);
