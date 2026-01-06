//! Main rendering pipeline.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use pulldown_cmark::{CodeBlockKind, Event, MetadataBlockKind, Options, Parser, Tag, TagEnd};

use crate::Result;
use crate::frontmatter::{Frontmatter, FrontmatterFormat};
use crate::handler::{
    BoxedHandler, BoxedReqHandler, CodeBlockHandler, DefaultReqHandler, RawCodeHandler, ReqHandler,
    html_escape,
};
use crate::headings::{Heading, slugify};
use crate::links::resolve_link;
use crate::reqs::{ReqDefinition, SourceSpan, parse_req_marker};

/// Parse context representing the current nested structure we're inside.
/// This replaces the ad-hoc state variables with a proper stack.
#[derive(Debug)]
#[allow(dead_code)] // Some fields are structural markers not yet used
enum ParseContext<'a> {
    /// Inside a metadata block (YAML/TOML frontmatter)
    Metadata { kind: MetadataBlockKind },

    /// Inside a heading
    Heading {
        level: u8,
        text: String,
        start_offset: usize,
    },

    /// Inside a paragraph (potential requirement)
    Paragraph {
        text: String,
        start_offset: usize,
        events: Vec<(Event<'a>, Range<usize>)>,
    },

    /// Inside a blockquote (potential requirement container)
    BlockQuote {
        start_offset: usize,
        events: Vec<(Event<'a>, Range<usize>)>,
        /// Text from first paragraph, used to detect r[...] marker
        first_para_text: String,
        /// Whether first paragraph has been completed
        first_para_done: bool,
    },

    /// Inside a code block
    CodeBlock {
        full_language: String,
        base_language: String,
        code: String,
        line: usize,
    },
}

impl<'a> ParseContext<'a> {
    /// Check if this context is a metadata block
    fn is_metadata(&self) -> bool {
        matches!(self, ParseContext::Metadata { .. })
    }

    /// Check if this context is a blockquote
    fn is_blockquote(&self) -> bool {
        matches!(self, ParseContext::BlockQuote { .. })
    }
}

/// Helper to check if any context in the stack matches a predicate
fn stack_contains<'a>(
    stack: &[ParseContext<'a>],
    predicate: impl Fn(&ParseContext<'a>) -> bool,
) -> bool {
    stack.iter().any(predicate)
}

/// A paragraph extracted from the markdown document.
/// This allows click-to-navigate features in tools like Tracy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    /// Line number where this paragraph starts (1-indexed)
    pub line: usize,
    /// Byte offset where this paragraph starts
    pub offset: usize,
}

/// An element in the document, in document order.
/// This allows consumers to build hierarchical structures (like outlines)
/// by walking the elements in order.
#[derive(Debug, Clone)]
pub enum DocElement {
    /// A heading (h1-h6)
    Heading(Heading),
    /// A requirement definition (r[req.id])
    Req(ReqDefinition),
    /// A regular paragraph (not a requirement)
    Paragraph(Paragraph),
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

    /// Custom handler for rendering requirement definitions
    pub req_handler: Option<BoxedReqHandler>,
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

    /// Set a custom handler for requirement definitions.
    pub fn with_req_handler<H: ReqHandler + 'static>(mut self, handler: H) -> Self {
        self.req_handler = Some(Arc::new(handler));
        self
    }

    /// Set the source file path for link resolution and paragraph source attributes.
    pub fn with_source_path(mut self, path: &str) -> Self {
        self.source_path = Some(path.to_string());
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

    /// Extracted requirement definitions
    pub reqs: Vec<ReqDefinition>,

    /// Code samples found in the document
    pub code_samples: Vec<CodeSample>,

    /// All document elements (headings and requirements) in document order.
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
    let mut reqs: Vec<ReqDefinition> = Vec::new();
    let mut elements: Vec<DocElement> = Vec::new();
    let mut code_samples: Vec<CodeSample> = Vec::new();

    // Output HTML - built directly as we process
    let mut html = String::new();

    // Metadata tracking (document-level, not nested)
    let mut raw_metadata: Option<String> = None;
    let mut metadata_format: Option<FrontmatterFormat> = None;

    // Track parent heading slugs for hierarchical IDs
    let mut heading_stack: Vec<(u8, String)> = Vec::new();

    // Track seen req IDs for duplicate detection
    let mut seen_req_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    // The context stack
    let mut context_stack: Vec<ParseContext<'_>> = Vec::new();

    // Default req handler
    let default_req_handler: Arc<dyn ReqHandler> = Arc::new(DefaultReqHandler);
    let req_handler = options.req_handler.as_ref().unwrap_or(&default_req_handler);

    // Default code handler
    let default_code_handler: BoxedHandler = Arc::new(RawCodeHandler);

    // Helper to check if inside blockquote
    let is_inside_blockquote =
        |stack: &[ParseContext<'_>]| stack_contains(stack, |c| c.is_blockquote());

    for (event, range) in parser {
        // If inside a blockquote, route events there
        if is_inside_blockquote(&context_stack) {
            match &event {
                Event::Start(Tag::BlockQuote(_)) => {
                    // Nested blockquote
                    context_stack.push(ParseContext::BlockQuote {
                        start_offset: range.start,
                        events: vec![(event, range)],
                        first_para_text: String::new(),
                        first_para_done: false,
                    });
                    continue;
                }
                Event::End(TagEnd::BlockQuote(_)) => {
                    // Pop and process the blockquote
                    if let Some(ParseContext::BlockQuote {
                        start_offset,
                        mut events,
                        first_para_text,
                        ..
                    }) = context_stack.pop()
                    {
                        events.push((event, range.clone()));

                        // Check if this is a req
                        let trimmed = first_para_text.trim();
                        if trimmed.starts_with("r[")
                            && let Some(req_result) = try_parse_blockquote_req(
                                trimmed,
                                markdown,
                                start_offset,
                                &mut seen_req_ids,
                            )
                        {
                            match req_result {
                                Ok(mut req) => {
                                    // Render req content HTML
                                    let content_html = render_blockquote_req_content(
                                        &events,
                                        options,
                                        &default_code_handler,
                                    )
                                    .await?;

                                    // Store content in req.html for API access
                                    req.html = content_html.clone();

                                    // Render req with start/end wrappers
                                    let start_html = req_handler.start(&req).await?;
                                    let end_html = req_handler.end(&req).await?;

                                    let req_html =
                                        format!("{}{}{}", start_html, content_html, end_html);

                                    // Check if nested in another blockquote
                                    if is_inside_blockquote(&context_stack) {
                                        if let Some(ParseContext::BlockQuote {
                                            events: parent_events,
                                            ..
                                        }) = context_stack.last_mut()
                                        {
                                            parent_events
                                                .push((Event::Html(req_html.into()), range));
                                        }
                                    } else {
                                        html.push_str(&req_html);
                                    }

                                    reqs.push(req.clone());
                                    elements.push(DocElement::Req(req));
                                    continue;
                                }
                                Err(_) => {
                                    // Invalid req, treat as normal blockquote
                                }
                            }
                        }

                        // Normal blockquote - render or add to parent
                        if is_inside_blockquote(&context_stack) {
                            if let Some(ParseContext::BlockQuote {
                                events: parent_events,
                                ..
                            }) = context_stack.last_mut()
                            {
                                parent_events.append(&mut events);
                            }
                        } else {
                            render_events_to_html(&mut html, &events, options, None);
                        }
                    }
                    continue;
                }
                Event::Start(Tag::Paragraph) => {
                    if let Some(ParseContext::BlockQuote { events, .. }) = context_stack.last_mut()
                    {
                        events.push((event, range));
                    }
                    continue;
                }
                Event::End(TagEnd::Paragraph) => {
                    if let Some(ParseContext::BlockQuote {
                        events,
                        first_para_done,
                        ..
                    }) = context_stack.last_mut()
                    {
                        events.push((event, range));
                        *first_para_done = true;
                    }
                    continue;
                }
                Event::Text(text) => {
                    if let Some(ParseContext::BlockQuote {
                        events,
                        first_para_text,
                        first_para_done,
                        ..
                    }) = context_stack.last_mut()
                    {
                        if !*first_para_done {
                            first_para_text.push_str(text);
                        }
                        events.push((event, range));
                    }
                    continue;
                }
                _ => {
                    if let Some(ParseContext::BlockQuote { events, .. }) = context_stack.last_mut()
                    {
                        events.push((event, range));
                    }
                    continue;
                }
            }
        }

        // Not inside blockquote - normal processing
        match &event {
            // ===== Blockquotes =====
            Event::Start(Tag::BlockQuote(_)) => {
                context_stack.push(ParseContext::BlockQuote {
                    start_offset: range.start,
                    events: vec![(event, range)],
                    first_para_text: String::new(),
                    first_para_done: false,
                });
            }

            // ===== Headings =====
            Event::Start(Tag::Heading { level, .. }) => {
                context_stack.push(ParseContext::Heading {
                    level: *level as u8,
                    text: String::new(),
                    start_offset: range.start,
                });
                // We'll emit the <h*> tag when we have the full heading text
            }
            Event::End(TagEnd::Heading(level)) => {
                let current_level = *level as u8;

                if let Some(ParseContext::Heading {
                    text: heading_text,
                    start_offset,
                    ..
                }) = context_stack.pop()
                {
                    let slug = slugify(&heading_text);

                    // Maintain heading hierarchy
                    while heading_stack
                        .last()
                        .is_some_and(|(lvl, _)| *lvl >= current_level)
                    {
                        heading_stack.pop();
                    }

                    let id = if heading_stack.is_empty() {
                        slug.clone()
                    } else {
                        let mut id = String::new();
                        for (_, parent_slug) in &heading_stack {
                            id.push_str(parent_slug);
                            id.push_str("--");
                        }
                        id.push_str(&slug);
                        id
                    };

                    heading_stack.push((current_level, slug));

                    let line = offset_to_line(markdown, start_offset);
                    let heading = Heading {
                        title: heading_text.clone(),
                        id: id.clone(),
                        level: current_level,
                        line,
                    };
                    headings.push(heading.clone());
                    elements.push(DocElement::Heading(heading));

                    // Emit the heading HTML
                    html.push_str(&format!(
                        "<h{} id=\"{}\">{}</h{}>",
                        current_level,
                        html_escape(&id),
                        html_escape(&heading_text),
                        current_level
                    ));
                }
            }

            // ===== Paragraphs (potential requirements) =====
            Event::Start(Tag::Paragraph) => {
                context_stack.push(ParseContext::Paragraph {
                    text: String::new(),
                    start_offset: range.start,
                    events: vec![(event, range)],
                });
            }
            Event::End(TagEnd::Paragraph) => {
                if let Some(ParseContext::Paragraph {
                    text: paragraph_text,
                    start_offset,
                    mut events,
                }) = context_stack.pop()
                {
                    events.push((event, range));

                    let trimmed = paragraph_text.trim();
                    if trimmed.starts_with("r[")
                        && let Some(req_result) = try_parse_paragraph_req(
                            trimmed,
                            markdown,
                            start_offset,
                            &mut seen_req_ids,
                            &events,
                        )
                    {
                        match req_result {
                            Ok(mut req) => {
                                // Render req content HTML
                                let content_html = render_paragraph_req_content(&events, options);

                                // Store content in req.html for API access
                                req.html = content_html.clone();

                                // Render req with start/end wrappers
                                let start_html = req_handler.start(&req).await?;
                                let end_html = req_handler.end(&req).await?;

                                html.push_str(&start_html);
                                html.push_str(&content_html);
                                html.push_str(&end_html);

                                reqs.push(req.clone());
                                elements.push(DocElement::Req(req));
                                continue;
                            }
                            Err(_) => {
                                // Invalid req, treat as normal paragraph
                            }
                        }
                    }

                    // Normal paragraph
                    let line = offset_to_line(markdown, start_offset);
                    elements.push(DocElement::Paragraph(Paragraph {
                        line,
                        offset: start_offset,
                    }));
                    render_events_to_html(&mut html, &events, options, Some(SourceInfo { line }));
                }
            }

            // ===== Code blocks =====
            Event::Start(Tag::CodeBlock(kind)) => {
                let full_language = match kind {
                    CodeBlockKind::Fenced(lang) => lang.split_whitespace().next().unwrap_or(""),
                    CodeBlockKind::Indented => "",
                };
                let base_language = full_language.split(',').next().unwrap_or(full_language);
                let line = offset_to_line(markdown, range.start);
                context_stack.push(ParseContext::CodeBlock {
                    full_language: full_language.to_string(),
                    base_language: base_language.to_string(),
                    code: String::new(),
                    line,
                });
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(ParseContext::CodeBlock {
                    full_language,
                    base_language,
                    code,
                    line,
                }) = context_stack.pop()
                {
                    // Render code block
                    let handler = options
                        .code_handlers
                        .get(&base_language)
                        .or(options.default_handler.as_ref())
                        .unwrap_or(&default_code_handler);

                    let rendered = handler.render(&base_language, &code).await?;
                    html.push_str(&rendered);

                    code_samples.push(CodeSample {
                        line,
                        language: full_language,
                        code,
                    });
                }
            }

            // ===== Metadata blocks =====
            Event::Start(Tag::MetadataBlock(kind)) => {
                metadata_format = Some(match kind {
                    MetadataBlockKind::YamlStyle => FrontmatterFormat::Yaml,
                    MetadataBlockKind::PlusesStyle => FrontmatterFormat::Toml,
                });
                context_stack.push(ParseContext::Metadata { kind: *kind });
            }
            Event::End(TagEnd::MetadataBlock(_)) => {
                context_stack.pop();
            }

            // ===== Text and content events =====
            Event::Text(text) => match context_stack.last_mut() {
                Some(ParseContext::Heading { text: t, .. }) => {
                    t.push_str(text);
                }
                Some(ParseContext::Paragraph {
                    text: t, events, ..
                }) => {
                    t.push_str(text);
                    events.push((event, range));
                }
                Some(ParseContext::CodeBlock { code, .. }) => {
                    code.push_str(text);
                }
                Some(ParseContext::Metadata { .. }) => {
                    raw_metadata = Some(text.to_string());
                }
                Some(ParseContext::BlockQuote { .. }) => {
                    unreachable!("BlockQuote text should be handled in blockquote branch");
                }
                None => {
                    html.push_str(&html_escape(text));
                }
            },
            Event::Code(code) => match context_stack.last_mut() {
                Some(ParseContext::Heading { text, .. }) => {
                    text.push_str(code);
                }
                Some(ParseContext::Paragraph { text, events, .. }) => {
                    text.push('`');
                    text.push_str(code);
                    text.push('`');
                    events.push((event, range));
                }
                _ => {
                    html.push_str("<code>");
                    html.push_str(&html_escape(code));
                    html.push_str("</code>");
                }
            },
            Event::SoftBreak => {
                if let Some(ParseContext::Paragraph { text, events, .. }) = context_stack.last_mut()
                {
                    text.push(' ');
                    events.push((event, range));
                } else {
                    html.push('\n');
                }
            }
            Event::HardBreak => {
                if let Some(ParseContext::Paragraph { text, events, .. }) = context_stack.last_mut()
                {
                    text.push('\n');
                    events.push((event, range));
                } else {
                    html.push_str("<br />\n");
                }
            }

            // ===== Everything else =====
            _ => {
                if let Some(ParseContext::Paragraph { events, .. }) = context_stack.last_mut() {
                    events.push((event, range));
                } else if !stack_contains(&context_stack, |c| c.is_metadata()) {
                    // Render directly using pulldown_cmark for other events
                    pulldown_cmark::html::push_html(&mut html, std::iter::once(event.clone()));
                }
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
        reqs,
        code_samples,
        elements,
    })
}

/// Render a list of events to HTML string
fn render_events_to_html(
    html: &mut String,
    events: &[(Event<'_>, Range<usize>)],
    options: &RenderOptions,
    source_info: Option<SourceInfo>,
) {
    for (event, _range) in events {
        match event {
            Event::Start(Tag::Paragraph) => {
                // Custom paragraph rendering with source location attributes
                let mut attrs = String::new();
                if let Some(ref info) = source_info {
                    attrs.push_str(&format!(" data-source-line=\"{}\"", info.line));
                    if let Some(ref file) = options.source_path {
                        attrs.push_str(&format!(" data-source-file=\"{}\"", html_escape(file)));
                    }
                }
                html.push_str(&format!("<p{}>", attrs));
            }
            Event::End(TagEnd::Paragraph) => {
                html.push_str("</p>\n");
            }
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => {
                let resolved = resolve_link(dest_url, options.source_path.as_deref());
                let title_attr = if title.is_empty() {
                    String::new()
                } else {
                    format!(" title=\"{}\"", html_escape(title))
                };
                html.push_str(&format!(
                    "<a href=\"{}\"{}>",
                    html_escape(&resolved),
                    title_attr
                ));
            }
            Event::End(TagEnd::Link) => {
                html.push_str("</a>");
            }
            _ => {
                pulldown_cmark::html::push_html(html, std::iter::once(event.clone()));
            }
        }
    }
}

/// Source location information for rendered elements
struct SourceInfo {
    line: usize,
}

/// Regex to match req markers like r[req.id] or r[req.id attr=value]
fn req_marker_regex() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^r\[[^\]]+\]\s*").unwrap())
}

/// Strip req marker from text if present, returns owned String
fn strip_req_marker(text: &str) -> String {
    req_marker_regex().replace(text, "").into_owned()
}
/// Render the content of a paragraph req (stripping the r[...] marker)
///
/// Uses a text buffer to accumulate consecutive text events (pulldown-cmark
/// splits text across multiple events), then strips the req marker when flushing.
fn render_paragraph_req_content(
    events: &[(Event<'_>, Range<usize>)],
    options: &RenderOptions,
) -> String {
    let mut html = String::new();
    let mut text_buffer = String::new();
    let mut marker_stripped = false;

    // Flush the text buffer, stripping the req marker if we haven't yet
    let flush_text = |html: &mut String, buffer: &mut String, stripped: &mut bool| {
        if buffer.is_empty() {
            return;
        }
        let text = if !*stripped {
            *stripped = true;
            strip_req_marker(buffer)
        } else {
            std::mem::take(buffer)
        };
        if !text.is_empty() {
            html.push_str(&html_escape(&text));
        }
        buffer.clear();
    };

    for (event, _range) in events {
        match event {
            Event::Text(t) => {
                text_buffer.push_str(t.as_ref());
            }
            Event::SoftBreak => {
                text_buffer.push('\n');
            }
            Event::HardBreak => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("<br />\n");
            }
            Event::Code(code) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("<code>");
                html.push_str(&html_escape(code));
                html.push_str("</code>");
            }
            Event::Start(Tag::Paragraph) => {
                html.push_str("<p>");
            }
            Event::End(TagEnd::Paragraph) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("</p>\n");
            }
            Event::Start(Tag::Emphasis) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("<em>");
            }
            Event::End(TagEnd::Emphasis) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("</em>");
            }
            Event::Start(Tag::Strong) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("<strong>");
            }
            Event::End(TagEnd::Strong) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("</strong>");
            }
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                let resolved = resolve_link(dest_url, options.source_path.as_deref());
                let title_attr = if title.is_empty() {
                    String::new()
                } else {
                    format!(" title=\"{}\"", html_escape(title))
                };
                html.push_str(&format!(
                    "<a href=\"{}\"{}>",
                    html_escape(&resolved),
                    title_attr
                ));
            }
            Event::End(TagEnd::Link) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("</a>");
            }
            _ => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                pulldown_cmark::html::push_html(&mut html, std::iter::once(event.clone()));
            }
        }
    }

    // Final flush
    flush_text(&mut html, &mut text_buffer, &mut marker_stripped);

    html
}

/// Render the content of a blockquote req (stripping blockquote wrapper and r[...] marker)
///
/// Uses a text buffer to accumulate consecutive text events, then strips the req marker.
async fn render_blockquote_req_content(
    events: &[(Event<'_>, Range<usize>)],
    options: &RenderOptions,
    default_code_handler: &BoxedHandler,
) -> Result<String> {
    let mut html = String::new();
    let mut text_buffer = String::new();
    let mut marker_stripped = false;
    let mut in_paragraph = false;
    let mut in_code_block = false;
    let mut code_block_lang = String::new();
    let mut code_block_content = String::new();

    // Flush the text buffer, stripping the req marker if we haven't yet
    let flush_text = |html: &mut String, buffer: &mut String, stripped: &mut bool| {
        if buffer.is_empty() {
            return;
        }
        let text = if !*stripped {
            *stripped = true;
            strip_req_marker(buffer)
        } else {
            std::mem::take(buffer)
        };
        if !text.is_empty() {
            html.push_str(&html_escape(&text));
        }
        buffer.clear();
    };

    for (event, _range) in events {
        match event {
            Event::Start(Tag::BlockQuote(_)) | Event::End(TagEnd::BlockQuote(_)) => {
                // Skip blockquote wrapper
            }
            Event::Start(Tag::Paragraph) => {
                html.push_str("<p>");
                in_paragraph = true;
            }
            Event::End(TagEnd::Paragraph) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("</p>\n");
                in_paragraph = false;
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                in_code_block = true;
                code_block_lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.split(',').next().unwrap_or("").to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                code_block_content.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                let handler = options
                    .code_handlers
                    .get(&code_block_lang)
                    .or(options.default_handler.as_ref())
                    .unwrap_or(default_code_handler);
                let rendered = handler
                    .render(&code_block_lang, &code_block_content)
                    .await?;
                html.push_str(&rendered);
            }
            Event::Text(t) if in_code_block => {
                code_block_content.push_str(t);
            }
            Event::Text(t) => {
                text_buffer.push_str(t.as_ref());
            }
            Event::SoftBreak if in_paragraph => {
                text_buffer.push('\n');
            }
            Event::HardBreak if in_paragraph => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("<br />\n");
            }
            Event::Code(code) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("<code>");
                html.push_str(&html_escape(code));
                html.push_str("</code>");
            }
            Event::Start(Tag::Emphasis) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("<em>");
            }
            Event::End(TagEnd::Emphasis) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("</em>");
            }
            Event::Start(Tag::Strong) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("<strong>");
            }
            Event::End(TagEnd::Strong) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("</strong>");
            }
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                let resolved = resolve_link(dest_url, options.source_path.as_deref());
                let title_attr = if title.is_empty() {
                    String::new()
                } else {
                    format!(" title=\"{}\"", html_escape(title))
                };
                html.push_str(&format!(
                    "<a href=\"{}\"{}>",
                    html_escape(&resolved),
                    title_attr
                ));
            }
            Event::End(TagEnd::Link) => {
                flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                html.push_str("</a>");
            }
            _ => {
                if !in_code_block {
                    flush_text(&mut html, &mut text_buffer, &mut marker_stripped);
                    pulldown_cmark::html::push_html(&mut html, std::iter::once(event.clone()));
                }
            }
        }
    }

    // Final flush
    flush_text(&mut html, &mut text_buffer, &mut marker_stripped);

    Ok(html)
}

/// Try to parse a paragraph as a requirement definition.
/// Returns Some(Ok(req)) if successful, Some(Err) if it looks like a req but is invalid,
/// or None if it's not a req at all.
fn try_parse_paragraph_req<'a>(
    text: &str,
    markdown: &str,
    offset: usize,
    seen_ids: &mut std::collections::HashSet<String>,
    _paragraph_events: &[(Event<'a>, std::ops::Range<usize>)],
) -> Option<Result<ReqDefinition>> {
    // Must start with r[ and have a closing ]
    if !text.starts_with("r[") {
        return None;
    }

    // Find the end of the req marker
    let marker_end = text.find(']')?;
    let marker_content = &text[2..marker_end];

    // Parse the req marker
    let (req_id, metadata) = match parse_req_marker(marker_content) {
        Ok(result) => result,
        Err(e) => return Some(Err(e)),
    };

    // Check for duplicates
    if seen_ids.contains(req_id) {
        return Some(Err(crate::Error::DuplicateReq(req_id.to_string())));
    }
    seen_ids.insert(req_id.to_string());

    // html is now generated separately by render_paragraph_req_content
    let html = String::new();

    let line = offset_to_line(markdown, offset);
    let anchor_id = format!("r-{}", req_id);

    let req = ReqDefinition {
        id: req_id.to_string(),
        anchor_id,
        span: SourceSpan {
            offset,
            length: text.len(),
        },
        line,
        metadata,
        html,
    };

    Some(Ok(req))
}

/// Try to parse a blockquote as a requirement definition.
/// Returns Some(Ok(req)) if successful, Some(Err) if it looks like a req but is invalid,
/// or None if it's not a req at all.
fn try_parse_blockquote_req(
    first_para_text: &str,
    markdown: &str,
    offset: usize,
    seen_ids: &mut std::collections::HashSet<String>,
) -> Option<Result<ReqDefinition>> {
    // Must start with r[ and have a closing ]
    if !first_para_text.starts_with("r[") {
        return None;
    }

    // Find the end of the req marker
    let marker_end = first_para_text.find(']')?;
    let marker_content = &first_para_text[2..marker_end];

    // Parse the req marker
    let (req_id, metadata) = match parse_req_marker(marker_content) {
        Ok(result) => result,
        Err(e) => return Some(Err(e)),
    };

    // Check for duplicates
    if seen_ids.contains(req_id) {
        return Some(Err(crate::Error::DuplicateReq(req_id.to_string())));
    }
    seen_ids.insert(req_id.to_string());

    // html is now generated separately by render_blockquote_req_content
    let html = String::new();

    let line = offset_to_line(markdown, offset);
    let anchor_id = format!("r-{}", req_id);

    let req = ReqDefinition {
        id: req_id.to_string(),
        anchor_id,
        span: SourceSpan {
            offset,
            length: first_para_text.len(),
        },
        line,
        metadata,
        html,
    };

    Some(Ok(req))
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
    async fn test_render_with_reqs() {
        let md = "r[my.req] This MUST be followed.\n";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "my.req");
        assert_eq!(doc.reqs[0].line, 1);
        assert!(doc.html.contains("id=\"r-my.req\""));
    }

    #[tokio::test]
    async fn test_render_req_with_links() {
        let md = "r[data.postcard] All payloads MUST use [Postcard](https://postcard.jamesmunns.com/wire-format).\n";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "data.postcard");
        // The HTML should preserve the link
        assert!(
            doc.html
                .contains("<a href=\"https://postcard.jamesmunns.com/wire-format\">"),
            "Link should be preserved in HTML: {}",
            doc.html
        );
        assert!(
            doc.html.contains("Postcard</a>"),
            "Link text should be preserved: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_render_req_with_formatting() {
        let md = "r[fmt.req] Text with **bold**, *italic*, and `code`.\n";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.reqs.len(), 1);
        assert!(
            doc.html.contains("<strong>bold</strong>"),
            "Bold should be preserved: {}",
            doc.html
        );
        assert!(
            doc.html.contains("<em>italic</em>"),
            "Italic should be preserved: {}",
            doc.html
        );
        assert!(
            doc.html.contains("<code>code</code>"),
            "Code should be preserved: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_render_code_block_default() {
        let md = "```rust\nfn main() {}\n```\n";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert!(doc.html.contains("<pre><code"));
        assert!(doc.html.contains("fn main()"));
    }

    #[tokio::test]
    async fn test_render_with_custom_req_handler() {
        use crate::handler::ReqHandler;
        use crate::reqs::ReqDefinition;
        use std::future::Future;
        use std::pin::Pin;

        struct CustomReqHandler;

        impl ReqHandler for CustomReqHandler {
            fn start<'a>(
                &'a self,
                req: &'a ReqDefinition,
            ) -> Pin<Box<dyn Future<Output = crate::Result<String>> + Send + 'a>> {
                Box::pin(async move {
                    Ok(format!(
                        "<div class=\"custom-req\" data-req=\"{}\">",
                        req.id
                    ))
                })
            }

            fn end<'a>(
                &'a self,
                _req: &'a ReqDefinition,
            ) -> Pin<Box<dyn Future<Output = crate::Result<String>> + Send + 'a>> {
                Box::pin(async move { Ok("</div>".to_string()) })
            }
        }

        let md = "r[custom.test] Some requirement text.\n";
        let opts = RenderOptions::new().with_req_handler(CustomReqHandler);
        let doc = render(md, &opts).await.unwrap();

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "custom.test");
        assert!(doc.html.contains("class=\"custom-req\""));
        assert!(doc.html.contains("data-req=\"custom.test\""));
    }

    #[tokio::test]
    async fn test_render_hierarchical_heading_ids() {
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
        // Top-level heading has no parent prefix
        assert_eq!(doc.headings[0].id, "main-title");
        // Level 2 headings include level 1 parent
        assert_eq!(doc.headings[1].id, "main-title--section-a");
        assert_eq!(doc.headings[2].id, "main-title--section-b");
        // Level 3 headings include both level 1 and level 2 parents
        assert_eq!(doc.headings[3].id, "main-title--section-b--subsection-b1");
        assert_eq!(doc.headings[4].id, "main-title--section-b--subsection-b2");

        assert!(doc.html.contains(r#"id="main-title""#));
        assert!(doc.html.contains(r#"id="main-title--section-a""#));
        assert!(doc.html.contains(r#"id="main-title--section-b""#));
        assert!(
            doc.html
                .contains(r#"id="main-title--section-b--subsection-b1""#)
        );
        assert!(
            doc.html
                .contains(r#"id="main-title--section-b--subsection-b2""#)
        );
    }

    #[tokio::test]
    async fn test_hierarchical_ids_reset_on_same_level() {
        // When we go back to the same level, the parent should change
        let md = r#"# Foo

## Bar

### Baz

## Qux

### Quux
"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.headings.len(), 5);
        assert_eq!(doc.headings[0].id, "foo");
        assert_eq!(doc.headings[1].id, "foo--bar");
        assert_eq!(doc.headings[2].id, "foo--bar--baz");
        // Qux is at level 2, so it resets the h3 context
        assert_eq!(doc.headings[3].id, "foo--qux");
        // Quux is under Qux, not under Bar
        assert_eq!(doc.headings[4].id, "foo--qux--quux");
    }

    #[tokio::test]
    async fn test_elements_in_document_order() {
        let md = r#"# Heading 1

r[req.one] First requirement.

## Heading 2

r[req.two] Second requirement.

r[req.three] Third requirement.

# Heading 3
"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.elements.len(), 6);

        // Check order: H1, req1, H2, req2, req3, H3
        assert!(matches!(&doc.elements[0], DocElement::Heading(h) if h.title == "Heading 1"));
        assert!(matches!(&doc.elements[1], DocElement::Req(r) if r.id == "req.one"));
        assert!(matches!(&doc.elements[2], DocElement::Heading(h) if h.title == "Heading 2"));
        assert!(matches!(&doc.elements[3], DocElement::Req(r) if r.id == "req.two"));
        assert!(matches!(&doc.elements[4], DocElement::Req(r) if r.id == "req.three"));
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
    async fn test_req_line_numbers() {
        let md = r#"# Heading

r[req.one] First.

Text.

r[req.two] Second.
"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.reqs.len(), 2);
        assert_eq!(doc.reqs[0].line, 3);
        assert_eq!(doc.reqs[1].line, 7);
    }

    // =========================================================================
    // Blockquote requirement tests
    // =========================================================================

    #[tokio::test]
    async fn test_req_in_blockquote_simple() {
        let md = "> r[my.req] This is a requirement in a blockquote.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);
        eprintln!("Reqs: {:?}", doc.reqs);

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "my.req");
        // Should NOT have blockquote wrapper in HTML - the whole blockquote IS the requirement
        assert!(
            !doc.html.contains("<blockquote>"),
            "Blockquote wrapper should be removed when it's a requirement. HTML: {}",
            doc.html
        );
        assert!(doc.html.contains("id=\"r-my.req\""));
    }

    #[tokio::test]
    async fn test_req_in_blockquote_multiline() {
        let md = r#"> r[my.req] First line of requirement.
> Second line continues.
> Third line ends."#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "my.req");
        // All lines should be in the rendered HTML
        assert!(
            doc.html.contains("First line"),
            "Should contain first line: {}",
            doc.html
        );
        assert!(
            doc.html.contains("Second line"),
            "Should contain second line: {}",
            doc.html
        );
        assert!(
            doc.html.contains("Third line"),
            "Should contain third line: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_req_in_blockquote_with_code_block() {
        let md = r#"> r[my.req] Requirement with code:
>
> ```rust
> fn main() {}
> ```"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "my.req");
        // The code block should be part of the requirement's HTML
        assert!(
            doc.html.contains("fn main()"),
            "Code block should be in HTML: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_req_in_blockquote_with_formatting() {
        let md = "> r[fmt.req] Text with **bold** and *italic*.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.reqs.len(), 1);
        assert!(
            doc.html.contains("<strong>bold</strong>"),
            "Bold should be preserved: {}",
            doc.html
        );
        assert!(
            doc.html.contains("<em>italic</em>"),
            "Italic should be preserved: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_regular_blockquote_not_req() {
        let md = r#"> This is just a regular blockquote.
> Not a requirement."#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.reqs.len(), 0);
        assert!(
            doc.html.contains("<blockquote>"),
            "Regular blockquote should be preserved: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_mixed_reqs_paragraph_and_blockquote() {
        let md = r#"r[para.req] This is a paragraph requirement.

> r[quote.req] This is a blockquote requirement."#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.reqs.len(), 2);
        assert_eq!(doc.reqs[0].id, "para.req");
        assert_eq!(doc.reqs[1].id, "quote.req");
    }

    #[tokio::test]
    async fn test_blockquote_req_with_link() {
        let md = "> r[link.req] See [the docs](https://example.com) for details.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.reqs.len(), 1);
        assert!(
            doc.html.contains("<a href=\"https://example.com\">"),
            "Link should be preserved: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_blockquote_req_in_document_order() {
        let md = r#"# Heading 1

r[para.req] Paragraph requirement.

> r[quote.req] Blockquote requirement.

## Heading 2
"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.elements.len(), 4);
        assert!(matches!(&doc.elements[0], DocElement::Heading(h) if h.title == "Heading 1"));
        assert!(matches!(&doc.elements[1], DocElement::Req(r) if r.id == "para.req"));
        assert!(matches!(&doc.elements[2], DocElement::Req(r) if r.id == "quote.req"));
        assert!(matches!(&doc.elements[3], DocElement::Heading(h) if h.title == "Heading 2"));
    }

    #[tokio::test]
    async fn test_paragraph_line_numbers() {
        let md = r#"First paragraph.

Second paragraph.

# Heading

Third paragraph.
"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        // Should have 3 paragraphs and 1 heading in elements
        let paragraphs: Vec<_> = doc
            .elements
            .iter()
            .filter_map(|e| match e {
                DocElement::Paragraph(p) => Some(p),
                _ => None,
            })
            .collect();

        assert_eq!(paragraphs.len(), 3);
        assert_eq!(paragraphs[0].line, 1);
        assert_eq!(paragraphs[0].offset, 0);
        assert_eq!(paragraphs[1].line, 3);
        assert_eq!(paragraphs[2].line, 7);
    }

    #[tokio::test]
    async fn test_paragraph_with_frontmatter_offset() {
        let md = r#"+++
title = "Test"
+++

First paragraph after frontmatter.

Second paragraph.
"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        let paragraphs: Vec<_> = doc
            .elements
            .iter()
            .filter_map(|e| match e {
                DocElement::Paragraph(p) => Some(p),
                _ => None,
            })
            .collect();

        assert_eq!(paragraphs.len(), 2);
        // First paragraph starts after frontmatter
        assert_eq!(paragraphs[0].line, 5);
        assert_eq!(paragraphs[1].line, 7);
    }

    #[tokio::test]
    async fn test_elements_include_paragraphs_in_order() {
        let md = r#"# Heading 1

Regular paragraph.

r[my.req] A requirement definition.

Another paragraph.

## Heading 2
"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        // Order: Heading 1, Paragraph, Requirement, Paragraph, Heading 2
        assert_eq!(doc.elements.len(), 5);
        assert!(matches!(&doc.elements[0], DocElement::Heading(h) if h.title == "Heading 1"));
        assert!(matches!(&doc.elements[1], DocElement::Paragraph(p) if p.line == 3));
        assert!(matches!(&doc.elements[2], DocElement::Req(r) if r.id == "my.req"));
        assert!(matches!(&doc.elements[3], DocElement::Paragraph(p) if p.line == 7));
        assert!(matches!(&doc.elements[4], DocElement::Heading(h) if h.title == "Heading 2"));
    }

    #[tokio::test]
    async fn test_paragraph_html_has_source_line_attribute() {
        let md = r#"First paragraph.

Second paragraph.

Third paragraph.
"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        // Check that paragraphs have data-source-line attributes
        assert!(
            doc.html.contains(r#"<p data-source-line="1">"#),
            "First paragraph should have data-source-line=\"1\": {}",
            doc.html
        );
        assert!(
            doc.html.contains(r#"<p data-source-line="3">"#),
            "Second paragraph should have data-source-line=\"3\": {}",
            doc.html
        );
        assert!(
            doc.html.contains(r#"<p data-source-line="5">"#),
            "Third paragraph should have data-source-line=\"5\": {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_paragraph_html_has_source_file_attribute() {
        let md = "A paragraph with source file info.";
        let opts = RenderOptions {
            source_path: Some("docs/test.md".to_string()),
            ..Default::default()
        };
        let doc = render(md, &opts).await.unwrap();

        assert!(
            doc.html.contains(r#"data-source-line="1""#),
            "Should have line attribute: {}",
            doc.html
        );
        assert!(
            doc.html.contains(r#"data-source-file="docs/test.md""#),
            "Should have file attribute: {}",
            doc.html
        );
    }

    // =========================================================================
    // Requirement marker stripping tests - comprehensive edge cases
    // =========================================================================

    #[test]
    fn test_strip_req_marker_basic() {
        assert_eq!(strip_req_marker("r[foo] bar"), "bar");
        assert_eq!(strip_req_marker("r[foo.bar] text"), "text");
        assert_eq!(strip_req_marker("r[foo]"), "");
        assert_eq!(
            strip_req_marker("r[foo.bar.baz status=stable] text"),
            "text"
        );
        assert_eq!(strip_req_marker("no marker here"), "no marker here");
        assert_eq!(strip_req_marker(""), "");
    }

    #[tokio::test]
    async fn test_req_marker_same_line() {
        // r[id] and text on same line
        let md = "r[same.line] This text is on the same line.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "same.line");
        // Should NOT contain the raw marker
        assert!(
            !doc.html.contains("r[same.line]"),
            "Raw marker should be stripped: {}",
            doc.html
        );
        assert!(
            !doc.html.contains("[same.line]"),
            "Marker brackets should be stripped from content: {}",
            doc.html
        );
        // Should contain the text
        assert!(
            doc.html.contains("This text is on the same line"),
            "Text should be present: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_req_marker_on_own_line() {
        // r[id] on its own line, text on next line
        let md = "r[own.line]\nText on next line.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "own.line");
        assert!(
            !doc.html.contains("r[own.line]"),
            "Raw marker should be stripped: {}",
            doc.html
        );
        assert!(
            !doc.html.contains("[own.line]"),
            "Marker brackets should be stripped: {}",
            doc.html
        );
        assert!(
            doc.html.contains("Text on next line"),
            "Text should be present: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_req_marker_with_blank_line() {
        // r[id] followed by blank line then text (separate paragraph)
        let md = "r[blank.after]\n\nText after blank line.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "blank.after");
        assert!(
            !doc.html.contains("r[blank.after]"),
            "Raw marker should be stripped: {}",
            doc.html
        );
        assert!(
            !doc.html.contains("[blank.after]"),
            "Marker brackets should be stripped: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_req_marker_with_metadata() {
        // r[id attr=value] with metadata
        let md = "r[meta.req status=stable level=must] Requirement with metadata.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "meta.req");
        assert!(
            !doc.html.contains("r[meta.req"),
            "Raw marker should be stripped: {}",
            doc.html
        );
        assert!(
            !doc.html.contains("status=stable"),
            "Metadata should be stripped from content: {}",
            doc.html
        );
        assert!(
            doc.html.contains("Requirement with metadata"),
            "Text should be present: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_req_in_blockquote_marker_stripped() {
        // > r[id] text - blockquote requirement
        let md = "> r[quote.req] Text in blockquote requirement.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "quote.req");
        assert!(
            !doc.html.contains("r[quote.req]"),
            "Raw marker should be stripped: {}",
            doc.html
        );
        assert!(
            !doc.html.contains("[quote.req]"),
            "Marker brackets should be stripped: {}",
            doc.html
        );
        assert!(
            doc.html.contains("Text in blockquote requirement"),
            "Text should be present: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_req_in_blockquote_multiline_marker_stripped() {
        // > r[id]
        // > text on next line
        let md = "> r[multiline.quote]\n> Text continues here.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "multiline.quote");
        assert!(
            !doc.html.contains("r[multiline.quote]"),
            "Raw marker should be stripped: {}",
            doc.html
        );
        assert!(
            !doc.html.contains("[multiline.quote]"),
            "Marker brackets should be stripped: {}",
            doc.html
        );
        assert!(
            doc.html.contains("Text continues here"),
            "Text should be present: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_multiple_reqs_markers_stripped() {
        // Multiple requirements in document
        let md = "r[first.req] First requirement text.\n\nr[second.req] Second requirement text.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert_eq!(doc.reqs.len(), 2);
        assert!(
            !doc.html.contains("r[first.req]"),
            "First marker should be stripped: {}",
            doc.html
        );
        assert!(
            !doc.html.contains("r[second.req]"),
            "Second marker should be stripped: {}",
            doc.html
        );
        assert!(
            doc.html.contains("First requirement text"),
            "First text should be present: {}",
            doc.html
        );
        assert!(
            doc.html.contains("Second requirement text"),
            "Second text should be present: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_req_only_marker_no_text() {
        // Just r[id] with nothing else
        let md = "r[lonely.req]";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "lonely.req");
        // The anchor link will contain [lonely.req] but not raw r[...]
        assert!(
            !doc.html.contains("r[lonely.req]"),
            "Raw marker should not appear: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_req_with_formatting_after_marker() {
        // r[id] with **bold** and *italic* after
        let md = "r[fmt.after] Text with **bold** and *italic*.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert_eq!(doc.reqs.len(), 1);
        assert!(
            !doc.html.contains("r[fmt.after]"),
            "Marker should be stripped: {}",
            doc.html
        );
        assert!(
            doc.html.contains("<strong>bold</strong>"),
            "Bold should be rendered: {}",
            doc.html
        );
        assert!(
            doc.html.contains("<em>italic</em>"),
            "Italic should be rendered: {}",
            doc.html
        );
    }
}
