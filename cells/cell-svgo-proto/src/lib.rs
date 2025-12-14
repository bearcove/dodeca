//! RPC protocol for dodeca SVGO plugin
//!
//! Defines services for SVG optimization.

use facet::Facet;

/// Result of SVG optimization
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum SvgoResult {
    /// Successfully optimized SVG
    Success { svg: String },
    /// Error during optimization
    Error { message: String },
}

/// SVG optimization service implemented by the plugin.
///
/// The host calls these methods to optimize SVG content.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait SvgoOptimizer {
    /// Optimize SVG content
    ///
    /// Returns optimized SVG, or an error if optimization fails.
    async fn optimize_svg(&self, svg: String) -> SvgoResult;
}