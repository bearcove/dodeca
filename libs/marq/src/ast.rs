use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

fn heading_level_from_u8(n: u8) -> HeadingLevel {
    match n {
        1 => HeadingLevel::H1,
        2 => HeadingLevel::H2,
        3 => HeadingLevel::H3,
        4 => HeadingLevel::H4,
        5 => HeadingLevel::H5,
        _ => HeadingLevel::H6,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    BlockQuote(Vec<Block>),
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    List {
        ordered: bool,
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    ThematicBreak,
    Table {
        alignments: Vec<Alignment>,
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    HtmlBlock(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Code(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Link {
        url: String,
        title: String,
        content: Vec<Inline>,
    },
    Image {
        url: String,
        title: String,
        alt: Vec<Inline>,
    },
    SoftBreak,
    HardBreak,
    Html(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    None,
    Left,
    Center,
    Right,
}

fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_HEADING_ATTRIBUTES
}

/// Parse markdown string into block-level AST.
pub fn parse(markdown: &str) -> Vec<Block> {
    let parser = Parser::new_ext(markdown, parser_options());
    let events: Vec<Event<'_>> = parser.collect();
    parse_blocks(&events, &mut 0)
}

fn parse_blocks(events: &[Event<'_>], pos: &mut usize) -> Vec<Block> {
    let mut blocks = Vec::new();
    while *pos < events.len() {
        match &events[*pos] {
            Event::Start(Tag::Paragraph) => {
                *pos += 1;
                let inlines = parse_inlines(events, pos, TagEnd::Paragraph);
                blocks.push(Block::Paragraph(inlines));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let level = *level as u8;
                *pos += 1;
                let content =
                    parse_inlines(events, pos, TagEnd::Heading(heading_level_from_u8(level)));
                blocks.push(Block::Heading { level, content });
            }
            Event::Start(Tag::BlockQuote(_)) => {
                *pos += 1;
                let inner = parse_blocks_until_end(events, pos, |e| {
                    matches!(e, Event::End(TagEnd::BlockQuote(_)))
                });
                blocks.push(Block::BlockQuote(inner));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(lang) if !lang.is_empty() => Some(lang.to_string()),
                    _ => None,
                };
                *pos += 1;
                let mut code = String::new();
                while *pos < events.len() {
                    match &events[*pos] {
                        Event::Text(t) => {
                            code.push_str(t);
                            *pos += 1;
                        }
                        Event::End(TagEnd::CodeBlock) => {
                            *pos += 1;
                            break;
                        }
                        _ => {
                            *pos += 1;
                        }
                    }
                }
                blocks.push(Block::CodeBlock { language, code });
            }
            Event::Start(Tag::List(start_num)) => {
                let ordered = start_num.is_some();
                let start = *start_num;
                *pos += 1;
                let mut items = Vec::new();
                while *pos < events.len() {
                    match &events[*pos] {
                        Event::Start(Tag::Item) => {
                            *pos += 1;
                            let item_blocks = parse_blocks_until_end(events, pos, |e| {
                                matches!(e, Event::End(TagEnd::Item))
                            });
                            items.push(item_blocks);
                        }
                        Event::End(TagEnd::List(_)) => {
                            *pos += 1;
                            break;
                        }
                        _ => {
                            *pos += 1;
                        }
                    }
                }
                blocks.push(Block::List {
                    ordered,
                    start,
                    items,
                });
            }
            Event::Start(Tag::Table(alignments)) => {
                let alignments: Vec<Alignment> = alignments
                    .iter()
                    .map(|a| match a {
                        pulldown_cmark::Alignment::None => Alignment::None,
                        pulldown_cmark::Alignment::Left => Alignment::Left,
                        pulldown_cmark::Alignment::Center => Alignment::Center,
                        pulldown_cmark::Alignment::Right => Alignment::Right,
                    })
                    .collect();
                *pos += 1;
                let mut header = Vec::new();
                let mut rows = Vec::new();
                while *pos < events.len() {
                    match &events[*pos] {
                        Event::Start(Tag::TableHead) => {
                            *pos += 1;
                            header = parse_table_row(events, pos, TagEnd::TableHead);
                        }
                        Event::Start(Tag::TableRow) => {
                            *pos += 1;
                            let row = parse_table_row(events, pos, TagEnd::TableRow);
                            rows.push(row);
                        }
                        Event::End(TagEnd::Table) => {
                            *pos += 1;
                            break;
                        }
                        _ => {
                            *pos += 1;
                        }
                    }
                }
                blocks.push(Block::Table {
                    alignments,
                    header,
                    rows,
                });
            }
            Event::Rule => {
                *pos += 1;
                blocks.push(Block::ThematicBreak);
            }
            Event::Html(html) => {
                *pos += 1;
                blocks.push(Block::HtmlBlock(html.to_string()));
            }
            Event::End(_) => {
                // Don't consume — let the caller handle it
                break;
            }
            _ => {
                // Skip events we don't handle at block level (metadata, footnotes, etc.)
                *pos += 1;
            }
        }
    }
    blocks
}

fn parse_blocks_until_end(
    events: &[Event<'_>],
    pos: &mut usize,
    is_end: impl Fn(&Event<'_>) -> bool,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    while *pos < events.len() {
        if is_end(&events[*pos]) {
            *pos += 1;
            return blocks;
        }
        let sub = parse_blocks(events, pos);
        blocks.extend(sub);
    }
    blocks
}

fn parse_table_row(events: &[Event<'_>], pos: &mut usize, end: TagEnd) -> Vec<Vec<Inline>> {
    let mut cells = Vec::new();
    while *pos < events.len() {
        match &events[*pos] {
            Event::Start(Tag::TableCell) => {
                *pos += 1;
                let inlines = parse_inlines(events, pos, TagEnd::TableCell);
                cells.push(inlines);
            }
            e if *e == Event::End(end) => {
                *pos += 1;
                break;
            }
            _ => {
                *pos += 1;
            }
        }
    }
    cells
}

fn parse_inlines(events: &[Event<'_>], pos: &mut usize, end: TagEnd) -> Vec<Inline> {
    let mut inlines = Vec::new();
    while *pos < events.len() {
        match &events[*pos] {
            e if *e == Event::End(end) => {
                *pos += 1;
                return inlines;
            }
            Event::Text(t) => {
                inlines.push(Inline::Text(t.to_string()));
                *pos += 1;
            }
            Event::Code(c) => {
                inlines.push(Inline::Code(c.to_string()));
                *pos += 1;
            }
            Event::SoftBreak => {
                inlines.push(Inline::SoftBreak);
                *pos += 1;
            }
            Event::HardBreak => {
                inlines.push(Inline::HardBreak);
                *pos += 1;
            }
            Event::Html(h) => {
                inlines.push(Inline::Html(h.to_string()));
                *pos += 1;
            }
            Event::Start(Tag::Emphasis) => {
                *pos += 1;
                let inner = parse_inlines(events, pos, TagEnd::Emphasis);
                inlines.push(Inline::Emphasis(inner));
            }
            Event::Start(Tag::Strong) => {
                *pos += 1;
                let inner = parse_inlines(events, pos, TagEnd::Strong);
                inlines.push(Inline::Strong(inner));
            }
            Event::Start(Tag::Strikethrough) => {
                *pos += 1;
                let inner = parse_inlines(events, pos, TagEnd::Strikethrough);
                inlines.push(Inline::Strikethrough(inner));
            }
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => {
                let url = dest_url.to_string();
                let title = title.to_string();
                *pos += 1;
                let content = parse_inlines(events, pos, TagEnd::Link);
                inlines.push(Inline::Link {
                    url,
                    title,
                    content,
                });
            }
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                let url = dest_url.to_string();
                let title = title.to_string();
                *pos += 1;
                let alt = parse_inlines(events, pos, TagEnd::Image);
                inlines.push(Inline::Image { url, title, alt });
            }
            _ => {
                // Skip unknown inline events
                *pos += 1;
            }
        }
    }
    inlines
}

/// Render AST back to markdown string.
pub fn render_to_markdown(blocks: &[Block]) -> String {
    let mut out = String::new();
    render_blocks(&mut out, blocks);
    out
}

fn render_blocks(out: &mut String, blocks: &[Block]) {
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 && !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        match block {
            Block::Paragraph(inlines) => {
                render_inlines(out, inlines);
                out.push_str("\n\n");
            }
            Block::Heading { level, content } => {
                for _ in 0..*level {
                    out.push('#');
                }
                out.push(' ');
                render_inlines(out, content);
                out.push_str("\n\n");
            }
            Block::BlockQuote(inner) => {
                let mut inner_md = String::new();
                render_blocks(&mut inner_md, inner);
                // Trim trailing newlines from inner content so we can prefix cleanly
                let trimmed = inner_md.trim_end_matches('\n');
                for line in trimmed.split('\n') {
                    if line.is_empty() {
                        out.push_str(">\n");
                    } else {
                        out.push_str("> ");
                        out.push_str(line);
                        out.push('\n');
                    }
                }
                out.push('\n');
            }
            Block::CodeBlock { language, code } => {
                out.push_str("```");
                if let Some(lang) = language {
                    out.push_str(lang);
                }
                out.push('\n');
                out.push_str(code);
                if !code.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```\n\n");
            }
            Block::List {
                ordered,
                start,
                items,
            } => {
                let start_num = start.unwrap_or(1);
                for (j, item) in items.iter().enumerate() {
                    if *ordered {
                        out.push_str(&format!("{}. ", start_num + j as u64));
                    } else {
                        out.push_str("- ");
                    }
                    let mut item_md = String::new();
                    render_blocks(&mut item_md, item);
                    let trimmed = item_md.trim_end_matches('\n');
                    let mut first = true;
                    for line in trimmed.split('\n') {
                        if first {
                            out.push_str(line);
                            out.push('\n');
                            first = false;
                        } else if line.is_empty() {
                            out.push('\n');
                        } else {
                            out.push_str("  ");
                            out.push_str(line);
                            out.push('\n');
                        }
                    }
                }
                out.push('\n');
            }
            Block::ThematicBreak => {
                out.push_str("---\n\n");
            }
            Block::Table {
                alignments,
                header,
                rows,
            } => {
                render_table_row(out, header);
                // Separator
                out.push('|');
                for a in alignments {
                    match a {
                        Alignment::None => out.push_str(" --- |"),
                        Alignment::Left => out.push_str(" :-- |"),
                        Alignment::Center => out.push_str(" :-: |"),
                        Alignment::Right => out.push_str(" --: |"),
                    }
                }
                out.push('\n');
                for row in rows {
                    render_table_row(out, row);
                }
                out.push('\n');
            }
            Block::HtmlBlock(html) => {
                out.push_str(html);
                if !html.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
            }
        }
    }
}

fn render_table_row(out: &mut String, cells: &[Vec<Inline>]) {
    out.push('|');
    for cell in cells {
        out.push(' ');
        render_inlines(out, cell);
        out.push_str(" |");
    }
    out.push('\n');
}

fn render_inlines(out: &mut String, inlines: &[Inline]) {
    for inline in inlines {
        match inline {
            Inline::Text(t) => out.push_str(t),
            Inline::Code(c) => {
                out.push('`');
                out.push_str(c);
                out.push('`');
            }
            Inline::Emphasis(inner) => {
                out.push('*');
                render_inlines(out, inner);
                out.push('*');
            }
            Inline::Strong(inner) => {
                out.push_str("**");
                render_inlines(out, inner);
                out.push_str("**");
            }
            Inline::Strikethrough(inner) => {
                out.push_str("~~");
                render_inlines(out, inner);
                out.push_str("~~");
            }
            Inline::Link {
                url,
                title,
                content,
            } => {
                out.push('[');
                render_inlines(out, content);
                out.push_str("](");
                out.push_str(url);
                if !title.is_empty() {
                    out.push_str(" \"");
                    out.push_str(title);
                    out.push('"');
                }
                out.push(')');
            }
            Inline::Image { url, title, alt } => {
                out.push_str("![");
                render_inlines(out, alt);
                out.push_str("](");
                out.push_str(url);
                if !title.is_empty() {
                    out.push_str(" \"");
                    out.push_str(title);
                    out.push('"');
                }
                out.push(')');
            }
            Inline::SoftBreak => out.push('\n'),
            Inline::HardBreak => out.push_str("  \n"),
            Inline::Html(h) => out.push_str(h),
        }
    }
}

/// Flatten an inline tree to plain text (for diffing).
pub(crate) fn inline_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) => out.push_str(t),
            Inline::Code(c) => {
                out.push('`');
                out.push_str(c);
                out.push('`');
            }
            Inline::Emphasis(inner) | Inline::Strong(inner) | Inline::Strikethrough(inner) => {
                out.push_str(&inline_text(inner));
            }
            Inline::Link { content, .. } => out.push_str(&inline_text(content)),
            Inline::Image { alt, .. } => out.push_str(&inline_text(alt)),
            Inline::SoftBreak | Inline::HardBreak => out.push(' '),
            Inline::Html(h) => out.push_str(h),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_paragraph() {
        let md = "Hello world.\n";
        let blocks = parse(md);
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], Block::Paragraph(_)));
        let rendered = render_to_markdown(&blocks);
        let reparsed = parse(&rendered);
        assert_eq!(blocks, reparsed);
    }

    #[test]
    fn round_trip_heading() {
        let md = "## My Heading\n";
        let blocks = parse(md);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Heading { level, content } => {
                assert_eq!(*level, 2);
                assert_eq!(content.len(), 1);
            }
            other => panic!("expected heading, got {other:?}"),
        }
        let rendered = render_to_markdown(&blocks);
        let reparsed = parse(&rendered);
        assert_eq!(blocks, reparsed);
    }

    #[test]
    fn round_trip_blockquote() {
        let md = "> Quoted text.\n";
        let blocks = parse(md);
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], Block::BlockQuote(_)));
        let rendered = render_to_markdown(&blocks);
        let reparsed = parse(&rendered);
        assert_eq!(blocks, reparsed);
    }

    #[test]
    fn round_trip_code_block() {
        let md = "```rust\nfn main() {}\n```\n";
        let blocks = parse(md);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::CodeBlock { language, code } => {
                assert_eq!(language.as_deref(), Some("rust"));
                assert_eq!(code, "fn main() {}\n");
            }
            other => panic!("expected code block, got {other:?}"),
        }
        let rendered = render_to_markdown(&blocks);
        let reparsed = parse(&rendered);
        assert_eq!(blocks, reparsed);
    }

    #[test]
    fn round_trip_unordered_list() {
        let md = "- item one\n- item two\n- item three\n";
        let blocks = parse(md);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::List { ordered, items, .. } => {
                assert!(!ordered);
                assert_eq!(items.len(), 3);
            }
            other => panic!("expected list, got {other:?}"),
        }
        let rendered = render_to_markdown(&blocks);
        let reparsed = parse(&rendered);
        assert_eq!(blocks, reparsed);
    }

    #[test]
    fn round_trip_emphasis_strong() {
        let md = "Text with *emphasis* and **strong**.\n";
        let blocks = parse(md);
        let rendered = render_to_markdown(&blocks);
        let reparsed = parse(&rendered);
        assert_eq!(blocks, reparsed);
    }

    #[test]
    fn round_trip_link() {
        let md = "See [example](https://example.com) for details.\n";
        let blocks = parse(md);
        let rendered = render_to_markdown(&blocks);
        let reparsed = parse(&rendered);
        assert_eq!(blocks, reparsed);
    }

    #[test]
    fn round_trip_image() {
        let md = "![alt text](image.png)\n";
        let blocks = parse(md);
        let rendered = render_to_markdown(&blocks);
        let reparsed = parse(&rendered);
        assert_eq!(blocks, reparsed);
    }

    #[test]
    fn round_trip_nested_blockquote() {
        let md = "> outer\n>\n> > inner\n";
        let blocks = parse(md);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::BlockQuote(inner) => {
                assert!(inner.len() >= 2);
                assert!(inner.iter().any(|b| matches!(b, Block::BlockQuote(_))));
            }
            other => panic!("expected blockquote, got {other:?}"),
        }
        let rendered = render_to_markdown(&blocks);
        let reparsed = parse(&rendered);
        assert_eq!(blocks, reparsed);
    }

    #[test]
    fn round_trip_mixed_inline() {
        let md = "This has **bold *nested italic*** and `code`.\n";
        let blocks = parse(md);
        let rendered = render_to_markdown(&blocks);
        let reparsed = parse(&rendered);
        assert_eq!(blocks, reparsed);
    }

    #[test]
    fn round_trip_thematic_break() {
        let md = "Before.\n\n---\n\nAfter.\n";
        let blocks = parse(md);
        assert!(blocks.iter().any(|b| matches!(b, Block::ThematicBreak)));
        let rendered = render_to_markdown(&blocks);
        let reparsed = parse(&rendered);
        assert_eq!(blocks, reparsed);
    }

    #[test]
    fn round_trip_table() {
        let md = "| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n";
        let blocks = parse(md);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Table { header, rows, .. } => {
                assert_eq!(header.len(), 2);
                assert_eq!(rows.len(), 2);
            }
            other => panic!("expected table, got {other:?}"),
        }
        let rendered = render_to_markdown(&blocks);
        let reparsed = parse(&rendered);
        assert_eq!(blocks, reparsed);
    }
}
