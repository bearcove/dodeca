use crate::ast::{self, Block, Inline, inline_text};

#[derive(Debug)]
enum DiffOp<T> {
    Equal(T),
    Remove(T),
    Add(T),
}

/// LCS-based sequence diff.
fn diff_sequences<'a, T: PartialEq>(old: &'a [T], new: &'a [T]) -> Vec<DiffOp<&'a T>> {
    let m = old.len();
    let n = new.len();

    // Build LCS table
    let mut table = vec![vec![0u32; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            if old[i - 1] == new[j - 1] {
                table[i][j] = table[i - 1][j - 1] + 1;
            } else {
                table[i][j] = table[i - 1][j].max(table[i][j - 1]);
            }
        }
    }

    // Backtrack
    let mut ops = Vec::new();
    let mut i = m;
    let mut j = n;
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            ops.push(DiffOp::Equal(&old[i - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || table[i][j - 1] >= table[i - 1][j]) {
            ops.push(DiffOp::Add(&new[j - 1]));
            j -= 1;
        } else {
            ops.push(DiffOp::Remove(&old[i - 1]));
            i -= 1;
        }
    }
    ops.reverse();
    ops
}

/// Diff two markdown strings and produce markdown with change markers.
/// Removed content wrapped in `~~strikethrough~~`, added in `**bold**`.
pub fn diff_markdown(old: &str, new: &str) -> String {
    let old_blocks = ast::parse(old);
    let new_blocks = ast::parse(new);
    let diff_ops = diff_sequences(&old_blocks, &new_blocks);

    let mut result_blocks: Vec<Block> = Vec::new();
    for op in diff_ops {
        match op {
            DiffOp::Equal(block) => {
                result_blocks.push(block.clone());
            }
            DiffOp::Remove(block) => {
                result_blocks.push(wrap_block_removed(block));
            }
            DiffOp::Add(block) => {
                result_blocks.push(wrap_block_added(block));
            }
        }
    }

    ast::render_to_markdown(&result_blocks)
}

/// Check if two blocks are the same variant (suitable for inline diffing).
fn same_variant(a: &Block, b: &Block) -> bool {
    matches!(
        (a, b),
        (Block::Paragraph(_), Block::Paragraph(_))
            | (Block::Heading { .. }, Block::Heading { .. })
            | (Block::BlockQuote(_), Block::BlockQuote(_))
            | (Block::CodeBlock { .. }, Block::CodeBlock { .. })
    )
}

/// Diff two markdown strings with inline-level diffing for changed blocks.
/// This is the smarter version that tries to match up structurally similar blocks.
pub fn diff_markdown_inline(old: &str, new: &str) -> String {
    let old_blocks = ast::parse(old);
    let new_blocks = ast::parse(new);
    let diff_ops = diff_sequences(&old_blocks, &new_blocks);

    // Collapse consecutive Remove/Add pairs of the same variant into inline diffs
    let mut result_blocks: Vec<Block> = Vec::new();
    let mut pending_removes: Vec<&Block> = Vec::new();

    for op in &diff_ops {
        match op {
            DiffOp::Equal(block) => {
                flush_removes(&mut result_blocks, &mut pending_removes);
                result_blocks.push((*block).clone());
            }
            DiffOp::Remove(block) => {
                pending_removes.push(block);
            }
            DiffOp::Add(block) => {
                // Try to pair with a pending remove of the same variant
                if let Some(idx) = pending_removes.iter().position(|r| same_variant(r, block)) {
                    let removed = pending_removes.remove(idx);
                    result_blocks.push(diff_block_inline(removed, block));
                } else {
                    flush_removes(&mut result_blocks, &mut pending_removes);
                    result_blocks.push(wrap_block_added(block));
                }
            }
        }
    }
    flush_removes(&mut result_blocks, &mut pending_removes);

    ast::render_to_markdown(&result_blocks)
}

fn flush_removes(result: &mut Vec<Block>, pending: &mut Vec<&Block>) {
    for block in pending.drain(..) {
        result.push(wrap_block_removed(block));
    }
}

fn diff_block_inline(old: &Block, new: &Block) -> Block {
    match (old, new) {
        (Block::Paragraph(old_inlines), Block::Paragraph(new_inlines)) => {
            Block::Paragraph(diff_inlines(old_inlines, new_inlines))
        }
        (
            Block::Heading {
                level,
                content: old_content,
            },
            Block::Heading {
                content: new_content,
                ..
            },
        ) => Block::Heading {
            level: *level,
            content: diff_inlines(old_content, new_content),
        },
        (Block::BlockQuote(old_inner), Block::BlockQuote(new_inner)) => {
            let inner_old_md = ast::render_to_markdown(old_inner);
            let inner_new_md = ast::render_to_markdown(new_inner);
            let diffed = diff_markdown_inline(&inner_old_md, &inner_new_md);
            Block::BlockQuote(ast::parse(&diffed))
        }
        (
            Block::CodeBlock {
                language: old_lang,
                code: old_code,
            },
            Block::CodeBlock {
                language: new_lang,
                code: new_code,
            },
        ) => {
            if old_code == new_code && old_lang == new_lang {
                new.clone()
            } else {
                // Show old code struck through, new code bold
                let mut inlines = Vec::new();
                if old_code != new_code || old_lang != new_lang {
                    inlines.push(Inline::Strikethrough(vec![Inline::Text(format!(
                        "```{}\n{}\n```",
                        old_lang.as_deref().unwrap_or(""),
                        old_code.trim_end_matches('\n')
                    ))]));
                    inlines.push(Inline::SoftBreak);
                    inlines.push(Inline::Strong(vec![Inline::Text(format!(
                        "```{}\n{}\n```",
                        new_lang.as_deref().unwrap_or(""),
                        new_code.trim_end_matches('\n')
                    ))]));
                }
                Block::Paragraph(inlines)
            }
        }
        _ => {
            // Fallback: show removed then added
            wrap_block_removed(old)
        }
    }
}

fn diff_inlines(old: &[Inline], new: &[Inline]) -> Vec<Inline> {
    // Flatten to words for word-level diffing
    let old_text = inline_text(old);
    let new_text = inline_text(new);

    let old_words: Vec<&str> = old_text.split_whitespace().collect();
    let new_words: Vec<&str> = new_text.split_whitespace().collect();

    let ops = diff_sequences(&old_words, &new_words);

    let mut result = Vec::new();
    let mut removed_run = Vec::new();
    let mut added_run = Vec::new();

    let flush_runs =
        |result: &mut Vec<Inline>, removed: &mut Vec<String>, added: &mut Vec<String>| {
            if !removed.is_empty() {
                if !result.is_empty() {
                    result.push(Inline::Text(" ".to_string()));
                }
                result.push(Inline::Strikethrough(vec![Inline::Text(
                    std::mem::take(removed).join(" "),
                )]));
            }
            if !added.is_empty() {
                if !result.is_empty() {
                    result.push(Inline::Text(" ".to_string()));
                }
                result.push(Inline::Strong(vec![Inline::Text(
                    std::mem::take(added).join(" "),
                )]));
            }
        };

    for op in ops {
        match op {
            DiffOp::Equal(word) => {
                flush_runs(&mut result, &mut removed_run, &mut added_run);
                if !result.is_empty() {
                    result.push(Inline::Text(" ".to_string()));
                }
                result.push(Inline::Text((*word).to_string()));
            }
            DiffOp::Remove(word) => {
                removed_run.push((*word).to_string());
            }
            DiffOp::Add(word) => {
                added_run.push((*word).to_string());
            }
        }
    }
    flush_runs(&mut result, &mut removed_run, &mut added_run);

    result
}

fn wrap_block_removed(block: &Block) -> Block {
    match block {
        Block::Paragraph(inlines) => Block::Paragraph(vec![Inline::Strikethrough(inlines.clone())]),
        Block::Heading { level, content } => Block::Heading {
            level: *level,
            content: vec![Inline::Strikethrough(content.clone())],
        },
        Block::CodeBlock { language, code } => {
            Block::Paragraph(vec![Inline::Strikethrough(vec![Inline::Text(format!(
                "```{}\n{}\n```",
                language.as_deref().unwrap_or(""),
                code.trim_end_matches('\n')
            ))])])
        }
        Block::BlockQuote(inner) => {
            let wrapped: Vec<Block> = inner.iter().map(wrap_block_removed).collect();
            Block::BlockQuote(wrapped)
        }
        Block::List {
            ordered,
            start,
            items,
        } => {
            let wrapped_items: Vec<Vec<Block>> = items
                .iter()
                .map(|item| item.iter().map(wrap_block_removed).collect())
                .collect();
            Block::List {
                ordered: *ordered,
                start: *start,
                items: wrapped_items,
            }
        }
        Block::ThematicBreak => Block::Paragraph(vec![Inline::Strikethrough(vec![Inline::Text(
            "---".to_string(),
        )])]),
        Block::Table {
            alignments,
            header,
            rows,
        } => {
            let wrap_cells = |cells: &[Vec<Inline>]| -> Vec<Vec<Inline>> {
                cells
                    .iter()
                    .map(|cell| vec![Inline::Strikethrough(cell.clone())])
                    .collect()
            };
            Block::Table {
                alignments: alignments.clone(),
                header: wrap_cells(header),
                rows: rows.iter().map(|row| wrap_cells(row)).collect(),
            }
        }
        Block::HtmlBlock(html) => {
            Block::Paragraph(vec![Inline::Strikethrough(vec![Inline::Text(
                html.clone(),
            )])])
        }
    }
}

fn wrap_block_added(block: &Block) -> Block {
    match block {
        Block::Paragraph(inlines) => Block::Paragraph(vec![Inline::Strong(inlines.clone())]),
        Block::Heading { level, content } => Block::Heading {
            level: *level,
            content: vec![Inline::Strong(content.clone())],
        },
        Block::CodeBlock { language, code } => {
            Block::Paragraph(vec![Inline::Strong(vec![Inline::Text(format!(
                "```{}\n{}\n```",
                language.as_deref().unwrap_or(""),
                code.trim_end_matches('\n')
            ))])])
        }
        Block::BlockQuote(inner) => {
            let wrapped: Vec<Block> = inner.iter().map(wrap_block_added).collect();
            Block::BlockQuote(wrapped)
        }
        Block::List {
            ordered,
            start,
            items,
        } => {
            let wrapped_items: Vec<Vec<Block>> = items
                .iter()
                .map(|item| item.iter().map(wrap_block_added).collect())
                .collect();
            Block::List {
                ordered: *ordered,
                start: *start,
                items: wrapped_items,
            }
        }
        Block::ThematicBreak => {
            Block::Paragraph(vec![Inline::Strong(vec![Inline::Text("---".to_string())])])
        }
        Block::Table {
            alignments,
            header,
            rows,
        } => {
            let wrap_cells = |cells: &[Vec<Inline>]| -> Vec<Vec<Inline>> {
                cells
                    .iter()
                    .map(|cell| vec![Inline::Strong(cell.clone())])
                    .collect()
            };
            Block::Table {
                alignments: alignments.clone(),
                header: wrap_cells(header),
                rows: rows.iter().map(|row| wrap_cells(row)).collect(),
            }
        }
        Block::HtmlBlock(html) => {
            Block::Paragraph(vec![Inline::Strong(vec![Inline::Text(html.clone())])])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_text() {
        let md = "Hello world.\n";
        let result = diff_markdown(md, md);
        // Should contain the text without markers
        assert!(result.contains("Hello world."));
        assert!(!result.contains("~~"));
        assert!(!result.contains("**"));
    }

    #[test]
    fn word_change_in_paragraph() {
        let old = "The quick brown fox.\n";
        let new = "The slow brown fox.\n";
        let result = diff_markdown_inline(old, new);
        assert!(
            result.contains("~~quick~~"),
            "should strike old word: {result}"
        );
        assert!(
            result.contains("**slow**"),
            "should bold new word: {result}"
        );
        assert!(
            result.contains("The"),
            "unchanged words preserved: {result}"
        );
        assert!(
            result.contains("brown"),
            "unchanged words preserved: {result}"
        );
        assert!(
            result.contains("fox."),
            "unchanged words preserved: {result}"
        );
    }

    #[test]
    fn added_paragraph() {
        let old = "First paragraph.\n";
        let new = "First paragraph.\n\nSecond paragraph.\n";
        let result = diff_markdown(old, new);
        assert!(
            result.contains("First paragraph."),
            "original preserved: {result}"
        );
        assert!(
            result.contains("**Second paragraph.**"),
            "added block bolded: {result}"
        );
    }

    #[test]
    fn removed_paragraph() {
        let old = "First paragraph.\n\nSecond paragraph.\n";
        let new = "First paragraph.\n";
        let result = diff_markdown(old, new);
        assert!(
            result.contains("First paragraph."),
            "remaining preserved: {result}"
        );
        assert!(
            result.contains("~~Second paragraph.~~"),
            "removed block struck: {result}"
        );
    }

    #[test]
    fn blockquote_structure_preserved() {
        let old = "> Important rule.\n";
        let new = "> Updated rule.\n";
        let result = diff_markdown_inline(old, new);
        assert!(result.contains("> "), "blockquote preserved: {result}");
    }

    #[test]
    fn code_block_change() {
        let old = "```rust\nfn old() {}\n```\n";
        let new = "```rust\nfn new() {}\n```\n";
        let result = diff_markdown_inline(old, new);
        // Code blocks changed as whole units
        assert!(result.contains("~~"), "old code struck: {result}");
        assert!(result.contains("**"), "new code bolded: {result}");
    }

    #[test]
    fn unchanged_code_block() {
        let md = "```rust\nfn main() {}\n```\n";
        let result = diff_markdown_inline(md, md);
        assert!(result.contains("fn main()"));
        assert!(!result.contains("~~"));
        assert!(!result.contains("**"));
    }

    #[test]
    fn multiple_word_changes() {
        let old = "This is the old text with some words.\n";
        let new = "This is the new text with different words.\n";
        let result = diff_markdown_inline(old, new);
        assert!(result.contains("~~old~~"), "old word struck: {result}");
        assert!(result.contains("**new**"), "new word bolded: {result}");
        assert!(result.contains("~~some~~"), "old word struck: {result}");
        assert!(
            result.contains("**different**"),
            "new word bolded: {result}"
        );
    }

    #[test]
    fn heading_change() {
        let old = "## Old Title\n";
        let new = "## New Title\n";
        let result = diff_markdown_inline(old, new);
        assert!(result.contains("##"), "heading preserved: {result}");
        assert!(result.contains("~~Old~~"), "old struck: {result}");
        assert!(result.contains("**New**"), "new bolded: {result}");
    }

    #[test]
    fn diff_sequences_basic() {
        let old = vec![1, 2, 3, 4, 5];
        let new = vec![1, 3, 4, 6];
        let ops = diff_sequences(&old, &new);
        let equal: Vec<_> = ops
            .iter()
            .filter_map(|op| match op {
                DiffOp::Equal(v) => Some(**v),
                _ => None,
            })
            .collect();
        assert_eq!(equal, vec![1, 3, 4]);
    }
}
