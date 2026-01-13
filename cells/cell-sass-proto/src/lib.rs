//! RPC protocol for dodeca SASS cell
//!
//! Defines services for SASS/SCSS compilation.

use facet::Facet;
use std::collections::HashMap;

/// Result of SASS compilation
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum SassResult {
    /// Successfully compiled SASS
    Success { css: String },
    /// Error during compilation
    Error { message: String },
}

/// Input for SASS compilation - a map of filename -> content pairs
#[derive(Debug, Clone, Facet)]
pub struct SassInput {
    pub files: HashMap<String, String>,
}

/// SASS compilation service implemented by the cell.
///
/// The host calls these methods to compile SASS/SCSS to CSS.
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait SassCompiler {
    /// Compile SASS/SCSS to CSS
    ///
    /// Takes a map of filename -> content pairs, with "main.scss" as the entry point.
    /// Returns compiled CSS, or an error if compilation fails.
    async fn compile_sass(&self, input: SassInput) -> SassResult;
}
