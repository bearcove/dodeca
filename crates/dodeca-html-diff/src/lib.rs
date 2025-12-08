//! HTML DOM diffing plugin for dodeca
//!
//! Strategy:
//! 1. Parse both old and new HTML into tree structures
//! 2. Hash subtrees to quickly identify unchanged regions
//! 3. For changed subtrees, use tree-edit-distance algorithm
//! 4. Generate minimal patch operations
//!
//! The client (Rust/WASM) applies patches directly to the DOM.

use std::collections::HashMap;

use facet::Facet;
use plugcard::{plugcard, PlugResult};

// Re-export patch types from protocol
pub use dodeca_protocol::{NodePath, Patch};

plugcard::export_plugin!();

/// Input for the diff_html function
#[derive(Facet)]
pub struct DiffInput {
    pub old_html: String,
    pub new_html: String,
}

/// Result of diffing two DOM trees
#[derive(Facet)]
pub struct DiffResult {
    /// Patches to apply (in order)
    pub patches: Vec<Patch>,
    /// Stats for debugging
    pub nodes_compared: usize,
    pub nodes_skipped: usize,
}

/// Diff two HTML documents and produce patches to transform old into new
#[plugcard]
pub fn diff_html(input: DiffInput) -> PlugResult<DiffResult> {
    let old_dom = match parse_html(&input.old_html) {
        Some(dom) => dom,
        None => return PlugResult::Err("Failed to parse old HTML".to_string()),
    };

    let new_dom = match parse_html(&input.new_html) {
        Some(dom) => dom,
        None => return PlugResult::Err("Failed to parse new HTML".to_string()),
    };

    let result = diff(&old_dom, &new_dom);

    PlugResult::Ok(DiffResult {
        patches: result.patches,
        nodes_compared: result.nodes_compared,
        nodes_skipped: result.nodes_skipped,
    })
}

// ============================================================================
// Internal DOM representation
// ============================================================================

/// A node in our simplified DOM tree
#[derive(Debug, Clone, PartialEq, Eq)]
struct DomNode {
    /// Element tag name (e.g., "div", "p") or "#text" for text nodes
    tag: String,
    /// Attributes (empty for text nodes)
    attrs: HashMap<String, String>,
    /// Text content (for text nodes) or empty
    text: String,
    /// Child nodes
    children: Vec<DomNode>,
    /// Precomputed hash of this subtree (for fast comparison)
    subtree_hash: u64,
}

/// Internal result of diffing two DOM trees
struct InternalDiffResult {
    patches: Vec<Patch>,
    nodes_compared: usize,
    nodes_skipped: usize,
}

impl DomNode {
    /// Create a new element node
    fn element(tag: impl Into<String>, attrs: HashMap<String, String>, children: Vec<DomNode>) -> Self {
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
    fn text(content: impl Into<String>) -> Self {
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
    fn compute_hash(&mut self) {
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

    /// Recursively compute hashes for all nodes (bottom-up) - parallel
    fn compute_hashes_parallel(&mut self) {
        use rayon::prelude::*;

        // Process children in parallel (they're independent)
        self.children
            .par_iter_mut()
            .for_each(|child| child.compute_hashes_parallel());

        // Then compute our own hash (depends on children being done)
        self.compute_hash();
    }

    /// Check if two nodes have identical subtrees (O(1) via hash)
    fn subtree_equal(&self, other: &DomNode) -> bool {
        self.subtree_hash == other.subtree_hash
    }
}

/// Diff two DOM trees and produce patches
fn diff(old: &DomNode, new: &DomNode) -> InternalDiffResult {
    let mut patches = Vec::new();
    let mut stats = DiffStats::default();

    diff_recursive(old, new, NodePath(vec![]), &mut patches, &mut stats);

    InternalDiffResult {
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

// ============================================================================
// HTML Parsing
// ============================================================================

/// Parse HTML string into DomNode tree
/// Returns the root element (typically <html> or a wrapper containing the parsed content)
fn parse_html(html: &str) -> Option<DomNode> {
    use html5ever::tendril::TendrilSink;
    use html5ever::{parse_document, ParseOpts};
    use markup5ever_rcdom::RcDom;

    let dom = parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .ok()?;

    // The document node contains the parsed tree
    // Find the first meaningful element (usually <html>)
    let mut result = convert_rcdom_node(&dom.document)?;
    result.compute_hashes_parallel();
    Some(result)
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

#[unsafe(no_mangle)]
pub extern "C" fn debug_struct_size() -> usize {
    std::mem::size_of::<plugcard::MethodSignature>()
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
    fn test_diff_html_via_plugin() {
        let input = DiffInput {
            old_html: "<html><body><p>hello</p></body></html>".to_string(),
            new_html: "<html><body><p>goodbye</p></body></html>".to_string(),
        };

        let result = diff_html(input);
        let PlugResult::Ok(diff_result) = result else {
            panic!("Expected Ok result");
        };

        assert_eq!(diff_result.patches.len(), 1);
    }
}
