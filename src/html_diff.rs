//! HTML DOM diffing with server-side patch generation
//!
//! Strategy:
//! 1. Parse both old and new HTML into tree structures
//! 2. Hash subtrees to quickly identify unchanged regions
//! 3. For changed subtrees, use tree-edit-distance algorithm
//! 4. Generate minimal patch operations
//! 5. Serialize patches with postcard for WASM client
//!
//! The client (Rust/WASM) applies patches directly to the DOM.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A node in our simplified DOM tree
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomNode {
    /// Element tag name (e.g., "div", "p") or "#text" for text nodes
    pub tag: String,
    /// Attributes (empty for text nodes)
    pub attrs: HashMap<String, String>,
    /// Text content (for text nodes) or empty
    pub text: String,
    /// Child nodes
    pub children: Vec<DomNode>,
    /// Precomputed hash of this subtree (for fast comparison)
    pub subtree_hash: u64,
}

/// A path to a node in the DOM tree
/// e.g., [0, 2, 1] means: root's child 0, then child 2, then child 1
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodePath(pub Vec<usize>);

/// Operations to transform old DOM into new DOM
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Patch {
    /// Replace node at path with new HTML
    Replace { path: NodePath, html: String },

    /// Insert HTML before the node at path
    InsertBefore { path: NodePath, html: String },

    /// Insert HTML after the node at path
    InsertAfter { path: NodePath, html: String },

    /// Append HTML as last child of node at path
    AppendChild { path: NodePath, html: String },

    /// Remove the node at path
    Remove { path: NodePath },

    /// Update text content of node at path
    SetText { path: NodePath, text: String },

    /// Set attribute on node at path
    SetAttribute {
        path: NodePath,
        name: String,
        value: String,
    },

    /// Remove attribute from node at path
    RemoveAttribute { path: NodePath, name: String },
}

/// Result of diffing two DOM trees
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    /// Patches to apply (in order)
    pub patches: Vec<Patch>,
    /// Stats for debugging
    pub nodes_compared: usize,
    pub nodes_skipped: usize,
}

/// Serialize patches to bytes for sending over WebSocket
pub fn serialize_patches(patches: &[Patch]) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(patches)
}

/// Deserialize patches from bytes (for WASM client)
pub fn deserialize_patches(data: &[u8]) -> Result<Vec<Patch>, postcard::Error> {
    postcard::from_bytes(data)
}

impl DomNode {
    /// Create a new element node
    pub fn element(tag: impl Into<String>, attrs: HashMap<String, String>, children: Vec<DomNode>) -> Self {
        let mut node = Self {
            tag: tag.into(),
            attrs,
            text: String::new(),
            children,
            subtree_hash: 0,
        };
        node.compute_hash();
        node
    }

    /// Create a new text node
    pub fn text(content: impl Into<String>) -> Self {
        let text = content.into();
        let mut node = Self {
            tag: "#text".to_string(),
            attrs: HashMap::new(),
            text,
            children: Vec::new(),
            subtree_hash: 0,
        };
        node.compute_hash();
        node
    }

    /// Compute hash of this subtree (call after building the tree)
    pub fn compute_hash(&mut self) {
        use std::hash::{Hash, Hasher};
        use std::collections::hash_map::DefaultHasher;

        let mut hasher = DefaultHasher::new();

        // Hash tag name
        self.tag.hash(&mut hasher);

        // Hash text content
        self.text.hash(&mut hasher);

        // Hash attributes (sorted for determinism)
        let mut attrs: Vec<_> = self.attrs.iter().collect();
        attrs.sort_by_key(|(k, _)| *k);
        for (k, v) in attrs {
            k.hash(&mut hasher);
            v.hash(&mut hasher);
        }

        // Hash children's hashes
        for child in &self.children {
            child.subtree_hash.hash(&mut hasher);
        }

        self.subtree_hash = hasher.finish();
    }

    /// Recursively compute hashes for all nodes (bottom-up) - sequential
    pub fn compute_hashes_recursive(&mut self) {
        for child in &mut self.children {
            child.compute_hashes_recursive();
        }
        self.compute_hash();
    }

    /// Recursively compute hashes for all nodes (bottom-up) - parallel
    pub fn compute_hashes_parallel(&mut self) {
        use rayon::prelude::*;

        // Process children in parallel (they're independent)
        self.children
            .par_iter_mut()
            .for_each(|child| child.compute_hashes_parallel());

        // Then compute our own hash (depends on children being done)
        self.compute_hash();
    }

    /// Check if two nodes have identical subtrees (O(1) via hash)
    pub fn subtree_equal(&self, other: &DomNode) -> bool {
        self.subtree_hash == other.subtree_hash
    }

    /// Count total nodes in this subtree
    pub fn node_count(&self) -> usize {
        1 + self.children.iter().map(|c| c.node_count()).sum::<usize>()
    }
}

/// Diff two DOM trees and produce patches
pub fn diff(old: &DomNode, new: &DomNode) -> DiffResult {
    let mut patches = Vec::new();
    let mut stats = DiffStats::default();

    diff_recursive(old, new, NodePath(vec![]), &mut patches, &mut stats);

    DiffResult {
        patches,
        nodes_compared: stats.compared,
        nodes_skipped: stats.skipped,
    }
}

#[derive(Default)]
struct DiffStats {
    compared: usize,
    skipped: usize,
}

fn diff_recursive(
    old: &DomNode,
    new: &DomNode,
    path: NodePath,
    patches: &mut Vec<Patch>,
    stats: &mut DiffStats,
) {
    // Fast path: if subtrees are identical, skip entirely
    if old.subtree_equal(new) {
        stats.skipped += count_nodes(old);
        return;
    }

    stats.compared += 1;

    // Different tag? Replace entirely
    if old.tag != new.tag {
        patches.push(Patch::Replace {
            path,
            html: node_to_html(new),
        });
        return;
    }

    // Same tag - check for attribute changes
    diff_attributes(old, new, &path, patches);

    // Text node? Check text content
    if old.tag == "#text" {
        if old.text != new.text {
            patches.push(Patch::SetText {
                path,
                text: new.text.clone(),
            });
        }
        return;
    }

    // Diff children
    diff_children(&old.children, &new.children, path, patches, stats);
}

fn diff_attributes(old: &DomNode, new: &DomNode, path: &NodePath, patches: &mut Vec<Patch>) {
    // Find changed/added attributes
    for (name, new_value) in &new.attrs {
        match old.attrs.get(name) {
            Some(old_value) if old_value == new_value => {}
            _ => {
                patches.push(Patch::SetAttribute {
                    path: path.clone(),
                    name: name.clone(),
                    value: new_value.clone(),
                });
            }
        }
    }

    // Find removed attributes
    for name in old.attrs.keys() {
        if !new.attrs.contains_key(name) {
            patches.push(Patch::RemoveAttribute {
                path: path.clone(),
                name: name.clone(),
            });
        }
    }
}

fn diff_children(
    old_children: &[DomNode],
    new_children: &[DomNode],
    parent_path: NodePath,
    patches: &mut Vec<Patch>,
    stats: &mut DiffStats,
) {
    // Simple strategy for now: match by position
    // TODO: Use tree-edit-distance for smarter matching

    let max_len = old_children.len().max(new_children.len());

    for i in 0..max_len {
        let child_path = NodePath({
            let mut p = parent_path.0.clone();
            p.push(i);
            p
        });

        match (old_children.get(i), new_children.get(i)) {
            (Some(old_child), Some(new_child)) => {
                // Both exist - recurse
                diff_recursive(old_child, new_child, child_path, patches, stats);
            }
            (Some(_), None) => {
                // Old exists, new doesn't - remove
                patches.push(Patch::Remove { path: child_path });
            }
            (None, Some(new_child)) => {
                // New exists, old doesn't - append
                patches.push(Patch::AppendChild {
                    path: parent_path.clone(),
                    html: node_to_html(new_child),
                });
            }
            (None, None) => unreachable!(),
        }
    }
}

fn count_nodes(node: &DomNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

fn node_to_html(node: &DomNode) -> String {
    if node.tag == "#text" {
        // TODO: proper HTML escaping
        return node.text.clone();
    }

    let mut html = String::new();
    html.push('<');
    html.push_str(&node.tag);

    for (name, value) in &node.attrs {
        html.push(' ');
        html.push_str(name);
        html.push_str("=\"");
        // TODO: proper attribute escaping
        html.push_str(value);
        html.push('"');
    }

    html.push('>');

    for child in &node.children {
        html.push_str(&node_to_html(child));
    }

    html.push_str("</");
    html.push_str(&node.tag);
    html.push('>');

    html
}

/// Parse HTML string into DomNode tree
/// Returns the root element (typically <html> or a wrapper containing the parsed content)
pub fn parse_html(html: &str) -> Option<DomNode> {
    use html5ever::tendril::TendrilSink;
    use html5ever::{parse_document, ParseOpts};
    use markup5ever_rcdom::RcDom;

    let dom = parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .ok()?;

    // The document node contains the parsed tree
    // Find the first meaningful element (usually <html>)
    convert_rcdom_node(&dom.document)
}

/// Parse an HTML fragment (not a full document) into a DomNode tree
/// Useful for parsing partial HTML like "<p>hello</p><p>world</p>"
pub fn parse_html_fragment(html: &str) -> Option<DomNode> {
    use html5ever::tendril::TendrilSink;
    use html5ever::{parse_fragment, ns, ParseOpts, QualName};
    use markup5ever_rcdom::RcDom;

    // Parse as children of a <body> element
    let context = QualName::new(None, ns!(html), "body".into());
    // 5th arg: form_element_is_associated_with_document (false for fragments)
    let dom = parse_fragment(RcDom::default(), ParseOpts::default(), context, vec![], false)
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .ok()?;

    // Return a wrapper div containing all parsed nodes
    let children: Vec<DomNode> = dom
        .document
        .children
        .borrow()
        .iter()
        .filter_map(convert_rcdom_node)
        .collect();

    if children.len() == 1 {
        children.into_iter().next()
    } else {
        Some(DomNode::element("div", HashMap::new(), children))
    }
}

/// Convert an rcdom Handle to our DomNode structure
fn convert_rcdom_node(handle: &markup5ever_rcdom::Handle) -> Option<DomNode> {
    use markup5ever_rcdom::NodeData;

    match &handle.data {
        NodeData::Document => {
            // Document node - return first element child (usually <html>)
            let children = handle.children.borrow();
            for child in children.iter() {
                if let Some(node) = convert_rcdom_node(child)
                    && node.tag != "#text" {
                        return Some(node);
                    }
            }
            None
        }
        NodeData::Element { name, attrs, .. } => {
            let tag_name = name.local.to_string();

            // Collect attributes
            let mut attr_map = HashMap::new();
            for attr in attrs.borrow().iter() {
                attr_map.insert(attr.name.local.to_string(), attr.value.to_string());
            }

            // Convert children
            let children: Vec<DomNode> = handle
                .children
                .borrow()
                .iter()
                .filter_map(convert_rcdom_node)
                .collect();

            Some(DomNode::element(tag_name, attr_map, children))
        }
        NodeData::Text { contents } => {
            let text = contents.borrow().to_string();
            // Skip whitespace-only text nodes
            if text.trim().is_empty() {
                return None;
            }
            Some(DomNode::text(text))
        }
        NodeData::Comment { .. } => None,
        NodeData::Doctype { .. } => None,
        NodeData::ProcessingInstruction { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> DomNode {
        DomNode::element("p", HashMap::new(), vec![DomNode::text(text)])
    }

    fn div(children: Vec<DomNode>) -> DomNode {
        DomNode::element("div", HashMap::new(), children)
    }

    #[test]
    fn test_identical_trees_no_patches() {
        let old = div(vec![p("hello"), p("world")]);
        let new = div(vec![p("hello"), p("world")]);

        let result = diff(&old, &new);
        assert!(result.patches.is_empty());
        assert_eq!(result.nodes_skipped, 5); // div + 2*(p + text)
    }

    #[test]
    fn test_text_change() {
        let old = div(vec![p("hello")]);
        let new = div(vec![p("goodbye")]);

        let result = diff(&old, &new);
        assert_eq!(result.patches.len(), 1);
        assert!(matches!(
            &result.patches[0],
            Patch::SetText { path, text } if path.0 == vec![0, 0] && text == "goodbye"
        ));
    }

    #[test]
    fn test_add_child() {
        let old = div(vec![p("one")]);
        let new = div(vec![p("one"), p("two")]);

        let result = diff(&old, &new);
        assert_eq!(result.patches.len(), 1);
        assert!(matches!(&result.patches[0], Patch::AppendChild { .. }));
    }

    #[test]
    fn test_remove_child() {
        let old = div(vec![p("one"), p("two")]);
        let new = div(vec![p("one")]);

        let result = diff(&old, &new);
        assert_eq!(result.patches.len(), 1);
        assert!(matches!(
            &result.patches[0],
            Patch::Remove { path } if path.0 == vec![1]
        ));
    }

    #[test]
    fn test_replace_different_tag() {
        let old = div(vec![DomNode::element("p", HashMap::new(), vec![DomNode::text("hi")])]);
        let new = div(vec![DomNode::element("span", HashMap::new(), vec![DomNode::text("hi")])]);

        let result = diff(&old, &new);
        assert_eq!(result.patches.len(), 1);
        assert!(matches!(&result.patches[0], Patch::Replace { .. }));
    }

    #[test]
    fn test_patch_serialization_roundtrip() {
        let patches = vec![
            Patch::SetText {
                path: NodePath(vec![0, 1, 2]),
                text: "hello world".to_string(),
            },
            Patch::SetAttribute {
                path: NodePath(vec![0]),
                name: "class".to_string(),
                value: "active".to_string(),
            },
            Patch::Remove {
                path: NodePath(vec![1, 0]),
            },
        ];

        let serialized = serialize_patches(&patches).unwrap();
        let deserialized = deserialize_patches(&serialized).unwrap();

        assert_eq!(patches, deserialized);
        // postcard is compact - these 3 patches should be < 100 bytes
        assert!(serialized.len() < 100, "Serialized size: {}", serialized.len());
    }

    /// Generate a realistic large page for benchmarking
    /// Simulates a documentation site with:
    /// - Header with nav
    /// - Sidebar with table of contents
    /// - Main content with many sections, each with paragraphs and code blocks
    /// - Footer
    fn generate_large_page(num_sections: usize, paragraphs_per_section: usize) -> DomNode {
        let mut attrs = HashMap::new();

        // Helper to make an element with class
        let with_class = |tag: &str, class: &str, children: Vec<DomNode>| {
            let mut attrs = HashMap::new();
            attrs.insert("class".to_string(), class.to_string());
            DomNode::element(tag, attrs, children)
        };

        // Header
        let header = with_class("header", "site-header", vec![
            with_class("nav", "main-nav", vec![
                DomNode::element("a", {
                    let mut a = HashMap::new();
                    a.insert("href".to_string(), "/".to_string());
                    a
                }, vec![DomNode::text("Home")]),
                DomNode::element("a", {
                    let mut a = HashMap::new();
                    a.insert("href".to_string(), "/docs".to_string());
                    a
                }, vec![DomNode::text("Docs")]),
                DomNode::element("a", {
                    let mut a = HashMap::new();
                    a.insert("href".to_string(), "/blog".to_string());
                    a
                }, vec![DomNode::text("Blog")]),
            ]),
        ]);

        // Sidebar TOC
        let toc_items: Vec<DomNode> = (0..num_sections)
            .map(|i| {
                DomNode::element("li", HashMap::new(), vec![
                    DomNode::element("a", {
                        let mut a = HashMap::new();
                        a.insert("href".to_string(), format!("#section-{}", i));
                        a
                    }, vec![DomNode::text(format!("Section {}", i))]),
                ])
            })
            .collect();
        let sidebar = with_class("aside", "sidebar", vec![
            with_class("nav", "toc", vec![
                DomNode::element("ul", HashMap::new(), toc_items),
            ]),
        ]);

        // Main content with sections
        let sections: Vec<DomNode> = (0..num_sections)
            .map(|section_idx| {
                let mut section_attrs = HashMap::new();
                section_attrs.insert("id".to_string(), format!("section-{}", section_idx));

                let heading = DomNode::element("h2", HashMap::new(), vec![
                    DomNode::text(format!("Section {} - Important Topic", section_idx)),
                ]);

                let paragraphs: Vec<DomNode> = (0..paragraphs_per_section)
                    .flat_map(|para_idx| {
                        vec![
                            DomNode::element("p", HashMap::new(), vec![
                                DomNode::text(format!(
                                    "This is paragraph {} of section {}. It contains some text \
                                     that simulates real documentation content with enough words \
                                     to be realistic. Lorem ipsum dolor sit amet.",
                                    para_idx, section_idx
                                )),
                            ]),
                            // Add a code block every few paragraphs
                            if para_idx % 3 == 2 {
                                with_class("pre", "code-block", vec![
                                    DomNode::element("code", HashMap::new(), vec![
                                        DomNode::text(format!(
                                            "fn example_{}() {{\n    let x = {};\n    println!(\"{{x}}\");\n}}",
                                            para_idx, section_idx * 100 + para_idx
                                        )),
                                    ]),
                                ])
                            } else {
                                // Return an empty span as placeholder (will be filtered if needed)
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

        let main = with_class("main", "content", vec![
            DomNode::element("article", HashMap::new(), sections),
        ]);

        // Footer
        let footer = with_class("footer", "site-footer", vec![
            DomNode::element("p", HashMap::new(), vec![
                DomNode::text("© 2024 Example Corp. All rights reserved."),
            ]),
        ]);

        // Full page
        attrs.insert("class".to_string(), "page".to_string());
        DomNode::element("div", attrs, vec![header, sidebar, main, footer])
    }

    #[test]
    fn test_large_page_node_count() {
        let page = generate_large_page(20, 10);
        let count = page.node_count();
        println!("Large page node count: {}", count);
        assert!(count > 500, "Expected large page to have >500 nodes, got {}", count);
    }

    #[test]
    fn test_hash_sequential_vs_parallel() {
        use std::time::Instant;

        // Generate two copies of a large page
        let mut page_seq = generate_large_page(50, 20);
        let mut page_par = generate_large_page(50, 20);

        let node_count = page_seq.node_count();
        println!("Benchmarking with {} nodes", node_count);

        // Sequential
        let start = Instant::now();
        page_seq.compute_hashes_recursive();
        let seq_time = start.elapsed();

        // Parallel
        let start = Instant::now();
        page_par.compute_hashes_parallel();
        let par_time = start.elapsed();

        println!("Sequential: {:?}", seq_time);
        println!("Parallel:   {:?}", par_time);
        println!("Speedup:    {:.2}x", seq_time.as_secs_f64() / par_time.as_secs_f64());

        // Verify they produce the same hash
        assert_eq!(page_seq.subtree_hash, page_par.subtree_hash);
    }

    #[test]
    fn test_diff_large_pages_one_change() {
        use std::time::Instant;

        let mut old = generate_large_page(30, 15);
        let mut new = generate_large_page(30, 15);

        // Modify one paragraph deep in the tree
        if let Some(main) = new.children.get_mut(2) {  // main
            if let Some(article) = main.children.get_mut(0) {  // article
                if let Some(section) = article.children.get_mut(15) {  // section 15
                    if let Some(para) = section.children.get_mut(3) {  // 3rd element
                        if let Some(text_node) = para.children.get_mut(0) {
                            text_node.text = "THIS TEXT WAS CHANGED BY THE TEST!".to_string();
                        }
                    }
                }
            }
        }

        // Compute hashes
        old.compute_hashes_parallel();
        new.compute_hashes_parallel();

        let node_count = old.node_count();
        println!("Diffing {} nodes with 1 change", node_count);

        let start = Instant::now();
        let result = diff(&old, &new);
        let diff_time = start.elapsed();

        println!("Diff time: {:?}", diff_time);
        println!("Patches: {}", result.patches.len());
        println!("Nodes compared: {}", result.nodes_compared);
        println!("Nodes skipped: {}", result.nodes_skipped);

        // Should have exactly 1 patch (the text change)
        assert_eq!(result.patches.len(), 1);

        // Should skip most of the tree
        assert!(result.nodes_skipped > node_count / 2,
            "Expected to skip most nodes, but only skipped {}/{}",
            result.nodes_skipped, node_count);
    }
}
