//! WASM client for dodeca live reload
//!
//! Re-exports facet-html-diff-wasm for DOM patching functionality.

// Re-export everything from facet-html-diff-wasm
pub use facet_html_diff_wasm::*;

// Re-export patch types from dodeca-protocol for convenience
pub use dodeca_protocol::{NodePath, Patch};
