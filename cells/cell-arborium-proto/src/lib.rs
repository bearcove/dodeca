//! RPC protocol for dodeca syntax highlighting plugin
//!
//! Defines the SyntaxHighlightService that both host and plugin implement.

use facet::Facet;

/// Result of syntax highlighting
#[derive(Facet, Debug, Clone)]
pub struct HighlightResult {
    /// The highlighted HTML (with <span> tags for tokens)
    pub html: String,
    /// Whether highlighting was successful (vs fallback to plain text)
    pub highlighted: bool,
}

/// Syntax highlighting service provided by the host
///
/// The plugin calls these methods to get syntax highlighting.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait SyntaxHighlightService {
    /// Highlight source code and return HTML with syntax highlighting.
    async fn highlight_code(&self, code: String, language: String) -> crate::HighlightResult;

    /// Get list of supported languages.
    async fn supported_languages(&self) -> Vec<String>;
}
