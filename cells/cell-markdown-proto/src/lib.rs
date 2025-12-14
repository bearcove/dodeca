//! RPC protocol for dodeca markdown processing plugin
//!
//! This plugin handles:
//! - Markdown to HTML conversion (pulldown-cmark)
//! - Frontmatter parsing (TOML/YAML)
//! - Heading extraction
//! - Code block extraction (for syntax highlighting by host)

use facet::Facet;

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

/// A code block that needs syntax highlighting
#[derive(Debug, Clone, Facet)]
pub struct CodeBlock {
    /// The code content
    pub code: String,
    /// The language (may be empty)
    pub language: String,
    /// Placeholder string to replace in the HTML output
    pub placeholder: String,
}

/// Parsed frontmatter fields
#[derive(Debug, Clone, Default, Facet)]
pub struct Frontmatter {
    pub title: String,
    pub weight: i32,
    pub description: Option<String>,
    pub template: Option<String>,
    /// Extra fields as JSON string (for flexibility)
    pub extra_json: String,
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
        /// HTML output (may contain code block placeholders)
        html: String,
        /// Extracted headings
        headings: Vec<Heading>,
        /// Code blocks that need highlighting (host will call arborium)
        code_blocks: Vec<CodeBlock>,
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
        code_blocks: Vec<CodeBlock>,
    },
    /// Error during parsing
    Error { message: String },
}

// ============================================================================
// Plugin service (host calls these)
// ============================================================================

/// Markdown processing service implemented by the PLUGIN.
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
    async fn render_markdown(&self, markdown: String) -> MarkdownResult;

    /// Parse frontmatter and render markdown in one call.
    ///
    /// Convenience method that combines parse_frontmatter and render_markdown.
    async fn parse_and_render(&self, content: String) -> ParseResult;
}
