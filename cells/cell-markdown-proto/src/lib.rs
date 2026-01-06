//! RPC protocol for dodeca markdown processing cell
//!
//! This cell uses bearmark for:
//! - Markdown to HTML conversion with syntax highlighting
//! - Frontmatter parsing (TOML/YAML)
//! - Heading extraction
//! - Rule definition extraction

use facet::Facet;
use facet_value::Value;

#[cfg(test)]
mod tests {
    use super::*;
    use facet_value::{DestructuredRef, VObject, VString};
    use rapace::facet_postcard;

    #[test]
    fn test_frontmatter_extra_roundtrip() {
        // Create a Frontmatter with extra fields
        let mut extra = VObject::new();
        extra.insert(VString::from("sidebar"), Value::from(true));
        extra.insert(VString::from("icon"), Value::from("book"));
        extra.insert(VString::from("custom_value"), Value::from(42i64));

        let fm = Frontmatter {
            title: "Test".to_string(),
            weight: 0,
            description: None,
            template: None,
            extra: Value::from(extra),
        };

        // Serialize with facet_postcard
        let bytes = facet_postcard::to_vec(&fm).expect("serialize");

        // Deserialize
        let fm2: Frontmatter = facet_postcard::from_slice(&bytes).expect("deserialize");

        // Verify extra fields survived
        assert_eq!(fm2.title, "Test");
        match fm2.extra.destructure_ref() {
            DestructuredRef::Object(obj) => {
                let sidebar = obj.get("sidebar").expect("sidebar should exist");
                assert_eq!(sidebar.as_bool(), Some(true));

                let icon = obj.get("icon").expect("icon should exist");
                assert_eq!(icon.as_string().unwrap().as_str(), "book");

                let custom_value = obj.get("custom_value").expect("custom_value should exist");
                assert_eq!(custom_value.as_number().and_then(|n| n.to_i64()), Some(42));
            }
            other => panic!("expected object, got {:?}", other),
        }
    }
}

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

/// A requirement definition for specification traceability.
///
/// Requirements are declared with `r[req.name]` syntax on their own line,
/// similar to the Rust Reference's mdbook-spec.
#[derive(Debug, Clone, Facet)]
pub struct ReqDefinition {
    /// The requirement identifier (e.g., "channel.id.allocation")
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
        /// Requirement definitions for specification traceability
        reqs: Vec<ReqDefinition>,
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
        /// Requirement definitions for specification traceability
        reqs: Vec<ReqDefinition>,
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
