//! Dodeca authoring language server.

pub mod authoring_lsp;

pub use authoring_lsp::{run, run_with_provider, serve_on};
