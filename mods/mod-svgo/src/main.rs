//! Dodeca SVGO plugin (dodeca-mod-svgo)
//!
//! This plugin handles SVG optimization.

use mod_svgo_proto::{SvgoOptimizer, SvgoResult, SvgoOptimizerServer};

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

dodeca_plugin_runtime::plugin_service!(
    SvgoOptimizerServer<SvgoOptimizerImpl>,
    SvgoOptimizerImpl
);

dodeca_plugin_runtime::run_plugin!(SvgoOptimizerImpl);
