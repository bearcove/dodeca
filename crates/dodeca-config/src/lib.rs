//! Configuration types for dodeca static site generator.
//!
//! This crate contains the configuration structs that are parsed from
//! `.config/dodeca.styx` (or `.config/dodeca.yaml`).

use facet::Facet;

// Re-export code execution config
pub use cell_code_execution_proto::CodeExecutionConfig;

/// Dodeca configuration from `.config/dodeca.styx`
#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "snake_case")]
pub struct DodecaConfig {
    /// Base URL for the site (e.g., `https://example.com`)
    /// Used to generate permalinks. Defaults to "/" for local development.
    #[facet(default)]
    pub base_url: Option<String>,

    /// Content directory (relative to project root)
    pub content: String,

    /// Output directory (relative to project root)
    pub output: String,

    /// Link checking configuration
    #[facet(default)]
    pub link_check: Option<LinkCheckConfig>,

    /// Assets that should be served at their original paths (no cache-busting)
    /// e.g., favicon.svg, robots.txt, og-image.png
    #[facet(default)]
    pub stable_assets: Option<Vec<String>>,

    /// Code execution configuration
    #[facet(default)]
    pub code_execution: Option<CodeExecutionConfig>,

    /// Syntax highlighting theme configuration
    #[facet(default)]
    pub syntax_highlight: Option<SyntaxHighlightConfig>,
}

/// Syntax highlighting theme configuration
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct SyntaxHighlightConfig {
    /// Light theme name (e.g., "github-light", "catppuccin-latte")
    #[facet(default)]
    pub light_theme: Option<String>,

    /// Dark theme name (e.g., "tokyo-night", "catppuccin-mocha")
    #[facet(default)]
    pub dark_theme: Option<String>,
}

/// Link checking configuration
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct LinkCheckConfig {
    /// Domains to skip checking (anti-bot policies, known flaky, etc.)
    #[facet(default)]
    pub skip_domains: Option<Vec<String>>,

    /// Minimum delay between requests to the same domain (milliseconds)
    /// Default: 1000ms (1 second)
    #[facet(default)]
    pub rate_limit_ms: Option<u64>,
}
