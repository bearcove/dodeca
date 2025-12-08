//! Benchmarks for HTML DOM diffing
//!
//! Run with: cargo bench --bench html_diff

use divan::{black_box, Bencher};
use dodeca::html_diff::{diff, DomNode};
use std::collections::HashMap;

fn main() {
    divan::main();
}

// ============================================================================
// Page generators
// ============================================================================

fn with_class(tag: &str, class: &str, children: Vec<DomNode>) -> DomNode {
    let mut attrs = HashMap::new();
    attrs.insert("class".to_string(), class.to_string());
    DomNode::element(tag, attrs, children)
}

/// Generate a realistic large page for benchmarking
/// Simulates a documentation site with:
/// - Header with nav
/// - Sidebar with table of contents
/// - Main content with many sections, each with paragraphs and code blocks
/// - Footer
fn generate_large_page(num_sections: usize, paragraphs_per_section: usize) -> DomNode {
    let mut attrs = HashMap::new();

    // Header
    let header = with_class(
        "header",
        "site-header",
        vec![with_class(
            "nav",
            "main-nav",
            vec![
                DomNode::element(
                    "a",
                    {
                        let mut a = HashMap::new();
                        a.insert("href".to_string(), "/".to_string());
                        a
                    },
                    vec![DomNode::text("Home")],
                ),
                DomNode::element(
                    "a",
                    {
                        let mut a = HashMap::new();
                        a.insert("href".to_string(), "/docs".to_string());
                        a
                    },
                    vec![DomNode::text("Docs")],
                ),
                DomNode::element(
                    "a",
                    {
                        let mut a = HashMap::new();
                        a.insert("href".to_string(), "/blog".to_string());
                        a
                    },
                    vec![DomNode::text("Blog")],
                ),
            ],
        )],
    );

    // Sidebar TOC
    let toc_items: Vec<DomNode> = (0..num_sections)
        .map(|i| {
            DomNode::element(
                "li",
                HashMap::new(),
                vec![DomNode::element(
                    "a",
                    {
                        let mut a = HashMap::new();
                        a.insert("href".to_string(), format!("#section-{}", i));
                        a
                    },
                    vec![DomNode::text(format!("Section {}", i))],
                )],
            )
        })
        .collect();
    let sidebar = with_class(
        "aside",
        "sidebar",
        vec![with_class(
            "nav",
            "toc",
            vec![DomNode::element("ul", HashMap::new(), toc_items)],
        )],
    );

    // Main content with sections
    let sections: Vec<DomNode> = (0..num_sections)
        .map(|section_idx| {
            let mut section_attrs = HashMap::new();
            section_attrs.insert("id".to_string(), format!("section-{}", section_idx));

            let heading = DomNode::element(
                "h2",
                HashMap::new(),
                vec![DomNode::text(format!(
                    "Section {} - Important Topic",
                    section_idx
                ))],
            );

            let paragraphs: Vec<DomNode> = (0..paragraphs_per_section)
                .flat_map(|para_idx| {
                    vec![
                        DomNode::element(
                            "p",
                            HashMap::new(),
                            vec![DomNode::text(format!(
                                "This is paragraph {} of section {}. It contains some text \
                                 that simulates real documentation content with enough words \
                                 to be realistic. Lorem ipsum dolor sit amet.",
                                para_idx, section_idx
                            ))],
                        ),
                        // Add a code block every few paragraphs
                        if para_idx % 3 == 2 {
                            with_class(
                                "pre",
                                "code-block",
                                vec![DomNode::element(
                                    "code",
                                    HashMap::new(),
                                    vec![DomNode::text(format!(
                                        "fn example_{}() {{\n    let x = {};\n    println!(\"{{x}}\");\n}}",
                                        para_idx,
                                        section_idx * 100 + para_idx
                                    ))],
                                )],
                            )
                        } else {
                            // Return an empty span as placeholder
                            DomNode::element("span", HashMap::new(), vec![])
                        },
                    ]
                })
                .filter(|node| !node.children.is_empty() || node.tag != "span")
                .collect();

            let mut children = vec![heading];
            children.extend(paragraphs);

            DomNode::element("section", section_attrs, children)
        })
        .collect();

    let main = with_class(
        "main",
        "content",
        vec![DomNode::element("article", HashMap::new(), sections)],
    );

    // Footer
    let footer = with_class(
        "footer",
        "site-footer",
        vec![DomNode::element(
            "p",
            HashMap::new(),
            vec![DomNode::text(
                "© 2024 Example Corp. All rights reserved.",
            )],
        )],
    );

    // Full page
    attrs.insert("class".to_string(), "page".to_string());
    DomNode::element("div", attrs, vec![header, sidebar, main, footer])
}

// ============================================================================
// Hash computation benchmarks
// ============================================================================

#[divan::bench]
fn hash_sequential_small(bencher: Bencher) {
    let page = generate_large_page(5, 5);
    bencher.bench(|| {
        let mut page = page.clone();
        page.compute_hashes_recursive();
        black_box(page.subtree_hash)
    });
}

#[divan::bench]
fn hash_parallel_small(bencher: Bencher) {
    let page = generate_large_page(5, 5);
    bencher.bench(|| {
        let mut page = page.clone();
        page.compute_hashes_parallel();
        black_box(page.subtree_hash)
    });
}

#[divan::bench]
fn hash_sequential_medium(bencher: Bencher) {
    let page = generate_large_page(20, 10);
    bencher.bench(|| {
        let mut page = page.clone();
        page.compute_hashes_recursive();
        black_box(page.subtree_hash)
    });
}

#[divan::bench]
fn hash_parallel_medium(bencher: Bencher) {
    let page = generate_large_page(20, 10);
    bencher.bench(|| {
        let mut page = page.clone();
        page.compute_hashes_parallel();
        black_box(page.subtree_hash)
    });
}

#[divan::bench]
fn hash_sequential_large(bencher: Bencher) {
    let page = generate_large_page(50, 20);
    bencher.bench(|| {
        let mut page = page.clone();
        page.compute_hashes_recursive();
        black_box(page.subtree_hash)
    });
}

#[divan::bench]
fn hash_parallel_large(bencher: Bencher) {
    let page = generate_large_page(50, 20);
    bencher.bench(|| {
        let mut page = page.clone();
        page.compute_hashes_parallel();
        black_box(page.subtree_hash)
    });
}

// ============================================================================
// Diff benchmarks
// ============================================================================

#[divan::bench]
fn diff_identical_medium(bencher: Bencher) {
    let mut old = generate_large_page(20, 10);
    let mut new = generate_large_page(20, 10);
    old.compute_hashes_parallel();
    new.compute_hashes_parallel();

    bencher.bench(|| black_box(diff(&old, &new)));
}

#[divan::bench]
fn diff_one_change_medium(bencher: Bencher) {
    let mut old = generate_large_page(20, 10);
    let mut new = generate_large_page(20, 10);

    // Modify one paragraph
    if let Some(main) = new.children.get_mut(2)
        && let Some(article) = main.children.get_mut(0)
            && let Some(section) = article.children.get_mut(10)
                && let Some(para) = section.children.get_mut(3)
                    && let Some(text_node) = para.children.get_mut(0) {
                        text_node.text = "THIS TEXT WAS CHANGED!".to_string();
                    }

    old.compute_hashes_parallel();
    new.compute_hashes_parallel();

    bencher.bench(|| black_box(diff(&old, &new)));
}

#[divan::bench]
fn diff_one_change_large(bencher: Bencher) {
    let mut old = generate_large_page(50, 20);
    let mut new = generate_large_page(50, 20);

    // Modify one paragraph deep in the tree
    if let Some(main) = new.children.get_mut(2)
        && let Some(article) = main.children.get_mut(0)
            && let Some(section) = article.children.get_mut(25)
                && let Some(para) = section.children.get_mut(5)
                    && let Some(text_node) = para.children.get_mut(0) {
                        text_node.text = "THIS TEXT WAS CHANGED!".to_string();
                    }

    old.compute_hashes_parallel();
    new.compute_hashes_parallel();

    bencher.bench(|| black_box(diff(&old, &new)));
}

#[divan::bench]
fn diff_many_changes_medium(bencher: Bencher) {
    let mut old = generate_large_page(20, 10);
    let mut new = generate_large_page(20, 10);

    // Modify several paragraphs across different sections
    for section_idx in [2, 5, 10, 15, 18] {
        if let Some(main) = new.children.get_mut(2)
            && let Some(article) = main.children.get_mut(0)
                && let Some(section) = article.children.get_mut(section_idx)
                    && let Some(para) = section.children.get_mut(1)
                        && let Some(text_node) = para.children.get_mut(0) {
                            text_node.text = format!("CHANGED section {}", section_idx);
                        }
    }

    old.compute_hashes_parallel();
    new.compute_hashes_parallel();

    bencher.bench(|| black_box(diff(&old, &new)));
}
