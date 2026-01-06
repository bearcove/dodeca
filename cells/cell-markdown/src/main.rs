//! Dodeca markdown processing cell (cell-markdown)
//!
//! This cell uses bearmark for markdown rendering with direct code block rendering.

use bearmark::{AasvgHandler, ArboriumHandler, PikruHandler, RenderOptions, render};
use cell_markdown_proto::*;

#[derive(Clone)]
pub struct MarkdownProcessorImpl;

impl MarkdownProcessor for MarkdownProcessorImpl {
    async fn parse_frontmatter(&self, content: String) -> FrontmatterResult {
        match bearmark::parse_frontmatter(&content) {
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
        // Configure bearmark with real handlers (no placeholders!)
        let mut opts = RenderOptions::new()
            .with_handler(&["aa", "aasvg"], AasvgHandler::new())
            .with_handler(&["pikchr"], PikruHandler::new())
            .with_default_handler(ArboriumHandler::new());

        // Set source path for link resolution
        opts.source_path = Some(source_path);

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
        let (fm, body) = match bearmark::parse_frontmatter(&content) {
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

// Helper functions to convert bearmark types to protocol types
fn convert_frontmatter(fm: bearmark::Frontmatter) -> Frontmatter {
    Frontmatter {
        title: fm.title,
        weight: fm.weight,
        description: fm.description,
        template: fm.template,
        extra: fm.extra, // Direct pass-through, no JSON conversion!
    }
}

fn convert_heading(h: bearmark::Heading) -> Heading {
    Heading {
        title: h.title,
        id: h.id,
        level: h.level,
    }
}

fn convert_req(r: bearmark::ReqDefinition) -> ReqDefinition {
    ReqDefinition {
        id: r.id,
        anchor_id: r.anchor_id,
    }
}

rapace_cell::cell_service!(
    MarkdownProcessorServer<MarkdownProcessorImpl>,
    MarkdownProcessorImpl
);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run(CellService::from(MarkdownProcessorImpl)).await?;
    Ok(())
}
