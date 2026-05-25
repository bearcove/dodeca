//! Dodeca markdown processing cell (cell-markdown)
//!
//! This cell uses marq for markdown rendering with direct code block rendering.
//! Mermaid diagrams are rendered via callback to the host, which delegates to the mermaid cell.

use cell_markdown_proto::*;
use dodeca_cell_runtime::HostHandle;
use marq::{
    AasvgHandler, ArboriumHandler, CompareHandler, InlineCodeHandler, LinkResolver, MermaidHandler,
    PikruHandler, RenderOptions, TermHandler, WikiLink, WikiLinkOutput, WikiLinkResolver, render,
};
use std::future::Future;
use std::pin::Pin;

/// Escape HTML special characters
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Inline code handler that converts rules to links.
/// Links to #r-rule.name anchors on the same page.
struct RuleRefHandler;

impl InlineCodeHandler for RuleRefHandler {
    fn render(&self, code: &str) -> Option<String> {
        let code = code.trim();

        // Match rule marker pattern
        if !code.starts_with("r[") || !code.ends_with(']') {
            return None;
        }

        // Extract rule.id from marker
        let rule_id = &code[2..code.len() - 1];

        // Validate it looks like a rule ID (alphanumeric, dots, dashes, underscores)
        if rule_id.is_empty()
            || !rule_id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == '_')
        {
            return None;
        }

        // Generate link to #r-rule.id anchor on same page
        let anchor = format!("r-{}", rule_id);
        Some(format!(
            "<code><a href=\"#{}\" class=\"rule-ref\">{}</a></code>",
            anchor,
            html_escape(code)
        ))
    }
}

/// Link resolver that passes through @/ links unchanged for dodeca to post-process.
/// This allows dodeca to resolve links using the site tree (for custom slugs)
/// and track dependencies via picante.
struct PassthroughLinkResolver;

impl LinkResolver for PassthroughLinkResolver {
    fn resolve<'a>(
        &'a self,
        link: &'a str,
        _source_path: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Option<String>> + Send + 'a>> {
        Box::pin(async move {
            // Keep @/ links unchanged - dodeca will resolve them with site tree access
            if link.starts_with("@/") || link.starts_with(WIKI_LINK_PREFIX) {
                Some(link.to_string())
            } else {
                // Let marq handle other links (relative .md, external, etc.)
                None
            }
        })
    }
}

struct DodecaWikiLinkResolver;

impl WikiLinkResolver for DodecaWikiLinkResolver {
    fn resolve<'a>(
        &'a self,
        link: &'a WikiLink,
        _source_path: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Option<WikiLinkOutput>> + Send + 'a>> {
        Box::pin(async move {
            let key = wiki_link_key(&link.target)?;
            Some(
                WikiLinkOutput::new(format!("{WIKI_LINK_PREFIX}{key}"))
                    .with_attr("data-wiki-target", link.target.as_str()),
            )
        })
    }
}

#[derive(Clone)]
pub struct MarkdownProcessorImpl;

impl MarkdownProcessorImpl {
    fn new(_host: HostHandle) -> Self {
        // The markdown cell does not call back into the host.
        Self
    }
}

fn render_options(source_path: &str, source_map: bool) -> RenderOptions {
    RenderOptions::new()
        .with_handler(&["aa", "aasvg"], AasvgHandler::new())
        .with_handler(&["compare"], CompareHandler::new())
        .with_handler(&["pikchr"], PikruHandler::with_css_variables(true))
        .with_handler(&["term"], TermHandler::new())
        .with_handler(&["mermaid"], MermaidHandler::new())
        .with_default_handler(ArboriumHandler::new())
        .with_source_path(source_path)
        .with_source_map(source_map)
        // Pass through @/ links unchanged - dodeca will resolve them with site tree
        .with_link_resolver(PassthroughLinkResolver)
        .with_wiki_link_resolver(DodecaWikiLinkResolver)
        // Convert rule marker inline code to links
        .with_inline_code_handler(RuleRefHandler)
}

impl MarkdownProcessor for MarkdownProcessorImpl {
    async fn parse_frontmatter(&self, content: String) -> FrontmatterResult {
        match marq::parse_frontmatter(&content) {
            Ok((fm, body)) => FrontmatterResult::Success {
                frontmatter: convert_frontmatter(fm),
                body: body.to_string(),
            },
            Err(e) => FrontmatterResult::Error {
                message: e.to_string(),
            },
        }
    }

    async fn render_markdown(
        &self,
        source_path: String,
        markdown: String,
        source_map: bool,
    ) -> MarkdownResult {
        let opts = render_options(&source_path, source_map);

        // Render markdown with all code blocks rendered inline
        match render(&markdown, &opts).await {
            Ok(doc) => MarkdownResult::Success {
                html: doc.html, // Fully rendered, no placeholders
                headings: doc.headings.into_iter().map(convert_heading).collect(),
                reqs: doc.reqs.into_iter().map(convert_req).collect(),
                head_injections: doc.head_injections,
                source_map: Box::new(convert_source_map(doc.source_map)),
            },
            Err(e) => MarkdownResult::Error {
                message: e.to_string(),
            },
        }
    }

    async fn highlight_code(&self, lang: String, code: String) -> HighlightResult {
        use marq::CodeBlockHandler;

        let handler = ArboriumHandler::new();
        match handler.render(&lang, &code).await {
            Ok(output) => HighlightResult::Success { html: output.html },
            Err(_e) => {
                // Fallback: return escaped code in a plain code-block div
                let escaped = html_escape(&code);
                let escaped_lang = html_escape(&lang);
                HighlightResult::Success {
                    html: format!(
                        "<div class=\"code-block\" data-lang=\"{escaped_lang}\"><pre><code>{escaped}</code></pre></div>"
                    ),
                }
            }
        }
    }

    async fn parse_and_render(
        &self,
        source_path: String,
        content: String,
        source_map: bool,
    ) -> ParseResult {
        // Parse frontmatter
        let (fm, _) = match marq::parse_frontmatter(&content) {
            Ok(result) => result,
            Err(e) => {
                return ParseResult::Error {
                    message: e.to_string(),
                };
            }
        };

        // Render the full document so source-map line and byte ranges refer to
        // the actual source file, including any frontmatter offset.
        match self.render_markdown(source_path, content, source_map).await {
            MarkdownResult::Success {
                html,
                headings,
                reqs,
                head_injections,
                source_map,
            } => ParseResult::Success {
                frontmatter: convert_frontmatter(fm),
                html,
                headings,
                reqs,
                head_injections,
                source_map,
            },
            MarkdownResult::Error { message } => ParseResult::Error { message },
        }
    }
}

const WIKI_LINK_PREFIX: &str = "dodeca-wiki:";

fn wiki_link_key(target: &str) -> Option<String> {
    let mut key = String::new();
    let mut last_was_dash = true;

    for c in target.chars() {
        if c.is_alphanumeric() {
            for lower in c.to_lowercase() {
                key.push(lower);
            }
            last_was_dash = false;
        } else if !last_was_dash {
            key.push('-');
            last_was_dash = true;
        }
    }

    while key.ends_with('-') {
        key.pop();
    }

    if key.is_empty() { None } else { Some(key) }
}

// Helper functions to convert marq types to protocol types
fn convert_frontmatter(fm: marq::Frontmatter) -> Frontmatter {
    Frontmatter {
        title: fm.title,
        weight: fm.weight,
        description: fm.description,
        template: fm.template,
        extra: fm.extra, // Direct pass-through, no JSON conversion!
    }
}

fn convert_heading(h: marq::Heading) -> Heading {
    Heading {
        title: h.title,
        id: h.id,
        level: h.level,
    }
}

fn convert_req(r: marq::ReqDefinition) -> ReqDefinition {
    ReqDefinition {
        id: r.id.to_string(),
        anchor_id: r.anchor_id,
    }
}

fn convert_source_kind(kind: marq::SourceKind) -> SourceKind {
    match kind {
        marq::SourceKind::Heading => SourceKind::Heading,
        marq::SourceKind::Paragraph => SourceKind::Paragraph,
        marq::SourceKind::BlockQuote => SourceKind::BlockQuote,
        marq::SourceKind::List => SourceKind::List,
        marq::SourceKind::ListItem => SourceKind::ListItem,
        marq::SourceKind::DefinitionList => SourceKind::DefinitionList,
        marq::SourceKind::DefinitionListTitle => SourceKind::DefinitionListTitle,
        marq::SourceKind::DefinitionListDefinition => SourceKind::DefinitionListDefinition,
        marq::SourceKind::ThematicBreak => SourceKind::ThematicBreak,
        marq::SourceKind::Table => SourceKind::Table,
        marq::SourceKind::TableHead => SourceKind::TableHead,
        marq::SourceKind::TableRow => SourceKind::TableRow,
        marq::SourceKind::TableCell => SourceKind::TableCell,
        marq::SourceKind::Image => SourceKind::Image,
    }
}

fn convert_source_map(source_map: marq::SourceMap) -> SourceMap {
    SourceMap {
        source_path: source_map.source_path,
        entries: source_map
            .entries
            .into_iter()
            .map(|entry| SourceMapEntry {
                id: entry.id.as_str().to_string(),
                kind: convert_source_kind(entry.kind),
                line_start: entry.line_start as u32,
                line_end: entry.line_end as u32,
                byte_start: entry.byte_start as u64,
                byte_end: entry.byte_end as u64,
            })
            .collect(),
    }
}

dodeca_cell_runtime::declare_cell!("markdown", |host| {
    let processor = MarkdownProcessorImpl::new(host);
    MarkdownProcessorDispatcher::new(processor)
});
