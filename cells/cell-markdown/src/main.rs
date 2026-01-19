//! Dodeca markdown processing cell (cell-markdown)
//!
//! This cell uses marq for markdown rendering with direct code block rendering.

use cell_markdown_proto::*;
use dodeca_cell_runtime::run_cell;
use marq::{
    AasvgHandler, ArboriumHandler, CompareHandler, InlineCodeHandler, LinkResolver, PikruHandler,
    RenderOptions, TermHandler, render,
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

/// Inline code handler that converts `r[rule.name]` to links.
/// Links to #r-rule.name anchors on the same page.
struct RuleRefHandler;

impl InlineCodeHandler for RuleRefHandler {
    fn render(&self, code: &str) -> Option<String> {
        let code = code.trim();

        // Match r[rule.id] pattern
        if !code.starts_with("r[") || !code.ends_with(']') {
            return None;
        }

        // Extract rule.id from r[rule.id]
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
            if link.starts_with("@/") {
                Some(link.to_string())
            } else {
                // Let marq handle other links (relative .md, external, etc.)
                None
            }
        })
    }
}

#[derive(Clone)]
pub struct MarkdownProcessorImpl;

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

    async fn render_markdown(&self, source_path: String, markdown: String) -> MarkdownResult {
        // Configure marq with real handlers (no placeholders!)
        let opts = RenderOptions::new()
            .with_handler(&["aa", "aasvg"], AasvgHandler::new())
            .with_handler(&["compare"], CompareHandler::new())
            .with_handler(&["pikchr"], PikruHandler::new())
            .with_handler(&["term"], TermHandler::new())
            .with_default_handler(ArboriumHandler::new())
            .with_source_path(&source_path)
            // Pass through @/ links unchanged - dodeca will resolve them with site tree
            .with_link_resolver(PassthroughLinkResolver)
            // Convert `r[rule.name]` inline code to links
            .with_inline_code_handler(RuleRefHandler);

        // Render markdown with all code blocks rendered inline
        match render(&markdown, &opts).await {
            Ok(doc) => MarkdownResult::Success {
                html: doc.html, // Fully rendered, no placeholders
                headings: doc.headings.into_iter().map(convert_heading).collect(),
                reqs: doc.reqs.into_iter().map(convert_req).collect(),
            },
            Err(e) => MarkdownResult::Error {
                message: e.to_string(),
            },
        }
    }

    async fn parse_and_render(&self, source_path: String, content: String) -> ParseResult {
        // Parse frontmatter
        let (fm, body) = match marq::parse_frontmatter(&content) {
            Ok(result) => result,
            Err(e) => {
                return ParseResult::Error {
                    message: e.to_string(),
                };
            }
        };

        // Render markdown body
        match self.render_markdown(source_path, body.to_string()).await {
            MarkdownResult::Success {
                html,
                headings,
                reqs,
            } => ParseResult::Success {
                frontmatter: convert_frontmatter(fm),
                html,
                headings,
                reqs,
            },
            MarkdownResult::Error { message } => ParseResult::Error { message },
        }
    }
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
        id: r.id,
        anchor_id: r.anchor_id,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("markdown", |_handle| {
        MarkdownProcessorDispatcher::new(MarkdownProcessorImpl)
    })
}
