//! Dodeca markdown processing cell (cell-markdown)
//!
//! This cell handles:
//! - Markdown to HTML conversion (pulldown-cmark)
//! - Frontmatter parsing (TOML)
//! - Heading extraction
//! - Code block extraction (for syntax highlighting by host)

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, html};

use cell_markdown_proto::{
    CodeBlock, Frontmatter, FrontmatterResult, Heading, MarkdownProcessor, MarkdownProcessorServer,
    MarkdownResult, ParseResult, RuleDefinition,
};

/// Markdown processor implementation
#[derive(Clone)]
pub struct MarkdownProcessorImpl;

impl MarkdownProcessor for MarkdownProcessorImpl {
    async fn parse_frontmatter(&self, content: String) -> FrontmatterResult {
        let (frontmatter_str, body) = split_frontmatter(&content);

        if frontmatter_str.is_empty() {
            return FrontmatterResult::Success {
                frontmatter: Frontmatter::default(),
                body,
            };
        }

        match parse_frontmatter_toml(&frontmatter_str) {
            Ok(fm) => FrontmatterResult::Success {
                frontmatter: fm,
                body,
            },
            Err(e) => FrontmatterResult::Error { message: e },
        }
    }

    async fn render_markdown(&self, source_path: String, markdown: String) -> MarkdownResult {
        match render_markdown_impl(&source_path, &markdown) {
            Ok((html, headings, code_blocks, rules)) => MarkdownResult::Success {
                html,
                headings,
                code_blocks,
                rules,
            },
            Err(e) => MarkdownResult::Error { message: e },
        }
    }

    async fn parse_and_render(&self, source_path: String, content: String) -> ParseResult {
        let (frontmatter_str, body) = split_frontmatter(&content);

        let frontmatter = if frontmatter_str.is_empty() {
            Frontmatter::default()
        } else {
            match parse_frontmatter_toml(&frontmatter_str) {
                Ok(fm) => fm,
                Err(e) => {
                    return ParseResult::Error { message: e };
                }
            }
        };

        match render_markdown_impl(&source_path, &body) {
            Ok((html, headings, code_blocks, rules)) => ParseResult::Success {
                frontmatter,
                html,
                headings,
                code_blocks,
                rules,
            },
            Err(e) => ParseResult::Error { message: e },
        }
    }
}

// ============================================================================
// Implementation helpers
// ============================================================================

/// Split frontmatter from content.
/// Supports +++ delimiters (TOML frontmatter).
fn split_frontmatter(content: &str) -> (String, String) {
    let content = content.trim_start();

    // Check for +++ delimiters (TOML frontmatter)
    if let Some(rest) = content.strip_prefix("+++")
        && let Some(end) = rest.find("+++")
    {
        let frontmatter = rest[..end].trim().to_string();
        let body = rest[end + 3..].trim_start().to_string();
        return (frontmatter, body);
    }

    // No frontmatter found
    (String::new(), content.to_string())
}

/// Internal struct for TOML parsing with optional fields
#[derive(Debug, Default, facet::Facet)]
struct FrontmatterToml {
    #[facet(default)]
    title: String,
    #[facet(default)]
    weight: i32,
    description: Option<String>,
    template: Option<String>,
    #[facet(default)]
    extra: facet_value::Value,
}

/// Parse TOML frontmatter into Frontmatter struct
fn parse_frontmatter_toml(toml_str: &str) -> Result<Frontmatter, String> {
    // Parse directly into our struct with defaults
    let parsed: FrontmatterToml =
        facet_toml::from_str(toml_str).map_err(|e| format!("TOML parse error: {}", e))?;

    // Convert extra Value to JSON string
    let extra_json = facet_json::to_string(&parsed.extra);

    Ok(Frontmatter {
        title: parsed.title,
        weight: parsed.weight,
        description: parsed.description,
        template: parsed.template,
        extra_json,
    })
}

/// Preprocess markdown to extract rule identifiers.
///
/// Rules are lines matching `r[rule.id]` (optionally followed by content on the next line).
/// We replace them with HTML directly since pulldown_cmark would merge them with following text.
///
/// Returns (processed_markdown, rules).
fn preprocess_rules(markdown: &str) -> Result<(String, Vec<RuleDefinition>), String> {
    use std::collections::HashSet;

    let mut result = String::with_capacity(markdown.len());
    let mut rules = Vec::new();
    let mut seen_rule_ids: HashSet<String> = HashSet::new();

    for line in markdown.lines() {
        let trimmed = line.trim();

        // Check if this line is a rule identifier: r[rule.id]
        if trimmed.starts_with("r[") && trimmed.ends_with(']') && trimmed.len() > 3 {
            let rule_id = &trimmed[2..trimmed.len() - 1];

            // Check for duplicates
            if !seen_rule_ids.insert(rule_id.to_string()) {
                return Err(format!("duplicate rule identifier: r[{}]", rule_id));
            }

            let anchor_id = format!("r-{}", rule_id);
            rules.push(RuleDefinition {
                id: rule_id.to_string(),
                anchor_id: anchor_id.clone(),
            });

            // Emit rule HTML directly (will pass through pulldown_cmark as raw HTML)
            result.push_str(&rule_to_html(rule_id, &anchor_id));
            result.push('\n');
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    Ok((result, rules))
}

/// Generate HTML for a rule anchor badge
fn rule_to_html(rule_id: &str, anchor_id: &str) -> String {
    // Insert <wbr> after dots for better line breaking
    let display_id = rule_id.replace('.', ".<wbr>");
    format!(
        "<div class=\"rule\" id=\"{anchor_id}\"><a class=\"rule-link\" href=\"#{anchor_id}\" title=\"{rule_id}\"><span>[{display_id}]</span></a></div>"
    )
}

/// Render markdown to HTML with heading and code block extraction
#[allow(clippy::type_complexity)]
fn render_markdown_impl(
    source_path: &str,
    markdown: &str,
) -> Result<(String, Vec<Heading>, Vec<CodeBlock>, Vec<RuleDefinition>), String> {
    // Preprocess to extract rule identifiers before pulldown_cmark
    let (preprocessed, rules) = preprocess_rules(markdown)?;

    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_HEADING_ATTRIBUTES;

    let parser = Parser::new_ext(&preprocessed, options);

    let mut headings = Vec::new();
    let mut current_heading: Option<(u8, String, String)> = None; // (level, id, text)

    // Track code block state
    let mut in_code_block = false;
    let mut code_block_lang = String::new();
    let mut code_block_content = String::new();

    // Collect code blocks for later highlighting by host
    let mut code_blocks: Vec<CodeBlock> = Vec::new();

    let mut output_events: Vec<Event> = Vec::new();

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(ref lang))) => {
                in_code_block = true;
                code_block_lang = lang.to_string();
                code_block_content.clear();
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)) => {
                in_code_block = true;
                code_block_lang.clear();
                code_block_content.clear();
            }
            Event::End(pulldown_cmark::TagEnd::CodeBlock) => {
                if in_code_block {
                    // Check if this is an ASCII diagram block
                    if code_block_lang == "aa" {
                        // Render ASCII art to SVG using aasvg
                        let options = aasvg::RenderOptions::new().with_stretch(true);
                        let svg = aasvg::render_with_options(&code_block_content, &options);
                        output_events.push(Event::Html(svg.into()));
                        code_block_content.clear();
                        code_block_lang.clear();
                    } else {
                        let idx = code_blocks.len();
                        let placeholder = format!("<!--CODE_BLOCK_PLACEHOLDER_{}-->", idx);
                        code_blocks.push(CodeBlock {
                            code: std::mem::take(&mut code_block_content),
                            language: std::mem::take(&mut code_block_lang),
                            placeholder: placeholder.clone(),
                        });
                        output_events.push(Event::Html(placeholder.into()));
                    }
                    in_code_block = false;
                }
            }
            Event::Text(ref text) if in_code_block => {
                code_block_content.push_str(text);
            }
            Event::Start(Tag::Heading { level, ref id, .. }) => {
                let level_num = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                current_heading = Some((
                    level_num,
                    id.as_ref().map(|s| s.to_string()).unwrap_or_default(),
                    String::new(),
                ));
                output_events.push(event);
            }
            Event::End(pulldown_cmark::TagEnd::Heading(_)) => {
                if let Some((level, id, text)) = current_heading.take() {
                    let id = if id.is_empty() { slugify(&text) } else { id };
                    headings.push(Heading {
                        title: text,
                        id,
                        level,
                    });
                }
                output_events.push(event);
            }
            Event::Text(ref text) | Event::Code(ref text) => {
                if let Some((_, _, ref mut heading_text)) = current_heading {
                    heading_text.push_str(text);
                }
                output_events.push(event);
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                // Resolve internal links (@/ absolute and relative .md)
                let resolved = resolve_internal_link(&dest_url, source_path);
                output_events.push(Event::Start(Tag::Link {
                    link_type,
                    dest_url: resolved.into(),
                    title,
                    id,
                }));
            }
            other => {
                output_events.push(other);
            }
        }
    }

    // Generate HTML
    let mut html_output = String::new();
    html::push_html(&mut html_output, output_events.into_iter());

    // Inject id attributes into headings
    let html_output = inject_heading_ids(&html_output, &headings);

    Ok((html_output, headings, code_blocks, rules))
}

/// Convert text to a URL-safe slug for heading IDs
fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Resolve internal links (both @/ absolute and relative .md links)
fn resolve_internal_link(link: &str, source_path: &str) -> String {
    // Handle absolute @/ links
    if let Some(path) = link.strip_prefix("@/") {
        return resolve_absolute_link(path);
    }

    // Handle relative .md links
    if link.ends_with(".md") && !link.starts_with("http://") && !link.starts_with("https://") {
        return resolve_relative_link(link, source_path);
    }

    // Pass through all other links unchanged (external URLs, fragments, etc.)
    link.to_string()
}

/// Resolve @/path/to/file.md links to absolute URLs
fn resolve_absolute_link(path: &str) -> String {
    // Split off fragment
    let (path_part, fragment) = match path.find('#') {
        Some(idx) => (&path[..idx], Some(&path[idx..])),
        None => (path, None),
    };

    let mut path = path_part.to_string();

    // Remove .md extension
    if path.ends_with(".md") {
        path = path[..path.len() - 3].to_string();
    }

    // Handle _index -> parent directory
    if path.ends_with("/_index") {
        path = path[..path.len() - 7].to_string();
    } else if path == "_index" {
        path = String::new();
    }

    // Ensure leading slash
    let result = if path.is_empty() {
        "/".to_string()
    } else {
        format!("/{}/", path)
    };

    // Append fragment if present
    match fragment {
        Some(f) => format!("{}{}", result, f),
        None => result,
    }
}

/// Resolve relative .md links based on current file location
fn resolve_relative_link(link: &str, source_path: &str) -> String {
    use std::path::Path;

    // Split off fragment
    let (link_part, fragment) = match link.find('#') {
        Some(idx) => (&link[..idx], Some(&link[idx..])),
        None => (link, None),
    };

    // Get the directory of the source file
    let source = Path::new(source_path);
    let source_dir = source.parent().unwrap_or(Path::new(""));

    // Resolve the relative link against the source directory
    let resolved = source_dir.join(link_part);

    // Convert to string and normalize
    let mut path = resolved
        .to_str()
        .unwrap_or(link)
        .to_string()
        .replace('\\', "/"); // Normalize Windows paths

    // Remove .md extension
    if path.ends_with(".md") {
        path = path[..path.len() - 3].to_string();
    }

    // Handle _index -> parent directory
    if path.ends_with("/_index") {
        path = path[..path.len() - 7].to_string();
    } else if path == "_index" {
        path = String::new();
    }

    // Ensure leading slash and trailing slash
    let result = if path.is_empty() {
        "/".to_string()
    } else if path.starts_with('/') {
        format!("{}/", path)
    } else {
        format!("/{}/", path)
    };

    // Append fragment if present
    match fragment {
        Some(f) => format!("{}{}", result, f),
        None => result,
    }
}

/// Inject id attributes into heading tags that don't have them
fn inject_heading_ids(html: &str, headings: &[Heading]) -> String {
    let mut result = html.to_string();

    for heading in headings {
        // Look for heading tags without id attribute
        for level in 1..=6 {
            let tag_without_id = format!("<h{}>", level);
            let tag_with_id = format!("<h{} id=\"{}\">", level, heading.id);

            // Only replace the first occurrence that matches the heading text
            if result.contains(&tag_without_id) {
                result = result.replacen(&tag_without_id, &tag_with_id, 1);
                break;
            }
        }
    }

    result
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
