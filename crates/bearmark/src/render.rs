//! Main rendering pipeline.

use std::collections::HashMap;
use std::sync::Arc;

use pulldown_cmark::{CodeBlockKind, Event, MetadataBlockKind, Options, Parser, Tag, TagEnd};

use crate::Result;
use crate::frontmatter::{Frontmatter, FrontmatterFormat};
use crate::handler::{
    BoxedHandler, BoxedRuleHandler, CodeBlockHandler, RawCodeHandler, RuleHandler, html_escape,
};
use crate::headings::{Heading, slugify};
use crate::links::resolve_link;
use crate::rules::{RuleDefinition, extract_rules};

/// Options for rendering markdown.
#[derive(Default)]
pub struct RenderOptions {
    /// Source file path (for relative link resolution)
    pub source_path: Option<String>,

    /// Code block handlers keyed by language
    pub code_handlers: HashMap<String, BoxedHandler>,

    /// Default handler for languages without a specific handler
    pub default_handler: Option<BoxedHandler>,

    /// Custom handler for rendering rule definitions
    pub rule_handler: Option<BoxedRuleHandler>,
}

impl RenderOptions {
    /// Create new render options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler for one or more languages.
    pub fn with_handler<H: CodeBlockHandler + 'static>(
        mut self,
        languages: &[&str],
        handler: H,
    ) -> Self {
        let handler = Arc::new(handler);
        for language in languages {
            self.code_handlers
                .insert(language.to_string(), handler.clone());
        }
        self
    }

    /// Set the default handler for unregistered languages.
    pub fn with_default_handler<H: CodeBlockHandler + 'static>(mut self, handler: H) -> Self {
        self.default_handler = Some(Arc::new(handler));
        self
    }

    /// Set a custom handler for rule definitions.
    pub fn with_rule_handler<H: RuleHandler + 'static>(mut self, handler: H) -> Self {
        self.rule_handler = Some(Arc::new(handler));
        self
    }
}

/// A code sample extracted from markdown
#[derive(Debug, Clone)]
pub struct CodeSample {
    /// Line number where this code block starts (1-indexed)
    pub line: usize,
    /// Full language string (e.g., "rust,test", "python,ignore")
    pub language: String,
    /// The raw code content
    pub code: String,
}

/// A rendered markdown document.
#[derive(Debug, Clone)]
pub struct Document {
    /// Raw metadata content (without delimiters)
    pub raw_metadata: Option<String>,

    /// Detected metadata format
    pub metadata_format: Option<FrontmatterFormat>,

    /// Parsed frontmatter (if present) - convenience accessor
    pub frontmatter: Option<Frontmatter>,

    /// Rendered HTML content
    pub html: String,

    /// Extracted headings for TOC generation
    pub headings: Vec<Heading>,

    /// Extracted rule definitions
    pub rules: Vec<RuleDefinition>,

    /// Code samples found in the document
    pub code_samples: Vec<CodeSample>,
}

/// Render markdown to HTML.
///
/// # Example
///
/// ```rust,ignore
/// use bearmark::{render, RenderOptions};
///
/// let markdown = r#"
/// +++
/// title = "Hello"
/// +++
///
/// # World
///
/// Some content.
/// "#;
///
/// let doc = render(markdown, &RenderOptions::default()).await?;
/// println!("{}", doc.html);
/// ```
pub async fn render(markdown: &str, options: &RenderOptions) -> Result<Document> {
    // 1. Extract and transform rule definitions
    let (content_with_rules, rules) =
        extract_rules(markdown, options.rule_handler.as_ref()).await?;

    // 2. Parse markdown with metadata block support
    let parser_options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;

    let parser = Parser::new_ext(&content_with_rules, parser_options);

    // Collect events, noting code blocks and metadata for processing
    let mut events: Vec<Event<'_>> = Vec::new();
    // (index, full_language, base_language, code, line_number)
    // full_language: e.g., "rust,test" - the complete language string from markdown
    // base_language: e.g., "rust" - the part before comma, used for syntax highlighting
    let mut code_blocks: Vec<(usize, String, String, String, usize)> = Vec::new();
    let mut headings: Vec<Heading> = Vec::new();

    // Metadata tracking
    let mut raw_metadata: Option<String> = None;
    let mut metadata_format: Option<FrontmatterFormat> = None;
    let mut in_metadata_block: Option<MetadataBlockKind> = None;

    // Track heading text accumulation
    let mut in_heading: Option<u8> = None;
    let mut heading_text = String::new();

    // Track line numbers for code samples
    let mut current_line = 1usize;

    for event in parser {
        match &event {
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = Some(*level as u8);
                heading_text.clear();
            }
            Event::End(TagEnd::Heading(level)) => {
                let id = slugify(&heading_text);
                headings.push(Heading {
                    title: heading_text.clone(),
                    id,
                    level: *level as u8,
                });
                in_heading = None;
            }
            Event::Text(text) if in_heading.is_some() => {
                heading_text.push_str(text);
            }
            Event::Code(code) if in_heading.is_some() => {
                heading_text.push_str(code);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let full_language = match kind {
                    CodeBlockKind::Fenced(lang) => lang.split_whitespace().next().unwrap_or(""),
                    CodeBlockKind::Indented => "",
                };
                // Extract base language (before comma) for syntax highlighting
                // e.g., "rust,test" -> "rust", "python,ignore" -> "python"
                let base_language = full_language.split(',').next().unwrap_or(full_language);
                // Mark position for later replacement, recording current line number
                code_blocks.push((
                    events.len(),
                    full_language.to_string(),
                    base_language.to_string(),
                    String::new(),
                    current_line,
                ));
            }
            Event::Start(Tag::MetadataBlock(kind)) => {
                in_metadata_block = Some(*kind);
                metadata_format = Some(match kind {
                    MetadataBlockKind::YamlStyle => FrontmatterFormat::Yaml,
                    MetadataBlockKind::PlusesStyle => FrontmatterFormat::Toml,
                });
                continue; // Don't add metadata events to output
            }
            Event::End(TagEnd::MetadataBlock(_)) => {
                in_metadata_block = None;
                continue; // Don't add metadata events to output
            }
            Event::Text(text) => {
                // Capture metadata block content
                if in_metadata_block.is_some() {
                    raw_metadata = Some(text.to_string());
                    continue; // Don't add to events
                }
                // If we're in a code block, accumulate the code
                if let Some((_, _, _, code, _)) = code_blocks.last_mut()
                    && matches!(events.last(), Some(Event::Start(Tag::CodeBlock(_))))
                {
                    code.push_str(text);
                    continue; // Don't add text event, we'll replace the whole block
                }
                // Track line numbers for accurate code sample locations
                current_line += text.matches('\n').count();
            }
            Event::End(TagEnd::CodeBlock) => {
                // Code block ends - we'll process it separately
            }
            _ => {}
        }
        events.push(event);
    }

    // 4. Process code blocks with handlers
    let fallback: BoxedHandler = Arc::new(RawCodeHandler);

    let mut rendered_blocks: HashMap<usize, String> = HashMap::new();

    for (idx, _full_language, base_language, code, _line) in &code_blocks {
        // Use base_language (before comma) to look up the handler
        let handler = options
            .code_handlers
            .get(base_language.as_str())
            .or(options.default_handler.as_ref())
            .unwrap_or(&fallback);

        // Pass base_language to the handler for syntax highlighting
        let rendered = handler.render(base_language, code).await?;
        rendered_blocks.insert(*idx, rendered);
    }

    // 4b. Build code samples for callers
    let code_samples: Vec<CodeSample> = code_blocks
        .iter()
        .map(|(_, full_language, _, code, line)| CodeSample {
            line: *line,
            language: full_language.clone(),
            code: code.clone(),
        })
        .collect();

    // 5. Generate final HTML
    let mut html = String::new();
    let mut skip_until_code_block_end = false;
    let mut heading_index = 0usize;

    for (idx, event) in events.iter().enumerate() {
        // Check if this is a code block start we need to replace
        if let Some(rendered) = rendered_blocks.get(&idx) {
            html.push_str(rendered);
            skip_until_code_block_end = true;
            continue;
        }

        if skip_until_code_block_end {
            if matches!(event, Event::End(TagEnd::CodeBlock)) {
                skip_until_code_block_end = false;
            }
            continue;
        }

        // Handle special events
        match event {
            Event::Start(Tag::Heading { level, id, .. }) => {
                let level_num = *level as u8;
                // Use the heading at current index (headings were collected in order)
                if let Some(heading) = headings.get(heading_index) {
                    let id_attr = id.as_ref().map(|s| s.as_ref()).unwrap_or(&heading.id);
                    html.push_str(&format!("<h{} id=\"{}\">", level_num, html_escape(id_attr)));
                    heading_index += 1;
                } else {
                    html.push_str(&format!("<h{}>", level_num));
                }
            }
            Event::End(TagEnd::Heading(level)) => {
                html.push_str(&format!("</h{}>", *level as u8));
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                // Resolve internal links (@/ and relative .md)
                let resolved = resolve_link(dest_url, options.source_path.as_deref());
                let title_attr = if title.is_empty() {
                    String::new()
                } else {
                    format!(" title=\"{}\"", html_escape(title))
                };
                // Include id attribute if present (for reference-style links)
                let id_attr = if id.is_empty() {
                    String::new()
                } else {
                    format!(" id=\"{}\"", html_escape(id))
                };
                html.push_str(&format!(
                    "<a href=\"{}\"{}{}>",
                    html_escape(&resolved),
                    title_attr,
                    id_attr
                ));
                let _ = link_type; // Acknowledge unused for now
            }
            Event::End(TagEnd::Link) => {
                html.push_str("</a>");
            }
            _ => {
                // Use pulldown_cmark's HTML rendering for other events
                pulldown_cmark::html::push_html(&mut html, std::iter::once(event.clone()));
            }
        }
    }

    // Parse frontmatter from raw metadata if present
    let frontmatter = match (&raw_metadata, &metadata_format) {
        (Some(raw), Some(FrontmatterFormat::Toml)) => facet_toml::from_str::<Frontmatter>(raw).ok(),
        (Some(raw), Some(FrontmatterFormat::Yaml)) => facet_yaml::from_str::<Frontmatter>(raw).ok(),
        _ => None,
    };

    Ok(Document {
        raw_metadata,
        metadata_format,
        frontmatter,
        html,
        headings,
        rules,
        code_samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_render_simple() {
        let md = "# Hello\n\nWorld.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert!(doc.html.contains("<h1"));
        assert!(doc.html.contains("Hello"));
        assert!(doc.html.contains("World"));
        assert_eq!(doc.headings.len(), 1);
        assert_eq!(doc.headings[0].title, "Hello");
        assert_eq!(doc.headings[0].id, "hello");
    }

    #[tokio::test]
    async fn test_render_with_frontmatter() {
        let md = "+++\ntitle = \"Test\"\nweight = 5\n+++\n# Content";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert!(doc.frontmatter.is_some());
        let fm = doc.frontmatter.unwrap();
        assert_eq!(fm.title, "Test");
        assert_eq!(fm.weight, 5);
    }

    #[tokio::test]
    async fn test_render_with_rules() {
        let md = "r[my.rule]\nThis MUST be followed.\n";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.rules.len(), 1);
        assert_eq!(doc.rules[0].id, "my.rule");
        assert!(doc.html.contains("id=\"r-my.rule\""));
    }

    #[tokio::test]
    async fn test_render_code_block_default() {
        let md = "```rust\nfn main() {}\n```\n";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        // Should use RawCodeHandler fallback
        assert!(doc.html.contains("<pre><code"));
        assert!(doc.html.contains("fn main()"));
    }

    #[tokio::test]
    async fn test_render_with_custom_rule_handler() {
        use crate::handler::RuleHandler;
        use crate::rules::RuleDefinition;
        use std::future::Future;
        use std::pin::Pin;

        struct CustomRuleHandler;

        impl RuleHandler for CustomRuleHandler {
            fn render<'a>(
                &'a self,
                rule: &'a RuleDefinition,
            ) -> Pin<Box<dyn Future<Output = crate::Result<String>> + Send + 'a>> {
                Box::pin(async move {
                    Ok(format!(
                        "<div class=\"custom-rule\" data-rule=\"{}\"></div>",
                        rule.id
                    ))
                })
            }
        }

        let md = "r[custom.test]\nSome rule text.\n";
        let opts = RenderOptions::new().with_rule_handler(CustomRuleHandler);
        let doc = render(md, &opts).await.unwrap();

        assert_eq!(doc.rules.len(), 1);
        assert_eq!(doc.rules[0].id, "custom.test");
        assert!(doc.html.contains("class=\"custom-rule\""));
        assert!(doc.html.contains("data-rule=\"custom.test\""));
    }

    #[tokio::test]
    async fn test_render_unique_heading_ids() {
        let md = r#"# Main Title

## Section A

Content A.

## Section B

Content B.

### Subsection B1

Details 1.

### Subsection B2

Details 2.
"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        // Each heading should have its own unique ID
        assert_eq!(doc.headings.len(), 5);
        assert_eq!(doc.headings[0].id, "main-title");
        assert_eq!(doc.headings[1].id, "section-a");
        assert_eq!(doc.headings[2].id, "section-b");
        assert_eq!(doc.headings[3].id, "subsection-b1");
        assert_eq!(doc.headings[4].id, "subsection-b2");

        // Verify HTML has correct IDs
        assert!(doc.html.contains(r#"id="main-title""#));
        assert!(doc.html.contains(r#"id="section-a""#));
        assert!(doc.html.contains(r#"id="section-b""#));
        assert!(doc.html.contains(r#"id="subsection-b1""#));
        assert!(doc.html.contains(r#"id="subsection-b2""#));
    }
}
