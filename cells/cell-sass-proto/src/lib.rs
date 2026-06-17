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

/// SASS compilation service implemented by the cell.
///
/// The host calls these methods to compile SASS/SCSS to CSS.
#[allow(async_fn_in_trait)]
pub trait SassCompiler {
    /// Compile SASS/SCSS to CSS
    ///
    /// Takes the entry point filename, a map of filename -> content pairs, and
    /// filesystem load paths for package imports.
    /// Returns compiled CSS, or an error if compilation fails.
    async fn compile_sass(
        &self,
        entrypoint: String,
        files: HashMap<String, String>,
        load_paths: Vec<String>,
    ) -> SassResult;
}
