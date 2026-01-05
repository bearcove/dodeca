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
use crate::rules::{RuleDefinition, SourceSpan, default_rule_html, parse_rule_marker};

/// An element in the document, in document order.
/// This allows consumers to build hierarchical structures (like outlines)
/// by walking the elements in order.
#[derive(Debug, Clone)]
pub enum DocElement {
    /// A heading (h1-h6)
    Heading(Heading),
    /// A rule definition (r[rule.id])
    Rule(RuleDefinition),
}

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

    /// All document elements (headings and rules) in document order.
    /// Useful for building hierarchical structures like outlines with coverage.
    pub elements: Vec<DocElement>,
}

/// Convert a byte offset to a 1-indexed line number.
fn offset_to_line(content: &str, offset: usize) -> usize {
    content[..offset.min(content.len())].matches('\n').count() + 1
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
    // Parse markdown with metadata block support, using offset iterator for line tracking
    let parser_options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;

    let parser = Parser::new_ext(markdown, parser_options).into_offset_iter();

    // Collected data
    let mut headings: Vec<Heading> = Vec::new();
    let mut rules: Vec<RuleDefinition> = Vec::new();
    let mut elements: Vec<DocElement> = Vec::new();

    // Events to render (may be modified for rules)
    let mut events_with_offsets: Vec<(Event<'_>, std::ops::Range<usize>)> = Vec::new();

    // Code blocks: (event_index, full_language, base_language, code, line_number)
    let mut code_blocks: Vec<(usize, String, String, String, usize)> = Vec::new();

    // Metadata tracking
    let mut raw_metadata: Option<String> = None;
    let mut metadata_format: Option<FrontmatterFormat> = None;
    let mut in_metadata_block: Option<MetadataBlockKind> = None;

    // Heading tracking
    let mut in_heading: Option<u8> = None;
    let mut heading_text = String::new();
    let mut heading_start_offset: usize = 0;

    // Paragraph/rule tracking
    let mut in_paragraph = false;
    let mut paragraph_start_offset: usize = 0;
    let mut paragraph_text = String::new();
    let mut paragraph_events: Vec<(Event<'_>, std::ops::Range<usize>)> = Vec::new();

    // Track seen rule IDs for duplicate detection
    let mut seen_rule_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Track rule event indices for later replacement with rendered HTML
    let mut rule_event_indices: Vec<(usize, String)> = Vec::new();

    for (event, range) in parser {
        match &event {
            // ===== Headings =====
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = Some(*level as u8);
                heading_text.clear();
                heading_start_offset = range.start;
                events_with_offsets.push((event, range));
            }
            Event::End(TagEnd::Heading(level)) => {
                let id = slugify(&heading_text);
                let line = offset_to_line(markdown, heading_start_offset);
                let heading = Heading {
                    title: heading_text.clone(),
                    id,
                    level: *level as u8,
                    line,
                };
                headings.push(heading.clone());
                elements.push(DocElement::Heading(heading));
                in_heading = None;
                events_with_offsets.push((event, range));
            }
            Event::Text(text) if in_heading.is_some() => {
                heading_text.push_str(text);
                events_with_offsets.push((event, range));
            }
            Event::Code(code) if in_heading.is_some() => {
                heading_text.push_str(code);
                events_with_offsets.push((event, range));
            }

            // ===== Paragraphs (potential rules) =====
            Event::Start(Tag::Paragraph) => {
                in_paragraph = true;
                paragraph_start_offset = range.start;
                paragraph_text.clear();
                paragraph_events.clear();
                paragraph_events.push((event, range));
            }
            Event::End(TagEnd::Paragraph) => {
                in_paragraph = false;
                paragraph_events.push((event, range));

                // Check if this paragraph is a rule definition
                let trimmed = paragraph_text.trim();
                if trimmed.starts_with("r[") {
                    // Try to parse as a rule
                    if let Some(rule_result) = try_parse_rule(
                        trimmed,
                        markdown,
                        paragraph_start_offset,
                        &mut seen_rule_ids,
                    ) {
                        match rule_result {
                            Ok(rule) => {
                                // Store the event index where we'll insert the rule HTML
                                let rule_event_idx = events_with_offsets.len();
                                rule_event_indices.push((rule_event_idx, rule.id.clone()));

                                // Add rule to collections
                                rules.push(rule.clone());
                                elements.push(DocElement::Rule(rule));

                                // Push a placeholder that we'll replace later
                                events_with_offsets.push((
                                    Event::Html("".into()),
                                    paragraph_start_offset..paragraph_start_offset,
                                ));
                                continue; // Don't add the paragraph events
                            }
                            Err(_) => {
                                // Not a valid rule, treat as normal paragraph
                            }
                        }
                    }
                }

                // Normal paragraph - add all collected events
                events_with_offsets.append(&mut paragraph_events);
            }
            Event::Text(text) if in_paragraph => {
                paragraph_text.push_str(text);
                paragraph_events.push((event, range));
            }
            Event::Code(code) if in_paragraph => {
                paragraph_text.push('`');
                paragraph_text.push_str(code);
                paragraph_text.push('`');
                paragraph_events.push((event, range));
            }
            Event::SoftBreak if in_paragraph => {
                paragraph_text.push(' ');
                paragraph_events.push((event, range));
            }
            Event::HardBreak if in_paragraph => {
                paragraph_text.push('\n');
                paragraph_events.push((event, range));
            }

            // ===== Code blocks =====
            Event::Start(Tag::CodeBlock(kind)) => {
                let full_language = match kind {
                    CodeBlockKind::Fenced(lang) => lang.split_whitespace().next().unwrap_or(""),
                    CodeBlockKind::Indented => "",
                };
                let base_language = full_language.split(',').next().unwrap_or(full_language);
                let line = offset_to_line(markdown, range.start);
                code_blocks.push((
                    events_with_offsets.len(),
                    full_language.to_string(),
                    base_language.to_string(),
                    String::new(),
                    line,
                ));
                events_with_offsets.push((event, range));
            }
            Event::Text(text)
                if !code_blocks.is_empty()
                    && matches!(
                        events_with_offsets.last(),
                        Some((Event::Start(Tag::CodeBlock(_)), _))
                    ) =>
            {
                // Accumulate code block content
                if let Some((_, _, _, code, _)) = code_blocks.last_mut() {
                    code.push_str(text);
                }
                // Don't add to events - we'll replace the whole block
                continue;
            }
            Event::End(TagEnd::CodeBlock) => {
                events_with_offsets.push((event, range));
            }

            // ===== Metadata blocks =====
            Event::Start(Tag::MetadataBlock(kind)) => {
                in_metadata_block = Some(*kind);
                metadata_format = Some(match kind {
                    MetadataBlockKind::YamlStyle => FrontmatterFormat::Yaml,
                    MetadataBlockKind::PlusesStyle => FrontmatterFormat::Toml,
                });
                // Don't add metadata events to output
                continue;
            }
            Event::End(TagEnd::MetadataBlock(_)) => {
                in_metadata_block = None;
                continue;
            }
            Event::Text(text) if in_metadata_block.is_some() => {
                raw_metadata = Some(text.to_string());
                continue;
            }

            // ===== Everything else =====
            _ => {
                if in_paragraph {
                    paragraph_events.push((event, range));
                } else {
                    events_with_offsets.push((event, range));
                }
            }
        }
    }

    // Process code blocks with handlers
    let fallback: BoxedHandler = Arc::new(RawCodeHandler);
    let mut rendered_blocks: HashMap<usize, String> = HashMap::new();

    for (idx, _full_language, base_language, code, _line) in &code_blocks {
        let handler = options
            .code_handlers
            .get(base_language.as_str())
            .or(options.default_handler.as_ref())
            .unwrap_or(&fallback);

        let rendered = handler.render(base_language, code).await?;
        rendered_blocks.insert(*idx, rendered);
    }

    // Build code samples
    let code_samples: Vec<CodeSample> = code_blocks
        .iter()
        .map(|(_, full_language, _, code, line)| CodeSample {
            line: *line,
            language: full_language.clone(),
            code: code.clone(),
        })
        .collect();

    // Render rules with custom handler if provided
    // We need to re-process rules through the handler now
    let mut rule_html_map: HashMap<String, String> = HashMap::new();
    for rule in &rules {
        let rendered = if let Some(handler) = &options.rule_handler {
            handler.render(rule).await?
        } else {
            default_rule_html(rule)
        };
        rule_html_map.insert(rule.id.clone(), rendered);
    }

    // Build a map from event index to rule ID for quick lookup
    let rule_idx_map: HashMap<usize, String> = rule_event_indices.into_iter().collect();

    // Generate final HTML
    let mut html = String::new();
    let mut skip_until_code_block_end = false;
    let mut heading_index = 0usize;

    for (idx, (event, _range)) in events_with_offsets.iter().enumerate() {
        // Check if this is a rule placeholder we need to replace
        if let Some(rule_id) = rule_idx_map.get(&idx) {
            if let Some(rendered) = rule_html_map.get(rule_id) {
                html.push_str(rendered);
            }
            continue;
        }

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

        match event {
            Event::Start(Tag::Heading { level, id, .. }) => {
                let level_num = *level as u8;
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
                let resolved = resolve_link(dest_url, options.source_path.as_deref());
                let title_attr = if title.is_empty() {
                    String::new()
                } else {
                    format!(" title=\"{}\"", html_escape(title))
                };
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
                let _ = link_type;
            }
            Event::End(TagEnd::Link) => {
                html.push_str("</a>");
            }
            Event::Html(raw_html) => {
                html.push_str(raw_html);
            }
            _ => {
                pulldown_cmark::html::push_html(&mut html, std::iter::once(event.clone()));
            }
        }
    }

    // Parse frontmatter
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
        elements,
    })
}

/// Try to parse a paragraph as a rule definition.
/// Returns Some(Ok(rule)) if successful, Some(Err) if it looks like a rule but is invalid,
/// or None if it's not a rule at all.
fn try_parse_rule(
    text: &str,
    markdown: &str,
    offset: usize,
    seen_ids: &mut std::collections::HashSet<String>,
) -> Option<Result<RuleDefinition>> {
    // Must start with r[ and have a closing ]
    if !text.starts_with("r[") {
        return None;
    }

    // Find the end of the rule marker
    let marker_end = text.find(']')?;
    let marker_content = &text[2..marker_end];

    // Parse the rule marker
    let (rule_id, metadata) = match parse_rule_marker(marker_content) {
        Ok(result) => result,
        Err(e) => return Some(Err(e)),
    };

    // Check for duplicates
    if seen_ids.contains(rule_id) {
        return Some(Err(crate::Error::DuplicateRule(rule_id.to_string())));
    }
    seen_ids.insert(rule_id.to_string());

    // Extract the rule text (everything after the marker)
    let rule_text = text[marker_end + 1..].trim().to_string();

    // Render the rule text as HTML
    let paragraph_html = if rule_text.is_empty() {
        String::new()
    } else {
        let parser = Parser::new_ext(&rule_text, Options::empty());
        let mut html = String::new();
        pulldown_cmark::html::push_html(&mut html, parser);
        html
    };

    let line = offset_to_line(markdown, offset);
    let anchor_id = format!("r-{}", rule_id);

    let rule = RuleDefinition {
        id: rule_id.to_string(),
        anchor_id,
        span: SourceSpan {
            offset,
            length: text.len(),
        },
        line,
        metadata,
        text: rule_text,
        paragraph_html,
    };

    Some(Ok(rule))
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
        assert_eq!(doc.headings[0].line, 1);
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
        let md = "r[my.rule] This MUST be followed.\n";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.rules.len(), 1);
        assert_eq!(doc.rules[0].id, "my.rule");
        assert_eq!(doc.rules[0].line, 1);
        assert!(doc.html.contains("id=\"r-my.rule\""));
    }

    #[tokio::test]
    async fn test_render_code_block_default() {
        let md = "```rust\nfn main() {}\n```\n";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

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

        let md = "r[custom.test] Some rule text.\n";
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

        assert_eq!(doc.headings.len(), 5);
        assert_eq!(doc.headings[0].id, "main-title");
        assert_eq!(doc.headings[1].id, "section-a");
        assert_eq!(doc.headings[2].id, "section-b");
        assert_eq!(doc.headings[3].id, "subsection-b1");
        assert_eq!(doc.headings[4].id, "subsection-b2");

        assert!(doc.html.contains(r#"id="main-title""#));
        assert!(doc.html.contains(r#"id="section-a""#));
        assert!(doc.html.contains(r#"id="section-b""#));
        assert!(doc.html.contains(r#"id="subsection-b1""#));
        assert!(doc.html.contains(r#"id="subsection-b2""#));
    }

    #[tokio::test]
    async fn test_elements_in_document_order() {
        let md = r#"# Heading 1

r[rule.one] First rule.

## Heading 2

r[rule.two] Second rule.

r[rule.three] Third rule.

# Heading 3
"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.elements.len(), 6);

        // Check order: H1, rule1, H2, rule2, rule3, H3
        assert!(matches!(&doc.elements[0], DocElement::Heading(h) if h.title == "Heading 1"));
        assert!(matches!(&doc.elements[1], DocElement::Rule(r) if r.id == "rule.one"));
        assert!(matches!(&doc.elements[2], DocElement::Heading(h) if h.title == "Heading 2"));
        assert!(matches!(&doc.elements[3], DocElement::Rule(r) if r.id == "rule.two"));
        assert!(matches!(&doc.elements[4], DocElement::Rule(r) if r.id == "rule.three"));
        assert!(matches!(&doc.elements[5], DocElement::Heading(h) if h.title == "Heading 3"));
    }

    #[tokio::test]
    async fn test_heading_line_numbers() {
        let md = r#"# Line 1

Some text.

## Line 5

More text.

### Line 9
"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.headings.len(), 3);
        assert_eq!(doc.headings[0].line, 1);
        assert_eq!(doc.headings[1].line, 5);
        assert_eq!(doc.headings[2].line, 9);
    }

    #[tokio::test]
    async fn test_rule_line_numbers() {
        let md = r#"# Heading

r[rule.one] First.

Text.

r[rule.two] Second.
"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.rules.len(), 2);
        assert_eq!(doc.rules[0].line, 3);
        assert_eq!(doc.rules[1].line, 7);
    }
}
