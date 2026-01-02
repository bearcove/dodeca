//! RPC protocol for dodeca markdown processing cell
//!
//! This cell uses bearmark for:
//! - Markdown to HTML conversion with syntax highlighting
//! - Frontmatter parsing (TOML/YAML)
//! - Heading extraction
//! - Rule definition extraction

use facet::Facet;
use facet_value::Value;

// ============================================================================
// Types
// ============================================================================

/// A heading extracted from markdown content
#[derive(Debug, Clone, Facet)]
pub struct Heading {
    /// The heading text
    pub title: String,
    /// The anchor ID (for linking)
    pub id: String,
    /// The heading level (1-6)
    pub level: u8,
}

/// A rule definition for specification traceability.
///
/// Rules are declared with `r[rule.name]` syntax on their own line,
/// similar to the Rust Reference's mdbook-spec.
#[derive(Debug, Clone, Facet)]
pub struct RuleDefinition {
    /// The rule identifier (e.g., "channel.id.allocation")
    pub id: String,
    /// The anchor ID for linking (e.g., "r-channel.id.allocation")
    pub anchor_id: String,
}

/// Parsed frontmatter fields
#[derive(Debug, Clone, Default, Facet)]
pub struct Frontmatter {
    pub title: String,
    pub weight: i32,
    pub description: Option<String>,
    pub template: Option<String>,
    /// Extra fields from frontmatter
    pub extra: Value,
}

// ============================================================================
// Result types
// ============================================================================

/// Result of markdown rendering
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum MarkdownResult {
    /// Successfully rendered markdown
    Success {
        /// Fully rendered HTML output (code blocks already highlighted)
        html: String,
        /// Extracted headings
        headings: Vec<Heading>,
        /// Rule definitions for specification traceability
        rules: Vec<RuleDefinition>,
    },
    /// Error during rendering
    Error { message: String },
}

/// Result of frontmatter parsing
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum FrontmatterResult {
    /// Successfully parsed frontmatter
    Success {
        frontmatter: Frontmatter,
        /// The remaining content after frontmatter
        body: String,
    },
    /// Error during parsing
    Error { message: String },
}

/// Result of combined parse (frontmatter + markdown)
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum ParseResult {
    /// Successfully parsed
    Success {
        frontmatter: Frontmatter,
        html: String,
        headings: Vec<Heading>,
        /// Rule definitions for specification traceability
        rules: Vec<RuleDefinition>,
    },
    /// Error during parsing
    Error { message: String },
}

// ============================================================================
// Cell service (host calls these)
// ============================================================================

/// Markdown processing service implemented by the CELL.
///
/// The host calls these methods to process markdown content.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait MarkdownProcessor {
    /// Parse frontmatter from content.
    ///
    /// Splits the frontmatter (TOML between `---` delimiters) from the body.
    async fn parse_frontmatter(&self, content: String) -> FrontmatterResult;

    /// Render markdown to HTML.
    ///
    /// Returns HTML with placeholders for code blocks, plus extracted headings
    /// and code blocks that need syntax highlighting.
    ///
    /// # Parameters
    /// - `source_path`: Path to the source file (e.g., "spec/_index.md") for resolving relative links
    /// - `markdown`: The markdown content to render
    async fn render_markdown(&self, source_path: String, markdown: String) -> MarkdownResult;

    /// Parse frontmatter and render markdown in one call.
    ///
    /// Convenience method that combines parse_frontmatter and render_markdown.
    ///
    /// # Parameters
    /// - `source_path`: Path to the source file (e.g., "spec/_index.md") for resolving relative links
    /// - `content`: The full content including frontmatter and markdown body
    async fn parse_and_render(&self, source_path: String, content: String) -> ParseResult;
}
