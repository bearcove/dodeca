//! Main rendering pipeline.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, LinkType, MetadataBlockKind, Options, Parser,
    Tag, TagEnd,
};

use crate::Result;
use crate::frontmatter::{Frontmatter, FrontmatterFormat};
use crate::handler::{
    BoxedHandler, BoxedInlineCodeHandler, BoxedLinkResolver, BoxedReqHandler,
    BoxedShortcodeResolver, BoxedWikiLinkResolver, CodeBlockHandler, CodeBlockOutput,
    DefaultReqHandler, InlineCodeHandler, RawCodeHandler, ReqHandler, Shortcode, ShortcodeArgs,
    ShortcodeResolver, WikiLink, WikiLinkOutput, WikiLinkResolver, html_escape,
};
use crate::headings::{Heading, slugify};
use crate::links::resolve_link;
use crate::reqs::{InlineCodeSpan, ReqDefinition, RuleId, SourceSpan, parse_req_marker};

/// Parse context representing the current nested structure we're inside.
/// This replaces the ad-hoc state variables with a proper stack.
#[derive(Debug)]
#[allow(dead_code)] // Some fields are structural markers not yet used
enum ParseContext<'a> {
    /// Inside a metadata block (YAML/TOML frontmatter, or a `+++ :name: +++` shortcode)
    Metadata {
        kind: MetadataBlockKind,
        /// Buffered block text; routed at End to frontmatter or a shortcode.
        text: String,
    },

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
    /// A requirement definition
    Req(ReqDefinition),
    /// A regular paragraph (not a requirement)
    Paragraph(Paragraph),
}

#[derive(Default)]
struct HtmlRenderState {
    table_alignments: Vec<Alignment>,
    table_in_head: bool,
    table_cell_index: usize,
    blockquote_stack: Vec<Option<SourceId>>,
    list_stack: Vec<Option<SourceId>>,
    list_item_stack: Vec<Option<SourceId>>,
    definition_list_stack: Vec<Option<SourceId>>,
    definition_title: Option<SourceId>,
    definition_definition: Option<SourceId>,
    table: Option<SourceId>,
    table_head: Option<SourceId>,
    table_row: Option<SourceId>,
    table_cell: Option<SourceId>,
}

/// Options for rendering markdown.
#[derive(Default, Clone)]
pub struct RenderOptions {
    /// Source file path for relative link resolution.
    pub source_path: Option<String>,

    /// Whether to build a source map and emit `data-sid` attributes in HTML.
    ///
    /// This is useful for development tooling that maps rendered HTML elements
    /// back to markdown source, but production builds can leave it disabled to
    /// keep the rendered HTML clean.
    pub source_map: bool,

    /// Whether to render inline `<!-- note … -->` annotations.
    ///
    /// When `true` (development), note comments are rendered to an
    /// `<aside class="dodeca-note">` with their markdown body rendered inline.
    /// When `false` (production), note comments are stripped entirely so they
    /// never reach the served HTML.
    pub render_notes: bool,

    /// Code block handlers keyed by language
    pub code_handlers: HashMap<String, BoxedHandler>,

    /// Default handler for languages without a specific handler
    pub default_handler: Option<BoxedHandler>,

    /// Custom handler for rendering requirement definitions
    pub req_handler: Option<BoxedReqHandler>,

    /// Custom handler for rendering inline code spans
    pub inline_code_handler: Option<BoxedInlineCodeHandler>,

    /// Custom handler for resolving links (for dependency tracking)
    pub link_resolver: Option<BoxedLinkResolver>,

    /// Custom handler for resolving wiki-style links.
    pub wiki_link_resolver: Option<BoxedWikiLinkResolver>,

    /// Custom resolver for shortcodes (`+++ :name: … +++` and `*:name(args)*`).
    ///
    /// When `None`, shortcode syntax is left as written — marq renders nothing on its
    /// own, keeping it dependency-agnostic. The host registers a resolver that renders
    /// via its template engine inside a tracked query, preserving dependency tracking.
    pub shortcode_resolver: Option<BoxedShortcodeResolver>,
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

    /// Set the source file path for link resolution.
    pub fn with_source_path(mut self, path: &str) -> Self {
        self.source_path = Some(path.to_string());
        self
    }

    /// Configure whether rendered elements include source IDs and source-map entries.
    pub fn with_source_map(mut self, enabled: bool) -> Self {
        self.source_map = enabled;
        self
    }

    /// Configure whether inline `<!-- note … -->` annotations are rendered
    /// (development) or stripped (production).
    pub fn with_render_notes(mut self, enabled: bool) -> Self {
        self.render_notes = enabled;
        self
    }

    /// Set a custom handler for inline code spans.
    pub fn with_inline_code_handler<H: InlineCodeHandler + 'static>(mut self, handler: H) -> Self {
        self.inline_code_handler = Some(Arc::new(handler));
        self
    }

    /// Set a custom link resolver for dependency tracking.
    pub fn with_link_resolver<R: crate::handler::LinkResolver + 'static>(
        mut self,
        resolver: R,
    ) -> Self {
        self.link_resolver = Some(Arc::new(resolver));
        self
    }

    /// Set a custom wiki-link resolver.
    pub fn with_wiki_link_resolver<R: WikiLinkResolver + 'static>(mut self, resolver: R) -> Self {
        self.wiki_link_resolver = Some(Arc::new(resolver));
        self
    }

    /// Set a custom resolver for shortcodes.
    pub fn with_shortcode_resolver<R: ShortcodeResolver + 'static>(mut self, resolver: R) -> Self {
        self.shortcode_resolver = Some(Arc::new(resolver));
        self
    }
}

/// Opaque ID for a rendered HTML element that has a source-map entry.
///
/// Source IDs are scoped to one rendered [`Document`]. They are derived from
/// the markdown construct and source slice, so unrelated insertions elsewhere
/// in the document do not renumber every following element.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(String);

impl SourceId {
    /// Return the ID emitted in `data-sid`.
    pub fn get(&self) -> &str {
        &self.0
    }

    /// Return the ID emitted in `data-sid`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The markdown construct represented by a source-map entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Heading,
    Paragraph,
    BlockQuote,
    List,
    ListItem,
    DefinitionList,
    DefinitionListTitle,
    DefinitionListDefinition,
    ThematicBreak,
    Table,
    TableHead,
    TableRow,
    TableCell,
    Image,
}

impl SourceKind {
    fn stable_tag(self) -> &'static str {
        match self {
            SourceKind::Heading => "heading",
            SourceKind::Paragraph => "paragraph",
            SourceKind::BlockQuote => "blockquote",
            SourceKind::List => "list",
            SourceKind::ListItem => "list-item",
            SourceKind::DefinitionList => "definition-list",
            SourceKind::DefinitionListTitle => "definition-list-title",
            SourceKind::DefinitionListDefinition => "definition-list-definition",
            SourceKind::ThematicBreak => "thematic-break",
            SourceKind::Table => "table",
            SourceKind::TableHead => "table-head",
            SourceKind::TableRow => "table-row",
            SourceKind::TableCell => "table-cell",
            SourceKind::Image => "image",
        }
    }
}

/// Source information for one rendered HTML element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMapEntry {
    /// ID emitted as `data-sid`.
    pub id: SourceId,
    /// Markdown construct represented by this entry.
    pub kind: SourceKind,
    /// Inclusive 1-indexed starting line.
    pub line_start: usize,
    /// Inclusive 1-indexed ending line.
    pub line_end: usize,
    /// Inclusive starting byte offset in the source markdown.
    pub byte_start: usize,
    /// Exclusive ending byte offset in the source markdown.
    pub byte_end: usize,
}

/// Sidecar map from rendered `data-sid` attributes back to markdown spans.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceMap {
    /// Source path for all entries, when provided in [`RenderOptions`].
    pub source_path: Option<String>,
    /// Entries in render order.
    pub entries: Vec<SourceMapEntry>,
}

impl SourceMap {
    /// Look up one source-map entry by ID.
    pub fn get(&self, id: &SourceId) -> Option<&SourceMapEntry> {
        self.entries.iter().find(|entry| &entry.id == id)
    }

    /// Look up one source-map entry by the value read from a `data-sid` attribute.
    pub fn get_by_sid(&self, sid: &str) -> Option<&SourceMapEntry> {
        self.entries.iter().find(|entry| entry.id.as_str() == sid)
    }
}

/// Render inline code, using the handler if available.
fn render_inline_code(code: &str, handler: Option<&BoxedInlineCodeHandler>) -> String {
    if let Some(h) = handler
        && let Some(rendered) = h.render(code)
    {
        return rendered;
    }
    // Default rendering
    format!("<code>{}</code>", html_escape(code))
}

/// Render a LaTeX math fragment to a MathML string via `pulldown-latex`.
///
/// `block` selects display mode (`<math display="block">` with `displaystyle`
/// sizing) vs inline. Invalid LaTeX is rendered by `pulldown-latex` itself as a
/// visible error node (in its error colour), so the author sees the mistake
/// rather than a silent drop; the `Err` arm only fires on a writer failure,
/// which a `String` sink never produces, but we fall back to the escaped source
/// for total safety.
fn tex_to_mathml(tex: &str, block: bool) -> String {
    use pulldown_latex::{
        Parser as LatexParser, Storage,
        config::{DisplayMode, RenderConfig},
        mathml::push_mathml,
    };

    let storage = Storage::new();
    let parser = LatexParser::new(tex, &storage);
    let config = RenderConfig {
        display_mode: if block {
            DisplayMode::Block
        } else {
            DisplayMode::Inline
        },
        ..Default::default()
    };

    let mut out = String::new();
    if push_mathml(&mut out, parser, config).is_err() {
        out.clear();
        out.push_str("<code class=\"math-error\">");
        out.push_str(&html_escape(tex));
        out.push_str("</code>");
    }
    out
}

/// Resolve a link using the custom resolver if available, otherwise use default resolution.
async fn resolve_link_with_resolver(
    link: &str,
    source_path: Option<&str>,
    resolver: Option<&BoxedLinkResolver>,
) -> String {
    // Try custom resolver first
    if let Some(r) = resolver
        && let Some(resolved) = r.resolve(link, source_path).await
    {
        return resolved;
    }
    // Fall back to default resolution
    resolve_link(link, source_path)
}

fn render_text(html: &mut String, text: &str) {
    html.push_str(&html_escape(text));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveLink {
    Regular,
    WikiResolved,
    WikiLiteral,
}

async fn render_link_start(
    html: &mut String,
    link_type: &LinkType,
    dest_url: &str,
    title: &str,
    options: &RenderOptions,
) -> ActiveLink {
    if let LinkType::WikiLink { has_pothole } = link_type {
        return render_wiki_link_start(html, dest_url, *has_pothole, options).await;
    }

    let resolved = resolve_link_with_resolver(
        dest_url,
        options.source_path.as_deref(),
        options.link_resolver.as_ref(),
    )
    .await;
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
    ActiveLink::Regular
}

fn render_link_end(html: &mut String, active_link: ActiveLink) {
    match active_link {
        ActiveLink::Regular | ActiveLink::WikiResolved => html.push_str("</a>"),
        ActiveLink::WikiLiteral => html.push_str("]]"),
    }
}

async fn render_wiki_link_start(
    html: &mut String,
    target: &str,
    has_label: bool,
    options: &RenderOptions,
) -> ActiveLink {
    let link = WikiLink {
        target: target.to_string(),
    };

    let Some(resolver) = options.wiki_link_resolver.as_ref() else {
        render_wiki_link_literal_start(html, target, has_label);
        return ActiveLink::WikiLiteral;
    };

    match resolver
        .resolve(&link, options.source_path.as_deref())
        .await
    {
        Some(output) => {
            render_wiki_link_anchor_start(html, &output);
            ActiveLink::WikiResolved
        }
        None => {
            render_wiki_link_literal_start(html, target, has_label);
            ActiveLink::WikiLiteral
        }
    }
}

fn render_wiki_link_literal_start(html: &mut String, target: &str, has_label: bool) {
    if has_label {
        html.push_str("[[");
        html.push_str(&html_escape(target));
        html.push('|');
    } else {
        html.push_str("[[");
    }
}

fn render_wiki_link_anchor_start(html: &mut String, output: &WikiLinkOutput) {
    html.push_str("<a href=\"");
    html.push_str(&html_escape(&output.href));
    html.push('"');
    for (name, value) in &output.attrs {
        if is_valid_html_attr_name(name) {
            html.push(' ');
            html.push_str(name);
            html.push_str("=\"");
            html.push_str(&html_escape(value));
            html.push('"');
        }
    }
    html.push('>');
}

fn is_valid_html_attr_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
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

    /// HTML snippets to inject into the page's `<head>` (or body end).
    /// Already deduplicated by key during rendering.
    pub head_injections: Vec<String>,

    /// All inline code spans (backtick-delimited) found in the document.
    /// Spans include byte offsets covering the backtick delimiters.
    pub inline_code_spans: Vec<InlineCodeSpan>,

    /// Source map for rendered elements with `data-sid` attributes.
    pub source_map: SourceMap,
}

/// Convert a byte offset to a 1-indexed line number.
fn offset_to_line(content: &str, offset: usize) -> usize {
    content[..offset.min(content.len())].matches('\n').count() + 1
}

fn offset_to_end_line(content: &str, end_offset: usize) -> usize {
    if end_offset == 0 {
        1
    } else {
        offset_to_line(content, end_offset.saturating_sub(1))
    }
}

struct SourceMapBuilder {
    enabled: bool,
    map: SourceMap,
    seen_ids: BTreeMap<String, usize>,
    next_placeholder: usize,
    replacements: Vec<(SourceId, SourceId)>,
}

impl SourceMapBuilder {
    fn new(options: &RenderOptions) -> Self {
        Self {
            enabled: options.source_map,
            map: SourceMap {
                source_path: options
                    .source_map
                    .then(|| options.source_path.clone())
                    .flatten(),
                entries: Vec::new(),
            },
            seen_ids: BTreeMap::new(),
            next_placeholder: 1,
            replacements: Vec::new(),
        }
    }

    fn finish(self, html: &mut String) -> SourceMap {
        for (placeholder, id) in &self.replacements {
            let from = format!("data-sid=\"{}\"", placeholder);
            let to = format!("data-sid=\"{}\"", id);
            *html = html.replace(&from, &to);
        }
        self.map
    }

    fn span_attr(&mut self, kind: SourceKind, range: Range<usize>, markdown: &str) -> String {
        let Some(id) = self.push_entry(kind, range, markdown) else {
            return String::new();
        };
        format!(" data-sid=\"{}\"", id)
    }

    fn open_attr(
        &mut self,
        kind: SourceKind,
        range: &Range<usize>,
        markdown: &str,
    ) -> (Option<SourceId>, String) {
        let Some(id) = self.push_open_entry(kind, range.clone(), markdown) else {
            return (None, String::new());
        };
        let attrs = format!(" data-sid=\"{}\"", id);
        (Some(id), attrs)
    }

    fn close(&mut self, id: Option<SourceId>, range: &Range<usize>, markdown: &str) {
        let Some(id) = id else {
            return;
        };
        let Some(index) = self.map.entry_index(&id) else {
            return;
        };
        let kind = self.map.entries[index].kind;
        let source_range = self.map.entries[index].byte_start..range.end;
        let final_id = self.source_id(kind, &source_range, markdown);
        let placeholder = std::mem::replace(&mut self.map.entries[index].id, final_id.clone());
        self.replacements.push((placeholder, final_id));

        let entry = &mut self.map.entries[index];
        entry.byte_end = range.end;
        entry.line_end = offset_to_end_line(markdown, range.end);
    }

    fn push_entry(
        &mut self,
        kind: SourceKind,
        range: Range<usize>,
        markdown: &str,
    ) -> Option<SourceId> {
        if !self.enabled {
            return None;
        }

        let id = self.source_id(kind, &range, markdown);
        self.map.entries.push(SourceMapEntry {
            id: id.clone(),
            kind,
            line_start: offset_to_line(markdown, range.start),
            line_end: offset_to_end_line(markdown, range.end),
            byte_start: range.start,
            byte_end: range.end,
        });
        Some(id)
    }

    fn push_open_entry(
        &mut self,
        kind: SourceKind,
        range: Range<usize>,
        markdown: &str,
    ) -> Option<SourceId> {
        if !self.enabled {
            return None;
        }

        let id = self.placeholder_id();
        self.map.entries.push(SourceMapEntry {
            id: id.clone(),
            kind,
            line_start: offset_to_line(markdown, range.start),
            line_end: offset_to_end_line(markdown, range.end),
            byte_start: range.start,
            byte_end: range.end,
        });
        Some(id)
    }

    fn source_id(&mut self, kind: SourceKind, range: &Range<usize>, markdown: &str) -> SourceId {
        let source = &markdown[range.start..range.end];
        let base = stable_source_id(kind, source);
        let count = self.seen_ids.entry(base.clone()).or_default();
        *count += 1;
        if *count == 1 {
            SourceId(base)
        } else {
            SourceId(format!("{base}-{}", *count))
        }
    }

    fn placeholder_id(&mut self) -> SourceId {
        let id = SourceId(format!("__marq-source-{}__", self.next_placeholder));
        self.next_placeholder += 1;
        id
    }
}

impl SourceMap {
    fn entry_index(&self, id: &SourceId) -> Option<usize> {
        self.entries.iter().position(|entry| &entry.id == id)
    }
}

fn stable_source_id(kind: SourceKind, source: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET;
    for byte in kind
        .stable_tag()
        .as_bytes()
        .iter()
        .copied()
        .chain([0])
        .chain(source.as_bytes().iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("s{hash:016x}")
}

/// Detect a fenced shortcode and return its name.
///
/// A `+++ … +++` (pluses) metadata block is a shortcode when its first non-empty
/// line is a YAML key of the form `:name:` (the leading `:` is the shortcode marker).
/// Returns the name (without colons) borrowed from `text`, or `None` for ordinary
/// TOML frontmatter. The whole block is later handed to the resolver as YAML.
fn parse_fenced_shortcode_name(text: &str) -> Option<&str> {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let name = first.strip_prefix(':')?;
    let name = name.strip_suffix(':').unwrap_or(name).trim();
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
    valid.then_some(name)
}

/// Parse an inline/blockquote shortcode marker `name(k=v, k2="v 2")`.
///
/// The leading `:` must already be stripped. Returns the name and the `key=value`
/// pairs in source order. Values may be bare or double-quoted (quotes let a value
/// contain commas/spaces). Unparenthesised markers (`name`) yield no pairs.
fn parse_emphasis_shortcode(input: &str) -> (String, Vec<(String, String)>) {
    let input = input.trim();
    let Some((name, rest)) = input.split_once('(') else {
        return (input.to_string(), Vec::new());
    };
    let args = rest.strip_suffix(')').unwrap_or(rest);

    let mut pairs = Vec::new();
    let mut chars = args.char_indices().peekable();
    // Split into `key = value` items, honoring quotes so a quoted value can hold commas.
    let mut item = String::new();
    let mut in_quotes = false;
    let flush = |item: &str, pairs: &mut Vec<(String, String)>| {
        let item = item.trim();
        if item.is_empty() {
            return;
        }
        if let Some((k, v)) = item.split_once('=') {
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(v);
            pairs.push((k.trim().to_string(), v.to_string()));
        } else {
            pairs.push((item.to_string(), String::new()));
        }
    };
    while let Some((_, c)) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                item.push(c);
            }
            ',' if !in_quotes => {
                flush(&item, &mut pairs);
                item.clear();
            }
            _ => item.push(c),
        }
    }
    flush(&item, &mut pairs);

    (name.trim().to_string(), pairs)
}

/// Detect a body shortcode at the head of a buffered blockquote.
///
/// A body shortcode is a blockquote whose first paragraph is a single emphasis whose
/// text starts with `:` (e.g. `> *:bearsays*`). On match, returns the marker text
/// (with the leading `:` removed) and the remaining blockquote events as a body to be
/// rendered — the marker paragraph itself is dropped. `events` is the full blockquote
/// run, `[Start(BlockQuote), … , End(BlockQuote)]`.
fn extract_body_shortcode<'a>(
    events: &[(Event<'a>, Range<usize>)],
) -> Option<(String, Vec<(Event<'a>, Range<usize>)>)> {
    // events[0] is Start(BlockQuote); the first paragraph must open with an emphasis.
    if !matches!(events.first(), Some((Event::Start(Tag::BlockQuote(_)), _))) {
        return None;
    }
    if !matches!(events.get(1), Some((Event::Start(Tag::Paragraph), _)))
        || !matches!(events.get(2), Some((Event::Start(Tag::Emphasis), _)))
    {
        return None;
    }

    // Collect the marker text inside that first emphasis.
    let mut marker = String::new();
    let mut idx = 3;
    while let Some((ev, _)) = events.get(idx) {
        match ev {
            Event::Text(t) | Event::Code(t) => marker.push_str(t),
            Event::End(TagEnd::Emphasis) => break,
            // Anything else inside the marker emphasis means it isn't a shortcode.
            _ => return None,
        }
        idx += 1;
    }
    let name_args = marker.trim().strip_prefix(':')?.to_string();
    if name_args.is_empty() {
        return None;
    }

    // The marker paragraph ends at the next End(Paragraph); the body is whatever
    // follows, re-wrapped in a blockquote run for `render_blockquote_req_content`.
    let para_end = events
        .iter()
        .position(|(ev, _)| matches!(ev, Event::End(TagEnd::Paragraph)))?;
    let mut body = Vec::with_capacity(events.len() - para_end);
    body.push(events[0].clone());
    body.extend(events[para_end + 1..].iter().cloned());

    Some((name_args, body))
}

/// Resolve inline `*:name(args)*` shortcodes (no body) within an inline event run.
///
/// Returns a new event list where each emphasis whose text starts with `:` is replaced
/// by the resolver's HTML (as an `InlineHtml` event). Emphasis that is not a shortcode —
/// or that the resolver declines — is left untouched, so ordinary `*emphasis*` still
/// renders as `<em>`. With no resolver registered, the events are returned unchanged.
///
/// Head injections from inline shortcodes are dropped: these are speech-bubble style
/// shortcodes that need none. Fenced shortcodes (handled in the main pass) collect theirs.
async fn resolve_inline_shortcodes<'a>(
    events: &[(Event<'a>, Range<usize>)],
    options: &RenderOptions,
) -> Result<Vec<(Event<'a>, Range<usize>)>> {
    let Some(resolver) = &options.shortcode_resolver else {
        return Ok(events.to_vec());
    };

    let mut out = Vec::with_capacity(events.len());
    let mut i = 0;
    while i < events.len() {
        if matches!(events[i].0, Event::Start(Tag::Emphasis)) {
            // A shortcode emphasis holds only text/code until its End(Emphasis).
            let mut text = String::new();
            let mut j = i + 1;
            let mut text_only = true;
            while j < events.len() {
                match &events[j].0 {
                    Event::Text(t) | Event::Code(t) => text.push_str(t),
                    Event::End(TagEnd::Emphasis) => break,
                    _ => {
                        text_only = false;
                        break;
                    }
                }
                j += 1;
            }
            let closed = j < events.len() && matches!(events[j].0, Event::End(TagEnd::Emphasis));
            if text_only
                && closed
                && let Some(name_args) = text.trim().strip_prefix(':')
                && !name_args.is_empty()
            {
                let (name, pairs) = parse_emphasis_shortcode(name_args);
                let args = ShortcodeArgs::Pairs(pairs);
                if let Some(output) = resolver
                    .resolve(Shortcode {
                        name: &name,
                        args: &args,
                        body: None,
                    })
                    .await?
                {
                    let range = events[i].1.start..events[j].1.end;
                    out.push((Event::InlineHtml(output.html.into()), range));
                    i = j + 1;
                    continue;
                }
            }
        }
        out.push(events[i].clone());
        i += 1;
    }
    Ok(out)
}

/// Render markdown to HTML.
///
/// # Example
///
/// ```rust,ignore
/// use marq::{render, RenderOptions};
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
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_MATH;

    // Convert `$…$` / `$$…$$` math to MathML right at the source, before any
    // downstream consumer sees the events: every catch-all renders `InlineHtml`
    // verbatim, so the rest of the pipeline needs no math awareness. Both inline
    // and display math stay `InlineHtml` (paragraph-safe); the `display="block"`
    // attribute, not a block-level event, drives display rendering.
    let parser = Parser::new_ext(markdown, parser_options)
        .into_offset_iter()
        .map(|(event, range)| {
            let event = match event {
                Event::InlineMath(tex) => Event::InlineHtml(tex_to_mathml(&tex, false).into()),
                Event::DisplayMath(tex) => Event::InlineHtml(tex_to_mathml(&tex, true).into()),
                other => other,
            };
            (event, range)
        });

    // Collected data
    let mut headings: Vec<Heading> = Vec::new();
    let mut reqs: Vec<ReqDefinition> = Vec::new();
    let mut elements: Vec<DocElement> = Vec::new();
    let mut code_samples: Vec<CodeSample> = Vec::new();
    let mut inline_code_spans: Vec<InlineCodeSpan> = Vec::new();
    let mut head_injection_map: BTreeMap<String, String> = BTreeMap::new();
    let mut html_state = HtmlRenderState::default();
    let mut source_map = SourceMapBuilder::new(options);

    // Output HTML - built directly as we process
    let mut html = String::new();

    // Metadata tracking (document-level, not nested)
    let mut raw_metadata: Option<String> = None;
    let mut metadata_format: Option<FrontmatterFormat> = None;

    // Track parent heading slugs for hierarchical IDs
    let mut heading_stack: Vec<(u8, String)> = Vec::new();

    // Track seen req IDs for duplicate detection
    let mut seen_req_ids: std::collections::HashSet<RuleId> = std::collections::HashSet::new();
    let mut seen_req_bases: std::collections::HashSet<String> = std::collections::HashSet::new();

    // The context stack
    let mut context_stack: Vec<ParseContext<'_>> = Vec::new();
    let mut inline_link_stack: Vec<ActiveLink> = Vec::new();

    // True while we are skipping the inner `Event::Html` lines of an inline
    // note comment (handled up-front at its `Start(HtmlBlock)`).
    let mut in_note_block = false;

    // Default req handler
    let default_req_handler: Arc<dyn ReqHandler> = Arc::new(DefaultReqHandler);
    let req_handler = options.req_handler.as_ref().unwrap_or(&default_req_handler);

    // Default code handler
    let default_code_handler: BoxedHandler = Arc::new(RawCodeHandler);

    // Helper to check if inside blockquote
    let is_inside_blockquote =
        |stack: &[ParseContext<'_>]| stack_contains(stack, |c| c.is_blockquote());

    for (event, range) in parser {
        // Collect all inline code spans centrally. pulldown_cmark only emits
        // Event::Code for genuine backtick spans, never for fenced code block
        // content, so this naturally excludes code blocks (even blockquoted ones).
        if let Event::Code(code) = &event {
            inline_code_spans.push(InlineCodeSpan {
                content: code.to_string(),
                span: SourceSpan {
                    offset: range.start,
                    length: range.len(),
                },
            });
        }

        // While inside an inline note comment, swallow its inner `Html` line
        // events; the whole block was already handled at its `Start(HtmlBlock)`.
        if in_note_block {
            if matches!(event, Event::End(TagEnd::HtmlBlock)) {
                in_note_block = false;
            }
            continue;
        }

        // Intercept inline note comments before they reach the generic raw-HTML
        // path. `Start(HtmlBlock)` carries the full block range, so the entire
        // comment is available in one shot.
        if let Event::Start(Tag::HtmlBlock) = &event
            && let Some(note) = crate::note::parse_note(&markdown[range.clone()])
        {
            in_note_block = true;
            if options.render_notes {
                let mut body_opts = options.clone();
                body_opts.source_map = false; // note bodies don't pollute the page map
                body_opts.render_notes = false; // notes don't nest
                let rendered = Box::pin(render(&note.body, &body_opts)).await?;
                html.push_str(&crate::note::render_aside(&note.meta, &rendered.html));
            }
            // Otherwise (production): strip the note entirely.
            continue;
        }

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
                        if let Some((prefix, _, _)) = parse_req_leading_marker(trimmed) {
                            // Find the actual marker position in the source (after the > prefix)
                            let marker = format!("{}[", prefix);
                            let marker_offset = markdown[start_offset..]
                                .find(&marker)
                                .map(|i| start_offset + i)
                                .unwrap_or(start_offset);
                            if let Some(req_result) = try_parse_blockquote_req(
                                trimmed,
                                markdown,
                                marker_offset,
                                range.end,
                                &mut seen_req_ids,
                                &mut seen_req_bases,
                            ) {
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
                        }

                        // Check if this is a body shortcode (`> *:name*` + body).
                        if let Some(resolver) = &options.shortcode_resolver
                            && let Some((name_args, body_events)) = extract_body_shortcode(&events)
                        {
                            let body_html = render_blockquote_req_content(
                                &body_events,
                                options,
                                &default_code_handler,
                            )
                            .await?;
                            let (name, pairs) = parse_emphasis_shortcode(&name_args);
                            let args = ShortcodeArgs::Pairs(pairs);
                            let rendered = resolver
                                .resolve(Shortcode {
                                    name: &name,
                                    args: &args,
                                    body: Some(&body_html),
                                })
                                .await?;
                            if let Some(output) = rendered {
                                if is_inside_blockquote(&context_stack) {
                                    if let Some(ParseContext::BlockQuote {
                                        events: parent_events,
                                        ..
                                    }) = context_stack.last_mut()
                                    {
                                        parent_events
                                            .push((Event::Html(output.html.into()), range));
                                    }
                                } else {
                                    html.push_str(&output.html);
                                }
                                for inj in output.head_injections {
                                    head_injection_map.entry(inj.key).or_insert(inj.html);
                                }
                                continue;
                            }
                            // Declined: fall through to render as a normal blockquote.
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
                            let events = resolve_inline_shortcodes(&events, options).await?;
                            render_events_to_html(
                                &mut html,
                                &events,
                                options,
                                markdown,
                                &mut source_map,
                            )
                            .await;
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
                    let source_range = start_offset..range.end;
                    html.push_str(&format!(
                        "<h{} id=\"{}\"{}>{}</h{}>",
                        current_level,
                        html_escape(&id),
                        source_map.span_attr(SourceKind::Heading, source_range, markdown),
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
                    if parse_req_leading_marker(trimmed).is_some()
                        && let Some(req_result) = try_parse_paragraph_req(
                            trimmed,
                            markdown,
                            start_offset,
                            &mut seen_req_ids,
                            &mut seen_req_bases,
                            &events,
                        )
                    {
                        match req_result {
                            Ok(mut req) => {
                                // Render req content HTML
                                let content_html =
                                    render_paragraph_req_content(&events, options).await;

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
                    let events = resolve_inline_shortcodes(&events, options).await?;
                    render_events_to_html(&mut html, &events, options, markdown, &mut source_map)
                        .await;
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

                    // Strip trailing newline from code - markdown typically includes
                    // a newline before the closing ``` fence, which would otherwise
                    // render as extra whitespace inside the <code> element.
                    let code_trimmed = code.trim_end_matches('\n');
                    let CodeBlockOutput {
                        html: rendered,
                        head_injections,
                    } = handler.render(&base_language, code_trimmed).await?;
                    html.push_str(&rendered);
                    for inj in head_injections {
                        head_injection_map.entry(inj.key).or_insert(inj.html);
                    }

                    code_samples.push(CodeSample {
                        line,
                        language: full_language,
                        code,
                    });
                }
            }

            // ===== Metadata blocks =====
            // A metadata block is either document frontmatter (`---` YAML or leading
            // `+++` TOML) or a fenced shortcode (`+++ :name: <yaml> +++`). The two are
            // told apart by content at End: a pluses block whose first line is a
            // `:name` key is a shortcode; anything else is frontmatter. `metadata_format`
            // / `raw_metadata` are therefore set at End, only for real frontmatter.
            Event::Start(Tag::MetadataBlock(kind)) => {
                context_stack.push(ParseContext::Metadata {
                    kind: *kind,
                    text: String::new(),
                });
            }
            Event::End(TagEnd::MetadataBlock(_)) => {
                let Some(ParseContext::Metadata { kind, text }) = context_stack.pop() else {
                    unreachable!("MetadataBlock end without matching Metadata context");
                };

                let shortcode = matches!(kind, MetadataBlockKind::PlusesStyle)
                    .then(|| parse_fenced_shortcode_name(&text))
                    .flatten();

                if let Some(name) = shortcode {
                    // Fenced shortcode: pass the whole block as YAML (a single
                    // `:name` mapping), let the resolver parse + render it.
                    let args = ShortcodeArgs::Yaml(text.clone());
                    let rendered = match &options.shortcode_resolver {
                        Some(resolver) => {
                            resolver
                                .resolve(Shortcode {
                                    name,
                                    args: &args,
                                    body: None,
                                })
                                .await?
                        }
                        None => None,
                    };
                    match rendered {
                        Some(output) => {
                            html.push_str(&output.html);
                            for inj in output.head_injections {
                                head_injection_map.entry(inj.key).or_insert(inj.html);
                            }
                        }
                        // Declined / no resolver: leave the source verbatim so nothing
                        // is silently dropped.
                        None => html.push_str(&markdown[range.clone()]),
                    }
                } else {
                    // Real document frontmatter.
                    metadata_format = Some(match kind {
                        MetadataBlockKind::YamlStyle => FrontmatterFormat::Yaml,
                        MetadataBlockKind::PlusesStyle => FrontmatterFormat::Toml,
                    });
                    raw_metadata = Some(text);
                }
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
                Some(ParseContext::Metadata { text: buf, .. }) => {
                    buf.push_str(text);
                }
                Some(ParseContext::BlockQuote { .. }) => {
                    unreachable!("BlockQuote text should be handled in blockquote branch");
                }
                None => {
                    if inline_link_stack.is_empty() {
                        render_text(&mut html, text);
                    } else {
                        html.push_str(&html_escape(text));
                    }
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
                    html.push_str(&render_inline_code(
                        code,
                        options.inline_code_handler.as_ref(),
                    ));
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

            // ===== Links (must be handled explicitly for @/ resolution) =====
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            }) => {
                if let Some(ParseContext::Paragraph { events, .. }) = context_stack.last_mut() {
                    events.push((event, range));
                } else if !stack_contains(&context_stack, |c| c.is_metadata()) {
                    let active_link =
                        render_link_start(&mut html, link_type, dest_url, title, options).await;
                    inline_link_stack.push(active_link);
                }
            }
            Event::End(TagEnd::Link) => {
                if let Some(ParseContext::Paragraph { events, .. }) = context_stack.last_mut() {
                    events.push((event, range));
                } else if !stack_contains(&context_stack, |c| c.is_metadata()) {
                    let active_link = inline_link_stack.pop().unwrap_or(ActiveLink::Regular);
                    render_link_end(&mut html, active_link);
                }
            }

            // ===== Everything else =====
            _ => {
                if let Some(ParseContext::Paragraph { events, .. }) = context_stack.last_mut() {
                    events.push((event, range));
                } else if !stack_contains(&context_stack, |c| c.is_metadata()) {
                    // Render directly using pulldown_cmark for other events
                    if !render_source_block_event(
                        &mut html,
                        &event,
                        &range,
                        markdown,
                        &mut html_state,
                        &mut source_map,
                    ) {
                        pulldown_cmark::html::push_html(&mut html, std::iter::once(event.clone()));
                    }
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

    let source_map = source_map.finish(&mut html);

    // In production (notes off), strip note highlight wrappers so they leave no
    // trace in the served HTML. Note comments are already stripped inline above.
    if !options.render_notes {
        html = crate::note::strip_marks(&html);
    }

    Ok(Document {
        raw_metadata,
        metadata_format,
        frontmatter,
        html,
        headings,
        reqs,
        code_samples,
        elements,
        head_injections: head_injection_map.into_values().collect(),
        inline_code_spans,
        source_map,
    })
}

/// Render a list of events to HTML string
async fn render_events_to_html(
    html: &mut String,
    events: &[(Event<'_>, Range<usize>)],
    options: &RenderOptions,
    markdown: &str,
    source_map: &mut SourceMapBuilder,
) {
    let mut html_state = HtmlRenderState::default();
    let mut link_stack: Vec<ActiveLink> = Vec::new();
    let mut i = 0;
    while i < events.len() {
        let (event, range) = &events[i];
        match event {
            Event::Start(Tag::Paragraph) => {
                let source_range = matching_end_range(events, i, TagEnd::Paragraph)
                    .map(|end_range| range.start..end_range.end)
                    .unwrap_or_else(|| range.clone());
                let attrs = source_map.span_attr(SourceKind::Paragraph, source_range, markdown);
                html.push_str(&format!("<p{}>", attrs));
            }
            Event::End(TagEnd::Paragraph) => {
                html.push_str("</p>\n");
            }
            Event::Text(text) => {
                if link_stack.is_empty() {
                    render_text(html, text);
                } else {
                    html.push_str(&html_escape(text));
                }
            }
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                let source_start = range.start;
                let mut source_end = range.end;
                let mut alt_text = String::new();
                i += 1;
                while i < events.len() {
                    match &events[i].0 {
                        Event::End(TagEnd::Image) => {
                            source_end = events[i].1.end;
                            break;
                        }
                        Event::Text(t) => alt_text.push_str(t),
                        Event::Code(c) => alt_text.push_str(c),
                        Event::SoftBreak | Event::HardBreak => alt_text.push(' '),
                        _ => {}
                    }
                    i += 1;
                }
                let title_attr = if title.is_empty() {
                    String::new()
                } else {
                    format!(" title=\"{}\"", html_escape(title))
                };
                let attrs =
                    source_map.span_attr(SourceKind::Image, source_start..source_end, markdown);
                html.push_str(&format!(
                    "<img{} src=\"{}\" alt=\"{}\"{} />",
                    attrs,
                    html_escape(dest_url),
                    html_escape(&alt_text),
                    title_attr
                ));
            }
            Event::End(TagEnd::Image) => {
                // Already handled by Start(Tag::Image)
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            }) => {
                let active_link =
                    render_link_start(html, link_type, dest_url, title, options).await;
                link_stack.push(active_link);
            }
            Event::End(TagEnd::Link) => {
                let active_link = link_stack.pop().unwrap_or(ActiveLink::Regular);
                render_link_end(html, active_link);
            }
            Event::Code(code) => {
                html.push_str(&render_inline_code(
                    code,
                    options.inline_code_handler.as_ref(),
                ));
            }
            _ => {
                if !render_source_block_event(
                    html,
                    event,
                    range,
                    markdown,
                    &mut html_state,
                    source_map,
                ) {
                    pulldown_cmark::html::push_html(html, std::iter::once(event.clone()));
                }
            }
        }
        i += 1;
    }
}

fn matching_end_range<'a>(
    events: &'a [(Event<'_>, Range<usize>)],
    start: usize,
    end: TagEnd,
) -> Option<&'a Range<usize>> {
    events
        .iter()
        .skip(start + 1)
        .find_map(|(event, range)| match event {
            Event::End(tag) if *tag == end => Some(range),
            _ => None,
        })
}

fn render_source_block_event(
    html: &mut String,
    event: &Event<'_>,
    range: &Range<usize>,
    markdown: &str,
    state: &mut HtmlRenderState,
    source_map: &mut SourceMapBuilder,
) -> bool {
    match event {
        Event::Start(Tag::BlockQuote(kind)) => {
            ensure_block_boundary(html);
            let (sid, attrs) = source_map.open_attr(SourceKind::BlockQuote, range, markdown);
            state.blockquote_stack.push(sid);
            let class_attr = blockquote_class_attr(*kind);
            html.push_str(&format!("<blockquote{}{}>\n", class_attr, attrs));
            true
        }
        Event::End(TagEnd::BlockQuote(_)) => {
            let sid = state.blockquote_stack.pop().flatten();
            source_map.close(sid, range, markdown);
            html.push_str("</blockquote>\n");
            true
        }
        Event::Start(Tag::List(Some(1))) => {
            ensure_block_boundary(html);
            let (sid, attrs) = source_map.open_attr(SourceKind::List, range, markdown);
            state.list_stack.push(sid);
            html.push_str(&format!("<ol{}>\n", attrs));
            true
        }
        Event::Start(Tag::List(Some(start))) => {
            ensure_block_boundary(html);
            let (sid, attrs) = source_map.open_attr(SourceKind::List, range, markdown);
            state.list_stack.push(sid);
            html.push_str(&format!("<ol start=\"{}\"{}>\n", start, attrs));
            true
        }
        Event::Start(Tag::List(None)) => {
            ensure_block_boundary(html);
            let (sid, attrs) = source_map.open_attr(SourceKind::List, range, markdown);
            state.list_stack.push(sid);
            html.push_str(&format!("<ul{}>\n", attrs));
            true
        }
        Event::End(TagEnd::List(true)) => {
            let sid = state.list_stack.pop().flatten();
            source_map.close(sid, range, markdown);
            html.push_str("</ol>\n");
            true
        }
        Event::End(TagEnd::List(false)) => {
            let sid = state.list_stack.pop().flatten();
            source_map.close(sid, range, markdown);
            html.push_str("</ul>\n");
            true
        }
        Event::Start(Tag::Item) => {
            if !html.ends_with('\n') {
                html.push('\n');
            }
            let (sid, attrs) = source_map.open_attr(SourceKind::ListItem, range, markdown);
            state.list_item_stack.push(sid);
            html.push_str(&format!("<li{}>", attrs));
            true
        }
        Event::End(TagEnd::Item) => {
            let sid = state.list_item_stack.pop().flatten();
            source_map.close(sid, range, markdown);
            html.push_str("</li>\n");
            true
        }
        Event::Start(Tag::DefinitionList) => {
            ensure_block_boundary(html);
            let (sid, attrs) = source_map.open_attr(SourceKind::DefinitionList, range, markdown);
            state.definition_list_stack.push(sid);
            html.push_str(&format!("<dl{}>\n", attrs));
            true
        }
        Event::End(TagEnd::DefinitionList) => {
            let sid = state.definition_list_stack.pop().flatten();
            source_map.close(sid, range, markdown);
            html.push_str("</dl>\n");
            true
        }
        Event::Start(Tag::DefinitionListTitle) => {
            if !html.ends_with('\n') {
                html.push('\n');
            }
            let (sid, attrs) =
                source_map.open_attr(SourceKind::DefinitionListTitle, range, markdown);
            state.definition_title = sid;
            html.push_str(&format!("<dt{}>", attrs));
            true
        }
        Event::End(TagEnd::DefinitionListTitle) => {
            source_map.close(state.definition_title.take(), range, markdown);
            html.push_str("</dt>\n");
            true
        }
        Event::Start(Tag::DefinitionListDefinition) => {
            if !html.ends_with('\n') {
                html.push('\n');
            }
            let (sid, attrs) =
                source_map.open_attr(SourceKind::DefinitionListDefinition, range, markdown);
            state.definition_definition = sid;
            html.push_str(&format!("<dd{}>", attrs));
            true
        }
        Event::End(TagEnd::DefinitionListDefinition) => {
            source_map.close(state.definition_definition.take(), range, markdown);
            html.push_str("</dd>\n");
            true
        }
        Event::Rule => {
            ensure_block_boundary(html);
            let attrs = source_map.span_attr(SourceKind::ThematicBreak, range.clone(), markdown);
            html.push_str(&format!("<hr{} />\n", attrs));
            true
        }
        Event::Start(Tag::Table(alignments)) => {
            state.table_alignments.clone_from(alignments);
            state.table_in_head = true;
            state.table_cell_index = 0;
            let (sid, attrs) = source_map.open_attr(SourceKind::Table, range, markdown);
            state.table = sid;
            ensure_block_boundary(html);
            html.push_str(&format!("<table{}>", attrs));
            true
        }
        Event::End(TagEnd::Table) => {
            source_map.close(state.table.take(), range, markdown);
            html.push_str("</tbody></table>\n");
            state.table_alignments.clear();
            state.table_in_head = false;
            state.table_cell_index = 0;
            true
        }
        Event::Start(Tag::TableHead) => {
            state.table_in_head = true;
            state.table_cell_index = 0;
            let (sid, attrs) = source_map.open_attr(SourceKind::TableHead, range, markdown);
            state.table_head = sid;
            html.push_str(&format!("<thead{}><tr{}>", attrs, attrs));
            true
        }
        Event::End(TagEnd::TableHead) => {
            source_map.close(state.table_head.take(), range, markdown);
            html.push_str("</tr></thead><tbody>\n");
            state.table_in_head = false;
            true
        }
        Event::Start(Tag::TableRow) => {
            state.table_cell_index = 0;
            let (sid, attrs) = source_map.open_attr(SourceKind::TableRow, range, markdown);
            state.table_row = sid;
            html.push_str(&format!("<tr{}>", attrs));
            true
        }
        Event::End(TagEnd::TableRow) => {
            source_map.close(state.table_row.take(), range, markdown);
            html.push_str("</tr>\n");
            true
        }
        Event::Start(Tag::TableCell) => {
            let tag = if state.table_in_head { "th" } else { "td" };
            let (sid, attrs) = source_map.open_attr(SourceKind::TableCell, range, markdown);
            state.table_cell = sid;
            html.push_str(&format!("<{}{}>", tag, table_cell_attrs(state, attrs)));
            true
        }
        Event::End(TagEnd::TableCell) => {
            let tag = if state.table_in_head { "th" } else { "td" };
            source_map.close(state.table_cell.take(), range, markdown);
            html.push_str(&format!("</{}>", tag));
            state.table_cell_index += 1;
            true
        }
        _ => false,
    }
}

fn ensure_block_boundary(html: &mut String) {
    if !html.is_empty() && !html.ends_with('\n') {
        html.push('\n');
    }
}

fn blockquote_class_attr(kind: Option<BlockQuoteKind>) -> &'static str {
    match kind {
        None => "",
        Some(BlockQuoteKind::Note) => " class=\"markdown-alert-note\"",
        Some(BlockQuoteKind::Tip) => " class=\"markdown-alert-tip\"",
        Some(BlockQuoteKind::Important) => " class=\"markdown-alert-important\"",
        Some(BlockQuoteKind::Warning) => " class=\"markdown-alert-warning\"",
        Some(BlockQuoteKind::Caution) => " class=\"markdown-alert-caution\"",
    }
}

fn table_cell_attrs(state: &HtmlRenderState, mut attrs: String) -> String {
    match state.table_alignments.get(state.table_cell_index) {
        Some(Alignment::Left) => attrs.push_str(" style=\"text-align: left\""),
        Some(Alignment::Center) => attrs.push_str(" style=\"text-align: center\""),
        Some(Alignment::Right) => attrs.push_str(" style=\"text-align: right\""),
        Some(Alignment::None) | None => {}
    }

    attrs
}

/// Regex to match req markers
fn req_marker_regex() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^[a-z0-9]+\[[^\]]+\]\s*").unwrap())
}

/// Strip req marker from text if present, returns owned String
fn strip_req_marker(text: &str) -> String {
    req_marker_regex().replace(text, "").into_owned()
}

async fn flush_req_text(
    html: &mut String,
    buffer: &mut String,
    marker_stripped: &mut bool,
    _options: &RenderOptions,
    render_wiki_links: bool,
) {
    if buffer.is_empty() {
        return;
    }

    let text = if !*marker_stripped {
        *marker_stripped = true;
        strip_req_marker(buffer)
    } else {
        std::mem::take(buffer)
    };

    if !text.is_empty() {
        if render_wiki_links {
            render_text(html, &text);
        } else {
            html.push_str(&html_escape(&text));
        }
    }
    buffer.clear();
}

/// Render the content of a paragraph req (stripping the r[...] marker)
///
/// Uses a text buffer to accumulate consecutive text events (pulldown-cmark
/// splits text across multiple events), then strips the req marker when flushing.
async fn render_paragraph_req_content(
    events: &[(Event<'_>, Range<usize>)],
    options: &RenderOptions,
) -> String {
    let mut html = String::new();
    let mut text_buffer = String::new();
    let mut marker_stripped = false;
    let mut link_stack: Vec<ActiveLink> = Vec::new();

    for (event, _range) in events {
        match event {
            Event::Text(t) => {
                text_buffer.push_str(t.as_ref());
            }
            Event::SoftBreak => {
                text_buffer.push('\n');
            }
            Event::HardBreak => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str("<br />\n");
            }
            Event::Code(code) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str(&render_inline_code(
                    code,
                    options.inline_code_handler.as_ref(),
                ));
            }
            Event::Start(Tag::Paragraph) => {
                html.push_str("<p>");
            }
            Event::End(TagEnd::Paragraph) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str("</p>\n");
            }
            Event::Start(Tag::Emphasis) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str("<em>");
            }
            Event::End(TagEnd::Emphasis) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str("</em>");
            }
            Event::Start(Tag::Strong) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str("<strong>");
            }
            Event::End(TagEnd::Strong) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str("</strong>");
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            }) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                let active_link =
                    render_link_start(&mut html, link_type, dest_url, title, options).await;
                link_stack.push(active_link);
            }
            Event::End(TagEnd::Link) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    false,
                )
                .await;
                let active_link = link_stack.pop().unwrap_or(ActiveLink::Regular);
                render_link_end(&mut html, active_link);
            }
            _ => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                pulldown_cmark::html::push_html(&mut html, std::iter::once(event.clone()));
            }
        }
    }

    // Final flush
    flush_req_text(
        &mut html,
        &mut text_buffer,
        &mut marker_stripped,
        options,
        link_stack.is_empty(),
    )
    .await;

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
    let mut blockquote_depth: usize = 0;
    let mut link_stack: Vec<ActiveLink> = Vec::new();

    // Resolve inline `*:name*` shortcodes in this content before rendering.
    let events = resolve_inline_shortcodes(events, options).await?;

    for (event, _range) in &events {
        match event {
            Event::Start(Tag::BlockQuote(_)) => {
                if blockquote_depth > 0 {
                    flush_req_text(
                        &mut html,
                        &mut text_buffer,
                        &mut marker_stripped,
                        options,
                        link_stack.is_empty(),
                    )
                    .await;
                    html.push_str("<blockquote>");
                }
                blockquote_depth += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                blockquote_depth -= 1;
                if blockquote_depth > 0 {
                    flush_req_text(
                        &mut html,
                        &mut text_buffer,
                        &mut marker_stripped,
                        options,
                        link_stack.is_empty(),
                    )
                    .await;
                    html.push_str("</blockquote>");
                }
            }
            Event::Start(Tag::Paragraph) => {
                html.push_str("<p>");
                in_paragraph = true;
            }
            Event::End(TagEnd::Paragraph) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str("</p>\n");
                in_paragraph = false;
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
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
                // Strip trailing newline from code
                let code_trimmed = code_block_content.trim_end_matches('\n');
                let output = handler.render(&code_block_lang, code_trimmed).await?;
                // Head injections from blockquote code blocks are discarded here;
                // the top-level render() call is responsible for collecting them.
                html.push_str(&output.html);
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
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str("<br />\n");
            }
            Event::Code(code) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str(&render_inline_code(
                    code,
                    options.inline_code_handler.as_ref(),
                ));
            }
            Event::Start(Tag::Emphasis) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str("<em>");
            }
            Event::End(TagEnd::Emphasis) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str("</em>");
            }
            Event::Start(Tag::Strong) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str("<strong>");
            }
            Event::End(TagEnd::Strong) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                html.push_str("</strong>");
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            }) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    link_stack.is_empty(),
                )
                .await;
                let active_link =
                    render_link_start(&mut html, link_type, dest_url, title, options).await;
                link_stack.push(active_link);
            }
            Event::End(TagEnd::Link) => {
                flush_req_text(
                    &mut html,
                    &mut text_buffer,
                    &mut marker_stripped,
                    options,
                    false,
                )
                .await;
                let active_link = link_stack.pop().unwrap_or(ActiveLink::Regular);
                render_link_end(&mut html, active_link);
            }
            _ => {
                if !in_code_block {
                    flush_req_text(
                        &mut html,
                        &mut text_buffer,
                        &mut marker_stripped,
                        options,
                        link_stack.is_empty(),
                    )
                    .await;
                    pulldown_cmark::html::push_html(&mut html, std::iter::once(event.clone()));
                }
            }
        }
    }

    // Final flush
    flush_req_text(
        &mut html,
        &mut text_buffer,
        &mut marker_stripped,
        options,
        link_stack.is_empty(),
    )
    .await;

    Ok(html)
}

/// Try to parse a paragraph as a requirement definition.
/// Returns Some(Ok(req)) if successful, Some(Err) if it looks like a req but is invalid,
/// or None if it's not a req at all.
fn try_parse_paragraph_req<'a>(
    text: &str,
    markdown: &str,
    offset: usize,
    seen_ids: &mut std::collections::HashSet<RuleId>,
    seen_bases: &mut std::collections::HashSet<String>,
    _paragraph_events: &[(Event<'a>, std::ops::Range<usize>)],
) -> Option<Result<ReqDefinition>> {
    // Must start with PREFIX[ and have a closing ]
    let (prefix, marker_content, marker_end) = parse_req_leading_marker(text)?;

    // Parse the req marker
    let (req_id, metadata) = match parse_req_marker(marker_content) {
        Ok(result) => result,
        Err(e) => return Some(Err(e)),
    };

    // Check for duplicates
    if seen_ids.contains(&req_id) {
        return Some(Err(crate::Error::DuplicateReq(req_id.to_string())));
    }
    if seen_bases.contains(&req_id.base) {
        return Some(Err(crate::Error::DuplicateReq(format!(
            "duplicate requirement base: {}",
            req_id.base
        ))));
    }
    seen_ids.insert(req_id.clone());
    seen_bases.insert(req_id.base.clone());

    let line = offset_to_line(markdown, offset);
    let anchor_id = format!("{}-{}", prefix, req_id);

    // @tracey:ignore-next-line
    // marker_span covers just r[req.id] - use for inlay hints and diagnostics
    let marker_len = marker_end + 1; // includes r[ and ]

    // raw is everything after the marker, trimmed
    let raw = text[marker_len..].trim().to_string();

    // html is generated later by render_paragraph_req_content
    let html = String::new();

    // r[impl dashboard.editing.byte-range.req-span]
    // r[impl dashboard.editing.byte-range.marker-and-content]
    let req = ReqDefinition {
        id: req_id,
        anchor_id,
        marker_span: SourceSpan {
            offset,
            length: marker_len,
        },
        span: SourceSpan {
            offset,
            length: text.len(),
        },
        line,
        metadata,
        raw,
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
    end_offset: usize,
    seen_ids: &mut std::collections::HashSet<RuleId>,
    seen_bases: &mut std::collections::HashSet<String>,
) -> Option<Result<ReqDefinition>> {
    // Must start with PREFIX[ and have a closing ]
    let (prefix, marker_content, marker_end) = parse_req_leading_marker(first_para_text)?;

    // Parse the req marker
    let (req_id, metadata) = match parse_req_marker(marker_content) {
        Ok(result) => result,
        Err(e) => return Some(Err(e)),
    };

    // Check for duplicates
    if seen_ids.contains(&req_id) {
        return Some(Err(crate::Error::DuplicateReq(req_id.to_string())));
    }
    if seen_bases.contains(&req_id.base) {
        return Some(Err(crate::Error::DuplicateReq(format!(
            "duplicate requirement base: {}",
            req_id.base
        ))));
    }
    seen_ids.insert(req_id.clone());
    seen_bases.insert(req_id.base.clone());

    let line = offset_to_line(markdown, offset);
    let anchor_id = format!("{}-{}", prefix, req_id);

    // @tracey:ignore-next-line
    // marker_span covers just r[req.id] - use for inlay hints and diagnostics
    let marker_len = marker_end + 1; // includes r[ and ]

    // Extract raw content: full blockquote source minus the first line (which has the marker)
    let full_source = &markdown[offset..end_offset];
    let raw = if let Some(newline_pos) = full_source.find('\n') {
        // Skip the first line containing the marker, trim trailing whitespace
        full_source[newline_pos + 1..].trim_end().to_string()
    } else {
        // Single line blockquote with just the marker - no content
        String::new()
    };

    // html is generated later by render_blockquote_req_content
    let html = String::new();

    // r[impl dashboard.editing.byte-range.req-span]
    // r[impl dashboard.editing.byte-range.marker-and-content]
    let req = ReqDefinition {
        id: req_id,
        anchor_id,
        marker_span: SourceSpan {
            offset,
            length: marker_len,
        },
        span: SourceSpan {
            offset,
            length: end_offset.saturating_sub(offset),
        },
        line,
        metadata,
        raw,
        html,
    };

    Some(Ok(req))
}

fn parse_req_leading_marker(text: &str) -> Option<(&str, &str, usize)> {
    let mut prefix_len = 0usize;
    for ch in text.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            prefix_len += ch.len_utf8();
        } else {
            break;
        }
    }
    if prefix_len == 0 || text.as_bytes().get(prefix_len) != Some(&b'[') {
        return None;
    }

    let marker_end = text.find(']')?;
    if marker_end <= prefix_len + 1 {
        return None;
    }

    let prefix = &text[..prefix_len];
    let marker_content = &text[prefix_len + 1..marker_end];
    Some((prefix, marker_content, marker_end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ShortcodeOutput;

    struct TestWikiResolver;

    impl WikiLinkResolver for TestWikiResolver {
        fn resolve<'a>(
            &'a self,
            link: &'a WikiLink,
            _source_path: Option<&'a str>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<WikiLinkOutput>> + Send + 'a>>
        {
            Box::pin(async move {
                let key = link
                    .target
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() {
                            c.to_ascii_lowercase()
                        } else {
                            '-'
                        }
                    })
                    .collect::<String>()
                    .trim_matches('-')
                    .to_string();
                Some(
                    WikiLinkOutput::new(format!("wiki:{key}"))
                        .with_attr("data-wiki-target", link.target.as_str()),
                )
            })
        }
    }

    /// Echoes each shortcode as a `<sc>` element so tests can assert on the name,
    /// the args passed through, and whether a body was supplied.
    struct TestShortcodeResolver;

    impl ShortcodeResolver for TestShortcodeResolver {
        fn resolve<'a>(
            &'a self,
            sc: Shortcode<'a>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<ShortcodeOutput>>> + Send + 'a>,
        > {
            let name = sc.name.to_string();
            let args = format!("{:?}", sc.args);
            let body = sc.body.map(str::to_string);
            Box::pin(async move {
                let body_attr = match &body {
                    Some(b) => format!(" body=\"{}\"", html_escape(b)),
                    None => String::new(),
                };
                Ok(Some(ShortcodeOutput::from(format!(
                    "<sc name=\"{}\" args=\"{}\"{} />",
                    html_escape(&name),
                    html_escape(&args),
                    body_attr,
                ))))
            })
        }
    }

    fn source_text<'a>(markdown: &'a str, entry: &SourceMapEntry) -> &'a str {
        &markdown[entry.byte_start..entry.byte_end]
    }

    fn sid_attr(entry: &SourceMapEntry) -> String {
        format!(r#"data-sid="{}""#, entry.id)
    }

    #[tokio::test]
    async fn fenced_shortcode_is_resolved() {
        let md = "# Heading\n\nintro\n\n+++\n:figure:\n  src: a.jxl\n  title: Hi\n+++\n\nafter\n";
        let opts = RenderOptions::new().with_shortcode_resolver(TestShortcodeResolver);
        let doc = render(md, &opts).await.unwrap();

        // The shortcode is rendered in document order, between the paragraphs.
        assert!(
            doc.html.contains(r#"<sc name="figure""#),
            "html: {}",
            doc.html
        );
        // The whole block is passed through as YAML (single `:figure` mapping).
        assert!(
            doc.html.contains(":figure:"),
            "args should carry the yaml: {}",
            doc.html
        );
        assert!(
            doc.html.contains("src: a.jxl"),
            "args yaml body: {}",
            doc.html
        );
        // No body for fenced shortcodes.
        assert!(
            !doc.html.contains(" body="),
            "fenced has no body: {}",
            doc.html
        );
        // Surrounding prose still renders.
        assert!(doc.html.contains("intro") && doc.html.contains("after"));
        // It was NOT mistaken for document frontmatter.
        assert!(
            doc.raw_metadata.is_none(),
            "shortcode must not become frontmatter"
        );
    }

    #[tokio::test]
    async fn fenced_shortcode_without_resolver_keeps_source() {
        // With no resolver registered, marq bakes nothing: the block is left verbatim
        // rather than silently dropped or mis-parsed as frontmatter.
        let md = "intro\n\n+++\n:figure:\n  src: a.jxl\n+++\n\nafter\n";
        let doc = render(md, &RenderOptions::default()).await.unwrap();
        assert!(doc.html.contains(":figure:"), "source kept: {}", doc.html);
        assert!(doc.raw_metadata.is_none());
    }

    #[tokio::test]
    async fn body_shortcode_passes_rendered_body() {
        // home's `shortblocks` oracle: `> *:bearsays*` + body.
        let md = "> *:bearsays*\n>\n> I am not sure about that\n";
        let opts = RenderOptions::new().with_shortcode_resolver(TestShortcodeResolver);
        let doc = render(md, &opts).await.unwrap();

        assert!(
            doc.html.contains(r#"name="bearsays""#),
            "html: {}",
            doc.html
        );
        // The body markdown is rendered to HTML and handed to the resolver.
        assert!(
            doc.html.contains(" body="),
            "should carry a body: {}",
            doc.html
        );
        assert!(
            doc.html.contains("I am not sure about that"),
            "body text: {}",
            doc.html
        );
        // The marker `*:bearsays*` itself is consumed, not rendered as <em>.
        assert!(
            !doc.html.contains("<em>:bearsays"),
            "marker leaked: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn body_shortcode_parses_parenthesised_args() {
        let md = "> *:tip(title=Hot, kind=\"cool bear\")*\n>\n> watch out\n";
        let opts = RenderOptions::new().with_shortcode_resolver(TestShortcodeResolver);
        let doc = render(md, &opts).await.unwrap();

        assert!(doc.html.contains(r#"name="tip""#), "html: {}", doc.html);
        // Pairs preserved in source order; quoted value keeps its space.
        assert!(
            doc.html.contains("title") && doc.html.contains("Hot"),
            "args: {}",
            doc.html
        );
        assert!(
            doc.html.contains("cool bear"),
            "quoted arg with space: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn inline_shortcode_in_paragraph_is_resolved() {
        // The rarer inline form: `*:name*` mid-paragraph, no body.
        let md = "before the marker *:bearsays* after the marker\n";
        let opts = RenderOptions::new().with_shortcode_resolver(TestShortcodeResolver);
        let doc = render(md, &opts).await.unwrap();

        assert!(
            doc.html.contains(r#"name="bearsays""#),
            "html: {}",
            doc.html
        );
        // No body for the inline form.
        assert!(
            !doc.html.contains(" body="),
            "inline has no body: {}",
            doc.html
        );
        // The marker is replaced, not rendered as <em>; surrounding prose stays.
        assert!(
            !doc.html.contains("<em>:bearsays"),
            "marker leaked: {}",
            doc.html
        );
        assert!(
            doc.html.contains("before the marker") && doc.html.contains("after the marker"),
            "prose preserved: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn ordinary_emphasis_is_not_a_shortcode() {
        // A normal `*word*` must still be <em>, even with a resolver registered.
        let md = "some *italic* text\n";
        let opts = RenderOptions::new().with_shortcode_resolver(TestShortcodeResolver);
        let doc = render(md, &opts).await.unwrap();
        assert!(doc.html.contains("<em>italic</em>"), "html: {}", doc.html);
        assert!(!doc.html.contains("<sc "), "no shortcode: {}", doc.html);
    }

    #[tokio::test]
    async fn plain_blockquote_is_untouched() {
        // A blockquote that isn't a shortcode must still render as a blockquote,
        // even with a resolver registered.
        let md = "> just a quote\n";
        let opts = RenderOptions::new().with_shortcode_resolver(TestShortcodeResolver);
        let doc = render(md, &opts).await.unwrap();
        assert!(doc.html.contains("<blockquote>"), "html: {}", doc.html);
        assert!(!doc.html.contains("<sc "), "no shortcode: {}", doc.html);
    }

    #[tokio::test]
    async fn leading_toml_frontmatter_is_not_a_shortcode() {
        // A leading `+++` block without a `:name` key is ordinary TOML frontmatter and
        // must keep working unchanged, even with a resolver registered.
        let md = "+++\ntitle = \"Hello\"\n+++\n\nbody\n";
        let opts = RenderOptions::new().with_shortcode_resolver(TestShortcodeResolver);
        let doc = render(md, &opts).await.unwrap();
        assert_eq!(doc.metadata_format, Some(FrontmatterFormat::Toml));
        assert!(
            doc.raw_metadata
                .as_deref()
                .unwrap()
                .contains("title = \"Hello\"")
        );
        assert!(
            !doc.html.contains("<sc "),
            "frontmatter must not resolve as shortcode"
        );
    }

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
    async fn test_wiki_links_are_plain_text_without_resolver() {
        let md = "See [[Company]] and [[Repository Map|repo map]].";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert!(
            doc.html.contains("[[Company]]"),
            "wiki syntax should stay literal without resolver: {}",
            doc.html
        );
        assert!(
            !doc.html.contains("wiki:company"),
            "wiki links should be opt-in: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_wiki_links_resolve_with_label_and_attrs() {
        let md = "See [[Company]] and [[Repository Map|repo map]].";
        let opts = RenderOptions::new().with_wiki_link_resolver(TestWikiResolver);
        let doc = render(md, &opts).await.unwrap();

        assert!(
            doc.html
                .contains(r#"<a href="wiki:company" data-wiki-target="Company">Company</a>"#),
            "bare wiki link should resolve: {}",
            doc.html
        );
        assert!(
            doc.html.contains(
                r#"<a href="wiki:repository-map" data-wiki-target="Repository Map">repo map</a>"#
            ),
            "labeled wiki link should resolve: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_wiki_links_skip_code_spans_and_code_blocks() {
        let md = "Inline `[[Company]]`.\n\n```md\n[[Company]]\n```\n";
        let opts = RenderOptions::new().with_wiki_link_resolver(TestWikiResolver);
        let doc = render(md, &opts).await.unwrap();

        assert!(
            !doc.html.contains("wiki:company"),
            "wiki syntax in code should not resolve: {}",
            doc.html
        );
        assert!(
            doc.html.contains("<code>[[Company]]</code>") || doc.html.contains("[[Company]]"),
            "literal wiki syntax should remain visible in code: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_wiki_links_do_not_disturb_regular_links() {
        let md = "[[Company]] and [literal link](https://example.com).";
        let opts = RenderOptions::new().with_wiki_link_resolver(TestWikiResolver);
        let doc = render(md, &opts).await.unwrap();

        assert_eq!(
            doc.html.matches("wiki:company").count(),
            1,
            "only the standalone wiki link should resolve: {}",
            doc.html
        );
        assert!(
            doc.html
                .contains(r#"<a href="https://example.com">literal link</a>"#),
            "regular links should still render normally: {}",
            doc.html
        );
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

    /// Test that multiline code blocks preserve newlines in the output.
    /// This is critical for code to display correctly.
    #[tokio::test]
    #[cfg(feature = "highlight")]
    async fn test_render_code_block_preserves_newlines() {
        use crate::handlers::ArboriumHandler;

        let md = r#"```rust
fn greet(name: &str) {
    println!("Hello, {}!", name);
}

fn main() {
    greet("World");
}
```
"#;
        let opts = RenderOptions::new().with_default_handler(ArboriumHandler::new());
        let doc = render(md, &opts).await.unwrap();

        // Count newlines in the code block portion
        let code_start = doc.html.find("<code").expect("should have <code>");
        let code_end = doc.html.find("</code>").expect("should have </code>");
        let code_section = &doc.html[code_start..code_end];

        let newlines = code_section.matches('\n').count();

        // The source has 7 lines, so at least 6 newlines should be present
        assert!(
            newlines >= 5,
            "Code block should preserve newlines. Found {} newlines in:\n{}",
            newlines,
            code_section
        );
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

        // Verify marker_span points to the r[ not the > prefix
        let req = &doc.reqs[0];
        let marker_text =
            &md[req.marker_span.offset..req.marker_span.offset + req.marker_span.length];
        assert_eq!(
            marker_text, "r[my.req]",
            "marker_span should point to r[my.req], got: {}",
            marker_text
        );
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
            doc.html.contains("<blockquote"),
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
    async fn test_paragraph_html_has_source_id_attributes() {
        let md = r#"First paragraph.

Second paragraph.

Third paragraph.
"#;
        let opts = RenderOptions::default().with_source_map(true);
        let doc = render(md, &opts).await.unwrap();

        let entries = &doc.source_map.entries;
        assert_eq!(entries.len(), 3);

        for entry in entries {
            let expected = format!(r#"<p {}>"#, sid_attr(entry));
            assert!(
                doc.html.contains(&expected),
                "expected {expected:?} in HTML:\n{}",
                doc.html
            );
        }

        assert_eq!(entries[0].kind, SourceKind::Paragraph);
        assert_eq!(entries[0].line_start, 1);
        assert_eq!(entries[0].line_end, 1);
        assert_eq!(
            doc.source_map
                .get(&entries[0].id)
                .map(|entry| entry.byte_start),
            Some(entries[0].byte_start)
        );
        assert_eq!(
            doc.source_map
                .get_by_sid(entries[0].id.as_str())
                .map(|entry| entry.byte_start),
            Some(entries[0].byte_start)
        );
        assert_eq!(source_text(md, &entries[0]), "First paragraph.\n");
        assert_eq!(entries[1].line_start, 3);
        assert_eq!(entries[1].line_end, 3);
        assert_eq!(source_text(md, &entries[1]), "Second paragraph.\n");
        assert_eq!(entries[2].line_start, 5);
        assert_eq!(entries[2].line_end, 5);
        assert_eq!(source_text(md, &entries[2]), "Third paragraph.\n");
    }

    #[tokio::test]
    async fn test_source_map_has_source_path() {
        let md = "A paragraph with source file info.";
        let opts = RenderOptions {
            source_path: Some("docs/test.md".to_string()),
            source_map: true,
            ..Default::default()
        };
        let doc = render(md, &opts).await.unwrap();

        assert!(
            doc.html.contains("data-sid"),
            "Should have source ID attribute: {}",
            doc.html
        );
        assert_eq!(doc.source_map.source_path.as_deref(), Some("docs/test.md"));
        assert_eq!(doc.source_map.entries.len(), 1);
        assert_eq!(doc.source_map.entries[0].line_start, 1);
        assert_eq!(source_text(md, &doc.source_map.entries[0]), md);
    }

    #[tokio::test]
    async fn test_source_map_is_opt_in() {
        let md = "# Title\n\nA paragraph with [a link](other.md).";
        let opts = RenderOptions {
            source_path: Some("docs/test.md".to_string()),
            ..Default::default()
        };
        let doc = render(md, &opts).await.unwrap();

        assert!(
            doc.html.contains(r#"href="/docs/other/""#),
            "source_path should still resolve links: {}",
            doc.html
        );
        assert!(
            !doc.html.contains("data-sid"),
            "source map should be disabled by default: {}",
            doc.html
        );
        assert!(doc.source_map.entries.is_empty());
        assert_eq!(doc.source_map.source_path, None);
    }

    #[tokio::test]
    async fn test_block_elements_have_source_ids_and_source_map_entries() {
        let md = r#"# Title

- first
- second

1. one
2. two

> quoted

![Alt](pic.png)

---

| A | B |
| - | -: |
| x | y |
"#;
        let opts = RenderOptions {
            source_path: Some("docs/test.md".to_string()),
            source_map: true,
            ..Default::default()
        };
        let doc = render(md, &opts).await.unwrap();

        let entries = &doc.source_map.entries;
        for expected in [
            format!(r#"<h1 id="title" {}>Title</h1>"#, sid_attr(&entries[0])),
            format!(r#"<ul {}>"#, sid_attr(&entries[1])),
            format!(r#"<li {}>"#, sid_attr(&entries[2])),
            format!(r#"<li {}>"#, sid_attr(&entries[3])),
            format!(r#"<ol {}>"#, sid_attr(&entries[4])),
            format!(r#"<li {}>"#, sid_attr(&entries[5])),
            format!(r#"<li {}>"#, sid_attr(&entries[6])),
            format!(r#"<blockquote {}>"#, sid_attr(&entries[7])),
            format!(r#"<p {}>"#, sid_attr(&entries[8])),
            format!(
                r#"<p {}><img {}"#,
                sid_attr(&entries[9]),
                sid_attr(&entries[10])
            ),
            format!(r#"<hr {} />"#, sid_attr(&entries[11])),
            format!(r#"<table {}>"#, sid_attr(&entries[12])),
            format!(
                r#"<thead {}><tr {}>"#,
                sid_attr(&entries[13]),
                sid_attr(&entries[13])
            ),
            format!(r#"<th {}>"#, sid_attr(&entries[14])),
            format!(
                r#"<th {} style="text-align: right">"#,
                sid_attr(&entries[15])
            ),
            format!(r#"<tr {}>"#, sid_attr(&entries[16])),
            format!(r#"<td {}>"#, sid_attr(&entries[17])),
            format!(
                r#"<td {} style="text-align: right">"#,
                sid_attr(&entries[18])
            ),
        ] {
            assert!(
                doc.html.contains(&expected),
                "expected {expected:?} in HTML:\n{}",
                doc.html
            );
        }

        let kinds: Vec<SourceKind> = entries.iter().map(|entry| entry.kind).collect();
        assert_eq!(
            kinds,
            vec![
                SourceKind::Heading,
                SourceKind::List,
                SourceKind::ListItem,
                SourceKind::ListItem,
                SourceKind::List,
                SourceKind::ListItem,
                SourceKind::ListItem,
                SourceKind::BlockQuote,
                SourceKind::Paragraph,
                SourceKind::Paragraph,
                SourceKind::Image,
                SourceKind::ThematicBreak,
                SourceKind::Table,
                SourceKind::TableHead,
                SourceKind::TableCell,
                SourceKind::TableCell,
                SourceKind::TableRow,
                SourceKind::TableCell,
                SourceKind::TableCell,
            ]
        );
        assert_eq!(doc.source_map.source_path.as_deref(), Some("docs/test.md"));
        assert_eq!(source_text(md, &entries[0]), "# Title\n");
        assert_eq!(source_text(md, &entries[10]), "![Alt](pic.png)");
        assert_eq!(entries[1].line_start, 3);
        assert_eq!(entries[4].line_start, 6);
        assert_eq!(entries[7].line_start, 9);
        assert_eq!(entries[11].line_start, 13);
        assert_eq!(entries[12].line_start, 15);
    }

    #[tokio::test]
    async fn test_source_ids_are_stable_after_unrelated_insertion() {
        let before = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.\n";
        let after =
            "Inserted paragraph.\n\nFirst paragraph.\n\nSecond paragraph.\n\nThird paragraph.\n";
        let opts = RenderOptions::default().with_source_map(true);

        let before_doc = render(before, &opts).await.unwrap();
        let after_doc = render(after, &opts).await.unwrap();

        for source in [
            "First paragraph.\n",
            "Second paragraph.\n",
            "Third paragraph.\n",
        ] {
            let before_id = before_doc
                .source_map
                .entries
                .iter()
                .find(|entry| source_text(before, entry) == source)
                .map(|entry| entry.id.clone())
                .unwrap();
            let after_id = after_doc
                .source_map
                .entries
                .iter()
                .find(|entry| source_text(after, entry) == source)
                .map(|entry| entry.id.clone())
                .unwrap();

            assert_eq!(before_id, after_id, "source ID changed for {source:?}");
        }

        assert!(!before_doc.html.contains("__marq-source"));
        assert!(!after_doc.html.contains("__marq-source"));
    }

    #[tokio::test]
    async fn test_block_source_ids_use_full_source_span() {
        let before = "- alpha\n- beta\n\nTail paragraph.\n";
        let after = "Intro paragraph.\n\n- alpha\n- beta\n\nTail paragraph.\n";
        let opts = RenderOptions::default().with_source_map(true);

        let before_doc = render(before, &opts).await.unwrap();
        let after_doc = render(after, &opts).await.unwrap();

        for (kind, source) in [
            (SourceKind::List, "- alpha\n- beta\n\n"),
            (SourceKind::ListItem, "- alpha\n"),
            (SourceKind::ListItem, "- beta\n\n"),
            (SourceKind::Paragraph, "Tail paragraph.\n"),
        ] {
            let before_entries: Vec<_> = before_doc
                .source_map
                .entries
                .iter()
                .map(|entry| (entry.kind, source_text(before, entry).to_string()))
                .collect();
            let after_entries: Vec<_> = after_doc
                .source_map
                .entries
                .iter()
                .map(|entry| (entry.kind, source_text(after, entry).to_string()))
                .collect();
            let before_id = before_doc
                .source_map
                .entries
                .iter()
                .find(|entry| entry.kind == kind && source_text(before, entry) == source)
                .map(|entry| entry.id.clone())
                .unwrap_or_else(|| {
                    panic!("missing before {kind:?} {source:?}; entries: {before_entries:?}")
                });
            let after_id = after_doc
                .source_map
                .entries
                .iter()
                .find(|entry| entry.kind == kind && source_text(after, entry) == source)
                .map(|entry| entry.id.clone())
                .unwrap_or_else(|| {
                    panic!("missing after {kind:?} {source:?}; entries: {after_entries:?}")
                });

            assert_eq!(
                before_id, after_id,
                "source ID changed for {kind:?} {source:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_duplicate_source_ids_get_document_scoped_suffixes() {
        let md = "Same paragraph.\n\nSame paragraph.\n";
        let opts = RenderOptions::default().with_source_map(true);
        let doc = render(md, &opts).await.unwrap();

        let entries = &doc.source_map.entries;
        assert_eq!(entries.len(), 2);
        assert_ne!(entries[0].id, entries[1].id);
        assert!(
            entries[1].id.as_str().ends_with("-2"),
            "expected duplicate suffix in {:?}",
            entries[1].id
        );
    }

    // =========================================================================
    // Requirement marker stripping tests - comprehensive edge cases
    // =========================================================================

    #[test]
    fn test_strip_req_marker_basic() {
        assert_eq!(strip_req_marker("r[foo] bar"), "bar");
        assert_eq!(strip_req_marker("req[foo] bar"), "bar");
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
    async fn test_req_marker_non_r_prefix_same_line() {
        let md = "req[same.line] This text is on the same line.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "same.line");
        assert_eq!(doc.reqs[0].anchor_id, "req-same.line");
        assert!(
            !doc.html.contains("req[same.line]"),
            "Raw marker should be stripped: {}",
            doc.html
        );
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

    // =========================================================================
    // Internal link resolution tests (@/ prefix)
    // =========================================================================

    #[tokio::test]
    async fn test_absolute_at_link_resolved() {
        // @/path/to/file.md links should be resolved to /path/to/file/
        let md = r#"Check out [structstruck](@/guide/structstruck.md) for more info."#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert!(
            !doc.html.contains("@/"),
            "@/ prefix should be resolved, not left in HTML: {}",
            doc.html
        );
        assert!(
            doc.html.contains(r#"href="/guide/structstruck/""#),
            "Link should resolve to /guide/structstruck/: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_absolute_at_link_with_backticks_in_text() {
        // Backticks in link text shouldn't affect URL resolution
        let md = r#"- [`structstruck`](@/guide/structstruck.md) — generate structs"#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert!(
            !doc.html.contains("@/"),
            "@/ prefix should be resolved even with backticks in link text: {}",
            doc.html
        );
        assert!(
            doc.html.contains(r#"href="/guide/structstruck/""#),
            "Link should resolve to /guide/structstruck/: {}",
            doc.html
        );
        assert!(
            doc.html.contains("<code>structstruck</code>"),
            "Backticks should render as code: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_absolute_at_link_with_fragment() {
        let md = r#"See [the section](@/guide/intro.md#getting-started) for details."#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert!(
            !doc.html.contains("@/"),
            "@/ prefix should be resolved: {}",
            doc.html
        );
        assert!(
            doc.html.contains(r#"href="/guide/intro/#getting-started""#),
            "Link should resolve with fragment: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_absolute_at_link_to_index() {
        let md = r#"Go to [the guide](@/guide/_index.md) section."#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert!(
            !doc.html.contains("@/"),
            "@/ prefix should be resolved: {}",
            doc.html
        );
        assert!(
            doc.html.contains(r#"href="/guide/""#),
            "_index.md should resolve to parent directory: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_relative_md_link_resolved() {
        // Relative .md links should be resolved based on source_path
        let md = r#"See [sibling](sibling.md) for more."#;
        let opts = RenderOptions {
            source_path: Some("guide/current.md".to_string()),
            ..Default::default()
        };
        let doc = render(md, &opts).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert!(
            doc.html.contains(r#"href="/guide/sibling/""#),
            "Relative .md link should resolve to /guide/sibling/: {}",
            doc.html
        );
        assert!(
            !doc.html.contains(r#"href="sibling.md""#),
            "Original .md link should not appear in href: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_external_link_unchanged() {
        // External links should pass through unchanged
        let md = r#"Visit [example](https://example.com/page.md) for docs."#;
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert!(
            doc.html.contains(r#"href="https://example.com/page.md""#),
            "External link should be unchanged: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_head_injections_collected() {
        use crate::handlers::MermaidHandler;

        let md = "```mermaid\ngraph TD\n    A-->B\n```\n";
        let opts = RenderOptions::new().with_handler(&["mermaid"], MermaidHandler::new());
        let doc = render(md, &opts).await.unwrap();

        assert_eq!(
            doc.head_injections.len(),
            1,
            "Should have exactly one head injection"
        );
        assert!(
            doc.head_injections[0].contains("mermaid"),
            "Head injection should contain mermaid script"
        );
    }

    #[tokio::test]
    async fn test_head_injections_deduplicated() {
        use crate::handlers::MermaidHandler;

        let md = "```mermaid\ngraph TD\n    A-->B\n```\n\n```mermaid\ngraph LR\n    X-->Y\n```\n";
        let opts = RenderOptions::new().with_handler(&["mermaid"], MermaidHandler::new());
        let doc = render(md, &opts).await.unwrap();

        assert_eq!(
            doc.head_injections.len(),
            1,
            "Two mermaid blocks should produce only one head injection, got: {}",
            doc.head_injections.len()
        );
    }

    #[tokio::test]
    async fn test_req_in_blockquote_with_nested_blockquote() {
        let md = "> r[my.rule]\n> > quoted text";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "my.rule");

        // The outer blockquote is replaced by the req div — no outer <blockquote> wrapper
        assert!(
            !doc.html.starts_with("<blockquote>"),
            "Outer blockquote should be replaced by req div: {}",
            doc.html
        );

        // Inner blockquote should be rendered as HTML <blockquote>
        assert!(
            doc.html.contains("<blockquote>"),
            "Nested blockquote should be rendered: {}",
            doc.html
        );
        assert!(
            doc.html.contains("quoted text"),
            "Nested blockquote text should be present: {}",
            doc.html
        );

        // raw should include the nested blockquote source
        assert!(
            doc.reqs[0].raw.contains("> quoted text"),
            "req.raw should include nested blockquote source: {}",
            doc.reqs[0].raw
        );
    }

    #[tokio::test]
    async fn test_req_in_blockquote_with_deeply_nested_blockquote() {
        let md = "> r[my.rule]\n> > > deeply quoted";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        eprintln!("HTML: {}", doc.html);

        assert_eq!(doc.reqs.len(), 1);
        assert_eq!(doc.reqs[0].id, "my.rule");

        // No outer blockquote wrapper
        assert!(
            !doc.html.starts_with("<blockquote>"),
            "Outer blockquote should be replaced by req div: {}",
            doc.html
        );

        // Two levels of nested <blockquote>
        let blockquote_count = doc.html.matches("<blockquote>").count();
        assert_eq!(
            blockquote_count, 2,
            "Should have two levels of nested blockquotes, got {}: {}",
            blockquote_count, doc.html
        );

        assert!(
            doc.html.contains("deeply quoted"),
            "Deeply nested text should be present: {}",
            doc.html
        );

        // raw should include the nested blockquote source
        assert!(
            doc.reqs[0].raw.contains("> > deeply quoted"),
            "req.raw should include deeply nested blockquote source: {}",
            doc.reqs[0].raw
        );
    }

    #[tokio::test]
    async fn test_mermaid_code_block_renders_client_side() {
        use crate::handlers::MermaidHandler;

        let md = "```mermaid\ngraph TD\n    A-->B\n```\n";
        let opts = RenderOptions::new().with_handler(&["mermaid"], MermaidHandler::new());
        let doc = render(md, &opts).await.unwrap();

        assert!(
            doc.html.contains("data-hotmeal-opaque=\"mermaid\""),
            "Should have hotmeal opaque wrapper: {}",
            doc.html
        );
        assert!(
            doc.html.contains("<pre class=\"mermaid\">"),
            "Should have pre.mermaid: {}",
            doc.html
        );
        assert!(
            doc.html.contains("A--&gt;B"),
            "Mermaid code should be HTML-escaped: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn inline_math_renders_mathml() {
        let doc = render(
            "The loss $L(x) = x^2$ is convex.",
            &RenderOptions::default(),
        )
        .await
        .unwrap();
        assert!(
            doc.html.contains("<math"),
            "inline $…$ should render a <math> element: {}",
            doc.html
        );
        assert!(
            !doc.html.contains("$L(x)"),
            "raw TeX delimiters should not survive: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn display_math_renders_block_mathml() {
        let doc = render(
            r"$$\int_0^1 x \, dx = \frac{1}{2}$$",
            &RenderOptions::default(),
        )
        .await
        .unwrap();
        assert!(
            doc.html.contains("display=\"block\""),
            "$$…$$ should render display=\"block\" MathML: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn lone_dollar_in_prose_is_not_math() {
        // A single unmatched `$` must stay literal — no math span.
        let doc = render("It costs $5 today.", &RenderOptions::default())
            .await
            .unwrap();
        assert!(
            !doc.html.contains("<math"),
            "lone $ should not start a math span: {}",
            doc.html
        );
        assert!(
            doc.html.contains("$5"),
            "the literal $5 should survive: {}",
            doc.html
        );
    }

    #[tokio::test]
    async fn test_inline_code_spans_collected() {
        let md = "See `r[auth.login]` and `r[data.format]` for details.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.inline_code_spans.len(), 2);
        assert_eq!(doc.inline_code_spans[0].content, "r[auth.login]");
        assert_eq!(doc.inline_code_spans[1].content, "r[data.format]");
        // Span should cover the backtick delimiters
        assert!(doc.inline_code_spans[0].span.length > "r[auth.login]".len());
    }

    #[tokio::test]
    async fn test_inline_code_spans_skip_fenced_code_blocks() {
        let md = "```rust\n// r[auth.login]\n```\n\nSee `r[real.ref]` here.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.inline_code_spans.len(), 1);
        assert_eq!(doc.inline_code_spans[0].content, "r[real.ref]");
    }

    #[tokio::test]
    async fn test_inline_code_spans_skip_blockquoted_fenced_code_blocks() {
        let md = "> ```rust\n> // r[auth.login]\n> ```\n\nSee `r[real.ref]` here.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.inline_code_spans.len(), 1);
        assert_eq!(doc.inline_code_spans[0].content, "r[real.ref]");
    }

    #[tokio::test]
    async fn test_inline_code_spans_inside_blockquote_prose() {
        let md = "> See `r[auth.login]` for details.";
        let doc = render(md, &RenderOptions::default()).await.unwrap();

        assert_eq!(doc.inline_code_spans.len(), 1);
        assert_eq!(doc.inline_code_spans[0].content, "r[auth.login]");
    }
}
