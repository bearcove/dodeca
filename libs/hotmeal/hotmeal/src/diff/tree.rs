//! Direct cinereus tree implementation for HTML DOM.
//!
//! This module implements cinereus traits directly on hotmeal's DOM types,
//! bypassing the facet reflection layer for simpler, more efficient diffing.

use crate::debug;
#[allow(unused_imports)]
use crate::trace;
use cinereus::{
    EditOp, Matching, MatchingConfig, NodeData, NodeHash, PropValue, Properties,
    PropertyInFinalState, Tree, TreeTypes,
    indextree::{self, NodeId},
};
use indexmap::IndexMap;
use rapidhash::RapidHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use super::apply::{Content, Element};
use super::{InsertContent, NodePath, NodeRef, Patch, PropChange};
use crate::arena_dom;

/// Node kind in the HTML tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HtmlNodeKind {
    /// An element node with a tag name
    Element(String),
    /// A text node
    Text,
    /// A comment node
    Comment,
}

impl std::fmt::Display for HtmlNodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HtmlNodeKind::Element(tag) => write!(f, "<{}>", tag),
            HtmlNodeKind::Text => write!(f, "#text"),
            HtmlNodeKind::Comment => write!(f, "#comment"),
        }
    }
}

/// HTML element properties (attributes + text content).
#[derive(Debug, Clone, Default)]
pub struct HtmlProps {
    /// Element attributes (preserves insertion order)
    pub attrs: IndexMap<String, String>,
    /// Text content (for text nodes)
    pub text: Option<String>,
}

impl Properties for HtmlProps {
    type Key = String;
    type Value = String;

    fn similarity(&self, other: &Self) -> f64 {
        // Text nodes: compare text content
        if let (Some(t1), Some(t2)) = (&self.text, &other.text) {
            return if t1 == t2 { 1.0 } else { 0.0 };
        }

        // Element nodes: compare attributes using Dice coefficient
        if self.attrs.is_empty() && other.attrs.is_empty() {
            return 1.0;
        }

        let self_keys: std::collections::HashSet<_> = self.attrs.keys().collect();
        let other_keys: std::collections::HashSet<_> = other.attrs.keys().collect();

        let intersection = self_keys.intersection(&other_keys).count();
        let union = self_keys.len() + other_keys.len();

        if union == 0 {
            1.0
        } else {
            (2 * intersection) as f64 / union as f64
        }
    }

    fn diff(&self, other: &Self) -> Vec<PropertyInFinalState<Self::Key, Self::Value>> {
        let mut result = Vec::new();

        // Diff text content - always include if present in final state
        if let Some(text) = &other.text {
            result.push(PropertyInFinalState {
                key: "_text".to_string(),
                value: if self.text.as_ref() == Some(text) {
                    PropValue::Same
                } else {
                    PropValue::Different(text.clone())
                },
            });
        }

        // Include all attributes from the final state in order
        for (key, new_val) in &other.attrs {
            let old_val = self.attrs.get(key);
            result.push(PropertyInFinalState {
                key: key.clone(),
                value: if old_val == Some(new_val) {
                    PropValue::Same
                } else {
                    PropValue::Different(new_val.clone())
                },
            });
        }

        result
    }

    fn is_empty(&self) -> bool {
        self.attrs.is_empty() && self.text.is_none()
    }
}

/// Tree types marker for HTML DOM.
pub struct HtmlTreeTypes;

impl TreeTypes for HtmlTreeTypes {
    type Kind = HtmlNodeKind;
    type Label = NodePath; // Store path for each node
    type Props = HtmlProps;
}

/// Build a cinereus tree from an Element.
pub fn build_tree(element: &Element) -> Tree<HtmlTreeTypes> {
    let root_data = make_element_node_data(element, NodePath(vec![]));
    let mut tree = Tree::new(root_data);

    // Add children recursively
    let root = tree.root;
    add_children(&mut tree, root, &element.children, NodePath(vec![]));

    // Recompute hashes bottom-up
    recompute_hashes(&mut tree);

    tree
}

/// Build a cinereus tree from an arena_dom::Document (body content only).
pub fn build_tree_from_arena(doc: &arena_dom::Document) -> Tree<HtmlTreeTypes> {
    // Find body element
    let body_id = doc.body().expect("document must have body");
    let body_node = doc.get(body_id);

    // Create root as body element
    let body_tag = if let arena_dom::NodeKind::Element(elem) = &body_node.kind {
        elem.tag.as_ref()
    } else {
        panic!("body must be an element");
    };

    let root_data = NodeData {
        hash: NodeHash(0),
        kind: HtmlNodeKind::Element(body_tag.to_string()),
        label: Some(NodePath(vec![])),
        properties: HtmlProps {
            attrs: IndexMap::new(),
            text: None,
        },
    };

    let mut tree = Tree::new(root_data);
    let tree_root = tree.root;

    // Add children from body
    add_arena_children(&mut tree, tree_root, doc, body_id, NodePath(vec![]));

    // Recompute hashes bottom-up
    recompute_hashes(&mut tree);

    tree
}

fn add_arena_children(
    tree: &mut Tree<HtmlTreeTypes>,
    parent: indextree::NodeId,
    doc: &arena_dom::Document,
    arena_parent: indextree::NodeId,
    parent_path: NodePath,
) {
    let children: Vec<_> = arena_parent.children(&doc.arena).collect();

    for (i, child_id) in children.into_iter().enumerate() {
        let mut child_path = parent_path.0.clone();
        child_path.push(i);
        let child_path = NodePath(child_path);

        let child_node = doc.get(child_id);
        match &child_node.kind {
            arena_dom::NodeKind::Element(elem) => {
                let kind = HtmlNodeKind::Element(elem.tag.as_ref().to_string());
                let props = HtmlProps {
                    attrs: elem
                        .attrs
                        .iter()
                        .map(|(k, v)| (k.clone(), v.as_ref().to_string()))
                        .collect(),
                    text: None,
                };
                let data = NodeData {
                    hash: NodeHash(0),
                    kind,
                    label: Some(child_path.clone()),
                    properties: props,
                };
                let node_id = tree.add_child(parent, data);
                add_arena_children(tree, node_id, doc, child_id, child_path);
            }
            arena_dom::NodeKind::Text(text) => {
                let kind = HtmlNodeKind::Text;
                let props = HtmlProps {
                    attrs: IndexMap::new(),
                    text: Some(text.as_ref().to_string()),
                };
                let data = NodeData {
                    hash: NodeHash(0),
                    kind,
                    label: Some(child_path),
                    properties: props,
                };
                tree.add_child(parent, data);
            }
            arena_dom::NodeKind::Comment(text) => {
                let kind = HtmlNodeKind::Comment;
                let props = HtmlProps {
                    attrs: IndexMap::new(),
                    text: Some(text.as_ref().to_string()),
                };
                let data = NodeData {
                    hash: NodeHash(0),
                    kind,
                    label: Some(child_path),
                    properties: props,
                };
                tree.add_child(parent, data);
            }
            arena_dom::NodeKind::Document => {
                // Skip document nodes
            }
        }
    }
}

fn add_children(
    tree: &mut Tree<HtmlTreeTypes>,
    parent: NodeId,
    children: &[Content],
    parent_path: NodePath,
) {
    for (i, child) in children.iter().enumerate() {
        let mut child_path = parent_path.0.clone();
        child_path.push(i);
        let child_path = NodePath(child_path);

        match child {
            Content::Element(elem) => {
                let data = make_element_node_data(elem, child_path.clone());
                let node_id = tree.add_child(parent, data);
                add_children(tree, node_id, &elem.children, child_path);
            }
            Content::Text(text) => {
                let data = make_text_node_data(text, child_path);
                tree.add_child(parent, data);
            }
            Content::Comment(comment) => {
                let data = make_comment_node_data(comment, child_path);
                tree.add_child(parent, data);
            }
        }
    }
}

fn make_element_node_data(elem: &Element, path: NodePath) -> NodeData<HtmlTreeTypes> {
    let kind = HtmlNodeKind::Element(elem.tag.clone());
    let props = HtmlProps {
        attrs: elem.attrs.clone(),
        text: None,
    };
    // Hash will be recomputed later
    NodeData {
        hash: NodeHash(0),
        kind,
        label: Some(path),
        properties: props,
    }
}

fn make_text_node_data(text: &str, path: NodePath) -> NodeData<HtmlTreeTypes> {
    let kind = HtmlNodeKind::Text;
    let props = HtmlProps {
        attrs: IndexMap::new(),
        text: Some(text.to_string()),
    };
    // Hash will be recomputed later
    NodeData {
        hash: NodeHash(0),
        kind,
        label: Some(path),
        properties: props,
    }
}

fn make_comment_node_data(comment: &str, path: NodePath) -> NodeData<HtmlTreeTypes> {
    let kind = HtmlNodeKind::Comment;
    let props = HtmlProps {
        attrs: IndexMap::new(),
        text: Some(comment.to_string()),
    };
    // Hash will be recomputed later
    NodeData {
        hash: NodeHash(0),
        kind,
        label: Some(path),
        properties: props,
    }
}

/// Recompute hashes for all nodes in bottom-up order.
///
/// IMPORTANT: Properties (attributes, text content) are NOT included in the hash.
/// The hash only captures the structural identity: node kind + children structure.
/// Properties are compared separately via the Properties trait after matching.
fn recompute_hashes(tree: &mut Tree<HtmlTreeTypes>) {
    // Process in post-order (children before parents)
    let nodes: Vec<NodeId> = tree.post_order().collect();

    for node_id in nodes {
        let mut hasher = RapidHasher::default();

        // Hash the node's kind only (not properties - those are compared separately)
        let data = tree.get(node_id);
        data.kind.hash(&mut hasher);

        // Hash children's hashes (Merkle-tree style)
        let children: Vec<NodeId> = tree.children(node_id).collect();
        for child in children {
            tree.get(child).hash.0.hash(&mut hasher);
        }

        // Update the hash
        let new_hash = NodeHash(hasher.finish());
        tree.arena
            .get_mut(node_id)
            .expect("node should exist")
            .get_mut()
            .hash = new_hash;
    }
}

/// Compute diff between two elements and return patches.
pub fn diff_elements(old: &Element, new: &Element) -> Result<Vec<Patch>, String> {
    let tree_a = build_tree(old);
    let tree_b = build_tree(new);

    #[cfg(test)]
    {
        trace!(
            "tree_a: root hash={:?}, kind={:?}",
            tree_a.get(tree_a.root).hash,
            tree_a.get(tree_a.root).kind
        );
        trace!(
            "tree_b: root hash={:?}, kind={:?}",
            tree_b.get(tree_b.root).hash,
            tree_b.get(tree_b.root).kind
        );
    }

    let config = MatchingConfig {
        min_height: 0, // Include all nodes including leaves in top-down matching
        ..MatchingConfig::default()
    };

    // Compute initial matching
    let mut matching = cinereus::compute_matching(&tree_a, &tree_b, &config);

    // IMPORTANT: For HTML diffing, always match the roots if they have the same tag.
    // Without this, when roots have different child counts (e.g., body with 1 child
    // vs body with 0 children), they won't match due to different hashes, causing
    // cinereus to generate a Delete for the entire tree_a root, which is invalid.
    let root_a = tree_a.get(tree_a.root);
    let root_b = tree_b.get(tree_b.root);
    if root_a.kind == root_b.kind && !matching.contains_a(tree_a.root) {
        matching.add(tree_a.root, tree_b.root);
    }

    // Generate edit script with the matching (including forced root match)
    let edit_ops = cinereus::generate_edit_script(&tree_a, &tree_b, &matching);
    #[cfg(test)]
    {
        debug!("matching pairs: {}", matching.len());
        for (a, b) in matching.pairs() {
            trace!("  matched: {:?} <-> {:?}", a, b);
        }
        trace!("edit_ops: {:?}", edit_ops);
    }

    debug!(
        ops_count = edit_ops.len(),
        matched_pairs = matching.len(),
        "cinereus diff complete"
    );

    // Convert cinereus ops to patches using shadow tree approach
    convert_ops_with_shadow(edit_ops, &tree_a, &tree_b, &matching)
}

/// Compute diff between two arena_dom::Documents and return patches.
pub fn diff_arena_documents(
    old: &arena_dom::Document,
    new: &arena_dom::Document,
) -> Result<Vec<Patch>, String> {
    let tree_a = build_tree_from_arena(old);
    let tree_b = build_tree_from_arena(new);

    #[cfg(test)]
    {
        trace!(
            "tree_a: root hash={:?}, kind={:?}",
            tree_a.get(tree_a.root).hash,
            tree_a.get(tree_a.root).kind
        );
        trace!(
            "tree_b: root hash={:?}, kind={:?}",
            tree_b.get(tree_b.root).hash,
            tree_b.get(tree_b.root).kind
        );
    }

    let config = MatchingConfig {
        min_height: 0,
        ..MatchingConfig::default()
    };

    let mut matching = cinereus::compute_matching(&tree_a, &tree_b, &config);

    // Force root match if same tag
    let root_a = tree_a.get(tree_a.root);
    let root_b = tree_b.get(tree_b.root);
    if root_a.kind == root_b.kind && !matching.contains_a(tree_a.root) {
        matching.add(tree_a.root, tree_b.root);
    }

    let edit_ops = cinereus::generate_edit_script(&tree_a, &tree_b, &matching);

    #[cfg(test)]
    {
        debug!("matching pairs: {}", matching.len());
        for (a, b) in matching.pairs() {
            trace!("  matched: {:?} <-> {:?}", a, b);
        }
        trace!("edit_ops: {:?}", edit_ops);
    }

    debug!(
        ops_count = edit_ops.len(),
        matched_pairs = matching.len(),
        "arena_dom cinereus diff complete"
    );

    convert_ops_with_shadow(edit_ops, &tree_a, &tree_b, &matching)
}

/// Encapsulates the shadow tree and its detached nodes (slots).
///
/// This prevents bugs where we forget to check if a node is detached.
/// All node reference queries go through this type, which automatically
/// handles both in-tree and detached node cases.
struct ShadowTree {
    arena: indextree::Arena<NodeData<HtmlTreeTypes>>,
    root: NodeId,
    /// Maps NodeId to slot number for nodes that have been detached
    detached_nodes: HashMap<NodeId, u32>,
    next_slot: u32,
}

impl ShadowTree {
    fn new(arena: indextree::Arena<NodeData<HtmlTreeTypes>>, root: NodeId) -> Self {
        Self {
            arena,
            root,
            detached_nodes: HashMap::new(),
            next_slot: 0,
        }
    }

    /// Get a NodeRef for any node - automatically checks if detached.
    /// This is the KEY method that prevents "forgot to check detached" bugs!
    fn get_node_ref(&self, node: NodeId) -> NodeRef {
        // Check if directly detached
        if let Some(&slot) = self.detached_nodes.get(&node) {
            debug!(?node, slot, "get_node_ref: node is directly in slot");
            return NodeRef::Slot(slot, None);
        }

        // Check if ancestor is detached
        if let Some((slot, rel_path)) = self.find_detached_ancestor(node) {
            debug!(
                ?node,
                slot,
                ?rel_path,
                "get_node_ref: node has detached ancestor"
            );
            // Check what's actually in the slot
            if let Some((slot_node, _)) = self.detached_nodes.iter().find(|(_, s)| **s == slot) {
                #[allow(unused_variables)]
                let slot_data = &self.arena[*slot_node].get();
                debug!(?slot_node, kind = ?slot_data.kind, "get_node_ref: slot contains");
            }
            return NodeRef::Slot(slot, rel_path);
        }

        // Node is in tree - compute path
        let path = self.compute_path(node);
        debug!(?node, ?path, "get_node_ref: node is in tree at path");
        NodeRef::Path(NodePath(path))
    }

    /// Compute path for a node in the tree.
    fn compute_path(&self, node: NodeId) -> Vec<usize> {
        let mut path = Vec::new();
        let mut current = node;

        while current != self.root {
            if let Some(parent_id) = self.arena.get(current).and_then(|n| n.parent()) {
                let position = parent_id
                    .children(&self.arena)
                    .position(|c| c == current)
                    .unwrap_or(0);
                path.push(position);
                current = parent_id;
            } else {
                break;
            }
        }

        path.reverse();
        path
    }

    /// Find if any ancestor of this node is detached, returning (slot, relative_path).
    fn find_detached_ancestor(&self, node: NodeId) -> Option<(u32, Option<NodePath>)> {
        let mut current = node;
        let mut traversal: Vec<(NodeId, NodeId)> = Vec::new();

        debug!(
            ?node,
            detached_count = self.detached_nodes.len(),
            "find_detached_ancestor: starting search"
        );

        loop {
            debug!(?current, "find_detached_ancestor: checking node");

            // Check if current node is detached
            if let Some(&slot) = self.detached_nodes.get(&current) {
                debug!(
                    ?current,
                    slot, "find_detached_ancestor: found detached ancestor"
                );

                // Build the relative path from slot root to the original node
                let relative_path = if traversal.is_empty() {
                    None // Node is directly the slot root
                } else {
                    // Compute relative path by position indices
                    let mut path_indices: Vec<usize> = Vec::new();
                    for (child, parent) in traversal.iter().rev() {
                        let pos = parent
                            .children(&self.arena)
                            .position(|c| c == *child)
                            .unwrap_or(0);
                        path_indices.push(pos);
                    }
                    Some(NodePath(path_indices))
                };
                return Some((slot, relative_path));
            }

            // Move to parent
            if let Some(parent_id) = self.arena.get(current).and_then(|n| n.parent()) {
                debug!(
                    ?current,
                    ?parent_id,
                    "find_detached_ancestor: moving to parent"
                );
                traversal.push((current, parent_id));
                current = parent_id;
            } else {
                debug!(
                    ?current,
                    "find_detached_ancestor: no more parents, returning None"
                );
                break;
            }
        }
        None
    }

    /// Detach a node to a slot, returning the slot number.
    fn detach_to_slot(&mut self, node: NodeId) -> u32 {
        let slot = self.next_slot;
        self.next_slot += 1;
        node.detach(&mut self.arena);
        self.detached_nodes.insert(node, slot);
        debug!(?node, slot, "detached node to slot");
        slot
    }

    /// Detach a node with a placeholder to prevent sibling shifts.
    fn detach_with_placeholder(&mut self, node: NodeId) {
        let placeholder = self.arena.new_node(NodeData {
            hash: NodeHash(0),
            kind: HtmlNodeKind::Text,
            label: None,
            properties: HtmlProps::default(),
        });
        node.insert_before(placeholder, &mut self.arena);
        node.detach(&mut self.arena);
    }

    /// Remove a node from detached tracking (when it's removed from a slot).
    fn remove_from_detached(&mut self, node: NodeId) {
        self.detached_nodes.remove(&node);
    }

    /// Pretty-print the shadow tree for debugging.
    #[allow(dead_code)]
    fn debug_print_tree(&self, _title: &str) {
        debug!("=== {} ===", _title);
        self.debug_print_node(self.root, 0);
        if !self.detached_nodes.is_empty() {
            debug!("Detached nodes:");
            for (&node, &_slot) in &self.detached_nodes {
                debug!("  Slot {}: {:?}", _slot, node);
                self.debug_print_node(node, 2);
            }
        }
        debug!("===");
    }

    fn debug_print_node(&self, node: NodeId, depth: usize) {
        let _indent = "  ".repeat(depth);
        let data = &self.arena[node].get();
        let _kind_str = match &data.kind {
            HtmlNodeKind::Element(tag) => format!("<{}>", tag),
            HtmlNodeKind::Text => {
                let text = data.properties.text.as_deref().unwrap_or("");
                format!("#text({:?})", text.chars().take(20).collect::<String>())
            }
            HtmlNodeKind::Comment => {
                let text = data.properties.text.as_deref().unwrap_or("");
                format!("#comment({:?})", text.chars().take(20).collect::<String>())
            }
        };
        debug!("{}{:?} {}", _indent, node, _kind_str);
        for child in node.children(&self.arena) {
            self.debug_print_node(child, depth + 1);
        }
    }

    /// Insert a new node at a position, handling displacement via slots.
    /// Returns the slot number if an occupant was displaced.
    fn insert_at_position(
        &mut self,
        parent: NodeId,
        position: usize,
        new_node: NodeId,
    ) -> Option<u32> {
        let children: Vec<_> = parent.children(&self.arena).collect();

        if position < children.len() {
            let occupant = children[position];
            // Insert new_node before occupant, then detach occupant to slot
            occupant.insert_before(new_node, &mut self.arena);
            let slot = self.detach_to_slot(occupant);
            Some(slot)
        } else {
            // Grow the children array with placeholder text nodes to reach the target position
            while parent.children(&self.arena).count() < position {
                let placeholder = self.arena.new_node(NodeData {
                    hash: NodeHash(0),
                    kind: HtmlNodeKind::Text,
                    label: None,
                    properties: HtmlProps {
                        text: Some(String::new()),
                        attrs: IndexMap::new(),
                    },
                });
                parent.append(placeholder, &mut self.arena);
            }

            // Now insert at position (either appending or displacing)
            let current_children: Vec<_> = parent.children(&self.arena).collect();
            if position < current_children.len() {
                let occupant = current_children[position];
                occupant.insert_before(new_node, &mut self.arena);
                let slot = self.detach_to_slot(occupant);
                Some(slot)
            } else {
                parent.append(new_node, &mut self.arena);
                None
            }
        }
    }

    /// Move a node (already in shadow tree) to a new position.
    /// Returns the slot number if an occupant was displaced.
    fn move_to_position(
        &mut self,
        node: NodeId,
        new_parent: NodeId,
        position: usize,
    ) -> Option<u32> {
        // First check if node is detached - if so, remove from detached tracking
        let was_detached = self.detached_nodes.remove(&node).is_some();

        if !was_detached {
            // Node is in tree - detach it with placeholder to prevent shifts
            self.detach_with_placeholder(node);
        }

        // Now insert at target position
        let children: Vec<_> = new_parent.children(&self.arena).collect();
        debug!(
            ?node,
            position,
            children_count = children.len(),
            "Move: checking target position"
        );

        if position < children.len() {
            let occupant = children[position];
            let _occupant_kind = &self.arena[occupant].get().kind;
            debug!(
                ?occupant,
                ?_occupant_kind,
                "Move: found occupant at target position"
            );

            if occupant != node {
                // Displace occupant to a slot
                occupant.insert_before(node, &mut self.arena);
                let slot = self.detach_to_slot(occupant);
                debug!(?occupant, slot, "Move: detached occupant to slot");
                Some(slot)
            } else {
                debug!("Move: node already at target position");
                None
            }
        } else {
            debug!("Move: appending (no occupant)");
            new_parent.append(node, &mut self.arena);
            None
        }
    }

    /// Get a NodeRef with a specific position appended (for MOVE target).
    fn get_node_ref_with_position(&self, parent: NodeId, position: usize) -> NodeRef {
        // Check if parent is directly in a slot
        if let Some(&slot) = self.detached_nodes.get(&parent) {
            return NodeRef::Slot(slot, Some(NodePath(vec![position])));
        }

        // Check if an ancestor is in a slot
        if let Some((slot, relative_path)) = self.find_detached_ancestor(parent) {
            let mut path = relative_path.map(|p| p.0).unwrap_or_default();
            path.push(position);
            return NodeRef::Slot(slot, Some(NodePath(path)));
        }

        // Parent is in tree - compute its path and append position
        let mut parent_path = self.compute_path(parent);
        parent_path.push(position);
        NodeRef::Path(NodePath(parent_path))
    }
}

/// Convert cinereus ops to path-based Patches.
///
/// This uses a "shadow tree" approach: we maintain a mutable copy of tree_a
/// and simulate applying each operation to it. This lets us compute correct
/// paths that account for index shifts from earlier operations.
///
/// Key insight from Chawathe semantics: INSERT and MOVE do NOT shift siblings.
/// They DISPLACE whatever is at the target position into a slot for later use.
fn convert_ops_with_shadow(
    ops: Vec<EditOp<HtmlTreeTypes>>,
    tree_a: &Tree<HtmlTreeTypes>,
    tree_b: &Tree<HtmlTreeTypes>,
    matching: &Matching,
) -> Result<Vec<Patch>, String> {
    // Create shadow tree with encapsulated state
    let mut shadow = ShadowTree::new(tree_a.arena.clone(), tree_a.root);

    // Map from tree_b NodeIds to shadow tree NodeIds
    // Initially populated from matching (matched nodes)
    let mut b_to_shadow: HashMap<NodeId, NodeId> = HashMap::new();
    for (a_id, b_id) in matching.pairs() {
        b_to_shadow.insert(b_id, a_id);
    }

    // Collect all nodes that have explicit Insert operations in the ops list.
    // These should not be included as children during extract_content_from_tree_b.
    let nodes_with_insert_ops: HashSet<NodeId> = ops
        .iter()
        .filter_map(|op| match op {
            EditOp::Insert { node_b, .. } => Some(*node_b),
            _ => None,
        })
        .collect();

    let mut result = Vec::new();

    #[cfg(test)]
    shadow.debug_print_tree("Initial shadow tree");

    // Process operations in cinereus order.
    // For each op: update shadow tree, THEN compute paths from updated state.
    for op in ops {
        debug!(?op, "Processing operation");
        match op {
            EditOp::UpdateProperties {
                node_a,
                node_b: _,
                changes,
            } => {
                // Path to the node containing the attributes
                let path = shadow.compute_path(node_a);

                // Convert cinereus PropertyInFinalState to our PropChange
                let prop_changes: Vec<PropChange> = changes
                    .into_iter()
                    .map(|c| PropChange {
                        name: c.key,
                        value: match c.value {
                            cinereus::tree::PropValue::Same => None,
                            cinereus::tree::PropValue::Different(v) => Some(v),
                        },
                    })
                    .collect();

                // Always generate UpdateProps - the changes vec represents the complete final
                // state. An empty vec (or vec with only _text) means the element has no
                // attributes in the final state.
                result.push(Patch::UpdateProps {
                    path: NodePath(path),
                    changes: prop_changes,
                });
                // No structural change for UpdateProps
            }

            EditOp::Insert {
                node_b,
                parent_b,
                position,
                kind,
                ..
            } => {
                // Find the parent in our shadow tree
                let shadow_parent = b_to_shadow.get(&parent_b).copied().unwrap_or(shadow.root);

                // Create a new node in shadow tree (placeholder for structure tracking)
                let new_data: NodeData<HtmlTreeTypes> = NodeData {
                    hash: NodeHash(0),
                    kind: kind.clone(),
                    label: tree_b.get(node_b).label.clone(),
                    properties: tree_b.get(node_b).properties.clone(),
                };
                let new_node = shadow.arena.new_node(new_data);

                // Insert and handle displacement automatically
                let detach_to_slot = shadow.insert_at_position(shadow_parent, position, new_node);

                b_to_shadow.insert(node_b, new_node);

                // Get reference with position included - this makes Insert consistent with Move!
                let at = shadow.get_node_ref_with_position(shadow_parent, position);

                // Create the patch based on node kind
                match kind {
                    HtmlNodeKind::Element(tag) => {
                        let (attrs, children) = extract_content_from_tree_b(
                            node_b,
                            tree_b,
                            &b_to_shadow,
                            &nodes_with_insert_ops,
                        );
                        result.push(Patch::InsertElement {
                            at,
                            tag,
                            attrs,
                            children,
                            detach_to_slot,
                        });
                    }
                    HtmlNodeKind::Text => {
                        let text = tree_b
                            .get(node_b)
                            .properties
                            .text
                            .clone()
                            .unwrap_or_default();
                        result.push(Patch::InsertText {
                            at,
                            text,
                            detach_to_slot,
                        });
                    }
                    HtmlNodeKind::Comment => {
                        let text = tree_b
                            .get(node_b)
                            .properties
                            .text
                            .clone()
                            .unwrap_or_default();
                        result.push(Patch::InsertComment {
                            at,
                            text,
                            detach_to_slot,
                        });
                    }
                }

                #[cfg(test)]
                {
                    // Debug: print what's at the insertion position after Insert
                    debug!(
                        ?shadow_parent,
                        position,
                        ?detach_to_slot,
                        "After Insert - checking parent state"
                    );
                    if let Some(_parent_node) = shadow.arena.get(shadow_parent) {
                        let children: Vec<_> = shadow_parent
                            .children(&shadow.arena)
                            .enumerate()
                            .map(|(i, child)| {
                                let data = &shadow.arena[child].get();
                                (i, child, &data.kind)
                            })
                            .collect();
                        debug!(?children, "Parent children after Insert");
                    }
                    shadow.debug_print_tree("After Insert");
                }
            }

            EditOp::Delete { node_a } => {
                let _node_kind = &tree_a.get(node_a).kind;
                debug!(?node_a, ?_node_kind, "Delete operation");

                // Get the node reference - shadow.get_node_ref() automatically handles
                // ALL cases: directly detached, ancestor detached, or in tree!
                let node = shadow.get_node_ref(node_a);

                // If node was directly detached, remove it from tracking
                shadow.remove_from_detached(node_a);

                // Detach from tree with placeholder (if still in tree)
                if matches!(node, NodeRef::Path(_)) {
                    shadow.detach_with_placeholder(node_a);
                }

                result.push(Patch::Remove { node });

                #[cfg(test)]
                shadow.debug_print_tree("After Delete");
            }

            EditOp::Move {
                node_a,
                node_b,
                new_parent_b,
                new_position,
            } => {
                // Find new parent in shadow tree
                let shadow_new_parent = b_to_shadow
                    .get(&new_parent_b)
                    .copied()
                    .unwrap_or(shadow.root);

                debug!(
                    ?node_a,
                    ?new_parent_b,
                    ?shadow_new_parent,
                    ?new_position,
                    "Move: starting"
                );

                // Get source reference - handles all cases automatically!
                debug!(?node_a, "Move: computing from reference for node");
                let from = shadow.get_node_ref(node_a);
                debug!(?node_a, ?from, "Move: computed from reference");

                // Check if parent is detached
                #[allow(unused_variables)]
                let parent_is_detached = shadow.detached_nodes.contains_key(&shadow_new_parent);
                debug!(
                    ?shadow_new_parent,
                    parent_is_detached, "Move: checking if parent is detached"
                );

                // Debug: check what's at the target position BEFORE the move
                if shadow.arena.get(shadow_new_parent).is_some() {
                    #[allow(unused_variables)]
                    let children: Vec<_> = shadow_new_parent
                        .children(&shadow.arena)
                        .enumerate()
                        .map(|(i, child)| {
                            let data = &shadow.arena[child].get();
                            (i, child, &data.kind)
                        })
                        .collect();
                    debug!(?children, "Parent children BEFORE Move");
                }

                // Move node to new position - handles displacement automatically!
                let detach_to_slot =
                    shadow.move_to_position(node_a, shadow_new_parent, new_position);

                // Get target reference with position - handles all cases automatically!
                let to = shadow.get_node_ref_with_position(shadow_new_parent, new_position);

                debug!(?node_a, ?from, ?to, ?detach_to_slot, "Generated Move patch");

                // Debug: if we displaced something, check what the shadow tree thinks vs what will happen
                if let Some(slot) = detach_to_slot
                    && let Some((displaced_node, _)) =
                        shadow.detached_nodes.iter().find(|(_, s)| **s == slot)
                {
                    #[allow(unused_variables)]
                    let displaced_data = &shadow.arena[*displaced_node].get();
                    debug!(slot, ?displaced_node, kind = ?displaced_data.kind, "Shadow tree displaced to slot");
                }

                result.push(Patch::Move {
                    from,
                    to,
                    detach_to_slot,
                });

                // Update b_to_shadow
                b_to_shadow.insert(node_b, node_a);

                #[cfg(test)]
                shadow.debug_print_tree("After Move");
            }
        }
    }

    Ok(result)
}

/// Extract attributes and children from a node in tree_b.
fn extract_content_from_tree_b(
    node_b: NodeId,
    tree_b: &Tree<HtmlTreeTypes>,
    b_to_shadow: &HashMap<NodeId, NodeId>,
    nodes_with_insert_ops: &HashSet<NodeId>,
) -> (Vec<(String, String)>, Vec<InsertContent>) {
    let data = tree_b.get(node_b);
    let attrs: Vec<_> = data
        .properties
        .attrs
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    // Get children
    let mut children = Vec::new();
    for child_id in tree_b.children(node_b) {
        // Skip children that:
        // 1. Are matched (will be handled by Move operations)
        // 2. Have their own Insert operations (not simplified away)
        if b_to_shadow.contains_key(&child_id) || nodes_with_insert_ops.contains(&child_id) {
            continue;
        }

        let child_data = tree_b.get(child_id);
        match &child_data.kind {
            HtmlNodeKind::Element(tag) => {
                let (child_attrs, child_children) = extract_content_from_tree_b(
                    child_id,
                    tree_b,
                    b_to_shadow,
                    nodes_with_insert_ops,
                );
                children.push(InsertContent::Element {
                    tag: tag.clone(),
                    attrs: child_attrs,
                    children: child_children,
                });
            }
            HtmlNodeKind::Text => {
                let text = child_data.properties.text.clone().unwrap_or_default();
                children.push(InsertContent::Text(text));
            }
            HtmlNodeKind::Comment => {
                let text = child_data.properties.text.clone().unwrap_or_default();
                children.push(InsertContent::Comment(text));
            }
        }
    }

    (attrs, children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use facet_testhelpers::test;

    #[test]
    fn test_build_tree_simple() {
        let elem = Element {
            tag: "div".to_string(),
            attrs: IndexMap::new(),
            children: vec![Content::Text("hello".to_string())],
        };

        let tree = build_tree(&elem);

        // Root is div element
        let root_data = tree.get(tree.root);
        assert!(matches!(root_data.kind, HtmlNodeKind::Element(ref t) if t == "div"));

        // One child (text)
        assert_eq!(tree.child_count(tree.root), 1);
    }

    #[test]
    fn test_diff_text_change() {
        let old = Element {
            tag: "div".to_string(),
            attrs: IndexMap::new(),
            children: vec![Content::Text("old".to_string())],
        };
        let new = Element {
            tag: "div".to_string(),
            attrs: IndexMap::new(),
            children: vec![Content::Text("new".to_string())],
        };

        let patches = diff_elements(&old, &new).unwrap();

        // Should have an UpdateProps patch for the text change
        let has_text_update = patches.iter().any(|p| {
            matches!(p, Patch::UpdateProps { changes, .. }
                if changes.iter().any(|c| c.name == "_text"))
        });
        assert!(
            has_text_update,
            "Expected text update patch, got: {:?}",
            patches
        );
    }

    #[test]
    fn test_diff_attr_change() {
        let mut old_attrs = IndexMap::new();
        old_attrs.insert("class".to_string(), "foo".to_string());

        let mut new_attrs = IndexMap::new();
        new_attrs.insert("class".to_string(), "bar".to_string());

        let old = Element {
            tag: "div".to_string(),
            attrs: old_attrs,
            children: vec![],
        };
        let new = Element {
            tag: "div".to_string(),
            attrs: new_attrs,
            children: vec![],
        };

        let patches = diff_elements(&old, &new).unwrap();

        let has_attr_update = patches.iter().any(|p| {
            matches!(p, Patch::UpdateProps { changes, .. }
                if changes.iter().any(|c| c.name == "class"))
        });
        assert!(
            has_attr_update,
            "Expected attr update patch, got: {:?}",
            patches
        );
    }

    #[test]
    fn test_diff_remove_all_children() {
        // Reproduce fuzzer failure: body with child -> body with no children
        let old = Element {
            tag: "body".to_string(),
            attrs: IndexMap::new(),
            children: vec![Content::Element(Element {
                tag: "span".to_string(),
                attrs: IndexMap::new(),
                children: vec![],
            })],
        };
        let new = Element {
            tag: "body".to_string(),
            attrs: IndexMap::new(),
            children: vec![],
        };

        let patches = diff_elements(&old, &new).unwrap();
        debug!("Patches: {:#?}", patches);

        // Should be able to apply the patches
        let mut tree = old.clone();
        super::super::apply::apply_patches(&mut tree, &patches).expect("apply should succeed");

        // Result HTML should match new HTML (Chawathe semantics use empty text placeholders,
        // so we compare serialized output rather than tree structure)
        assert_eq!(tree.to_html(), new.to_html(), "HTML output should match");
    }

    #[test]
    fn test_diff_complex_fuzzer_case() {
        // Fuzzer found: <body><strong>old_text</strong></body> -> <body>new_text<strong>updated</strong></body>
        let old = Element {
            tag: "body".to_string(),
            attrs: IndexMap::new(),
            children: vec![Content::Element(Element {
                tag: "strong".to_string(),
                attrs: IndexMap::new(),
                children: vec![Content::Text("old".to_string())],
            })],
        };
        let new = Element {
            tag: "body".to_string(),
            attrs: IndexMap::new(),
            children: vec![
                Content::Text("new_text".to_string()),
                Content::Element(Element {
                    tag: "strong".to_string(),
                    attrs: IndexMap::new(),
                    children: vec![Content::Text("updated".to_string())],
                }),
            ],
        };

        let patches = diff_elements(&old, &new).unwrap();
        debug!("Patches: {:#?}", patches);

        let mut tree = old.clone();
        super::super::apply::apply_patches(&mut tree, &patches).expect("apply should succeed");

        debug!("Result: {}", tree.to_html());
        debug!("Expected: {}", new.to_html());
        assert_eq!(tree.to_html(), new.to_html(), "HTML output should match");
    }

    #[test]
    fn test_diff_actual_fuzzer_crash() {
        // Actual fuzzer crash case (simplified):
        // Old: <strong>text1</strong><strong>text2</strong><img>
        // New: text3<strong>text4</strong>
        let old = Element {
            tag: "body".to_string(),
            attrs: IndexMap::new(),
            children: vec![
                Content::Element(Element {
                    tag: "strong".to_string(),
                    attrs: IndexMap::new(),
                    children: vec![Content::Text("text1".to_string())],
                }),
                Content::Element(Element {
                    tag: "strong".to_string(),
                    attrs: IndexMap::new(),
                    children: vec![Content::Text("text2".to_string())],
                }),
                Content::Element(Element {
                    tag: "img".to_string(),
                    attrs: IndexMap::new(),
                    children: vec![],
                }),
            ],
        };
        let new = Element {
            tag: "body".to_string(),
            attrs: IndexMap::new(),
            children: vec![
                Content::Text("text3".to_string()),
                Content::Element(Element {
                    tag: "strong".to_string(),
                    attrs: IndexMap::new(),
                    children: vec![Content::Text("text4".to_string())],
                }),
            ],
        };

        let patches = diff_elements(&old, &new).unwrap();
        debug!("Patches: {:#?}", patches);

        let mut tree = old.clone();
        super::super::apply::apply_patches(&mut tree, &patches).expect("apply should succeed");

        debug!("Result: {}", tree.to_html());
        debug!("Expected: {}", new.to_html());
        assert_eq!(tree.to_html(), new.to_html(), "HTML output should match");
    }

    #[test]
    fn test_fuzzer_special_chars() {
        trace!("what");

        // Test with actual fuzzer input that has special chars
        // html5ever parses "<jva       xx a >" as an element, creating nested structure
        // The bug: UpdateProps at [1,0] followed by Remove at [1,0] - we update text then delete it!
        // This appears to be a path tracking bug when handling complex displacement scenarios.
        let old_html = r#"<html><body><strong>n<&nhnnz"""" v</strong><strong>< bit<jva       xx a ></strong><img src="n" alt="v"></body></html>"#;
        let new_html = r#"<html><body>n<strong>aaa</strong></body></html>"#;

        let patches = super::super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut tree = super::super::apply::parse_html(old_html).expect("parse old failed");
        trace!("Old tree: {:#?}", tree);

        super::super::apply::apply_patches(&mut tree, &patches).expect("apply failed");

        let result = tree.to_html();
        let expected_tree = super::super::apply::parse_html(new_html).expect("parse new failed");
        let expected = expected_tree.to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_img_li_roundtrip() {
        // Fuzzer found roundtrip failure with img and li elements
        let old_html = r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://www.w3.org/TR/html4/strict.dtd"><html><body><p>Unclosed paragraph<span class=""></span><div>Inside P which browsers will auto-close</div><span>Unclosed span<div>Block in span</div></body></html>"#;
        let new_html = r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://www.w3.org/TR/html4/strict.dtd"><html><body><img src="vvv" alt="ttt">d <li>d<<<&<<a"d <<<</li></body></html>"#;

        let patches = super::super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = crate::arena_dom::parse(old_html);
        doc.apply_patches(&patches).expect("apply failed");

        let result = doc.to_html();
        let expected_doc = crate::arena_dom::parse(new_html);
        let expected = expected_doc.to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_nested_ul_remove() {
        // Fuzzer found issue with nested ul elements - img not being removed correctly
        let old_html = r#"<!DOCTYPE html><html><body><ul class="h"><ul class="z"><img src="vvv" alt="wvv"><ul class="h"><img src="vvv"></ul></ul></ul></body></html>"#;
        let new_html = r#"<!DOCTYPE html><html><body><ul class="h"><ul class="h"></ul><ul class="q"><img src="aaa"></ul></ul></body></html>"#;

        let patches = super::super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = crate::arena_dom::parse(old_html);
        doc.apply_patches(&patches).expect("apply failed");

        let result = doc.to_html();
        let expected_doc = crate::arena_dom::parse(new_html);
        let expected = expected_doc.to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_em_li_navigate_text() {
        // Fuzzer found "Insert: cannot navigate through text node" error
        // The fuzzer generates unescaped HTML which html5ever parses as actual elements
        let old_html = r#"<html><body><em> <v<      << v</em></body></html>"#;
        let new_html = r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd"><html><body><li>a< <v<      <<</li><img src=""></body></html>"#;

        let old_tree = super::super::apply::parse_html(old_html).expect("parse old failed");
        let new_tree = super::super::apply::parse_html(new_html).expect("parse new failed");
        debug!("Old HTML parsed: {:#?}", old_tree);
        debug!("New HTML parsed: {:#?}", new_tree);

        let patches = super::super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut tree = super::super::apply::parse_html(old_html).expect("parse old failed");
        super::super::apply::apply_patches(&mut tree, &patches).expect("apply failed");

        let result = tree.to_html();
        let expected_tree = super::super::apply::parse_html(new_html).expect("parse new failed");
        let expected = expected_tree.to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_nested_ol_patch_order() {
        // Fuzzer found "patch at index 4 is out of order" error
        let old_html = r#"<!DOCTYPE html><html><body><ol start="0"></ol></body></html>"#;
        let new_html = r#"<html><body><ol start="255"></ol><ol start="93"></ol><ol start="91"><ol start="1"><a href="vaaaaaaaaaaaaa"></a></ol></ol></body></html>"#;

        let patches = super::super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = crate::arena_dom::parse(old_html);
        doc.apply_patches(&patches).expect("apply failed");

        let result = doc.to_html();
        let expected_doc = crate::arena_dom::parse(new_html);
        let expected = expected_doc.to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_slot_contains_text() {
        // Fuzzer found "Move: slot contains text, cannot navigate to child" error
        let old_html = r#"<!DOCTYPE html><html><body><article><code><</code><code><</code><code><</code><code><</code></article></body></html>"#;
        let new_html = r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd"><html><body><code><</code><code><</code><code><</code><code><</code><article><code><</code><code><</code><h2><<<<<<<<<<<<<</h2></article></body></html>"#;

        let patches = super::super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut tree = super::super::apply::parse_html(old_html).expect("parse old failed");
        super::super::apply::apply_patches(&mut tree, &patches).expect("apply failed");

        let result = tree.to_html();
        let expected_tree = super::super::apply::parse_html(new_html).expect("parse new failed");
        let expected = expected_tree.to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_article_code_move() {
        // Fuzzer found "Move: slot contains text, cannot navigate to child"
        // Fuzzer generates unescaped HTML
        let old_html = r#"<!DOCTYPE html><html><body><article><code><</code><code><</code><code><</code><code><</code><article><article><code><</code></article></article></article></body></html>"#;
        let new_html = r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd"><html><body><code><</code><code><</code><code><</code><code><</code><article><code><</code><code><</code><h2><<<<<<<<<<<<<</h2></article></body></html>"#;

        let patches = super::super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut tree = super::super::apply::parse_html(old_html).expect("parse old failed");
        super::super::apply::apply_patches(&mut tree, &patches).expect("apply failed");

        let result = tree.to_html();
        let expected_tree = super::super::apply::parse_html(new_html).expect("parse new failed");
        let expected = expected_tree.to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_arena_dom_diff() {
        // Test diffing with arena_dom documents
        let old_html = "<html><body><div>Old content</div></body></html>";
        let new_html = "<html><body><div>New content</div></body></html>";

        let old_doc = arena_dom::parse(old_html);
        let new_doc = arena_dom::parse(new_html);

        let patches = diff_arena_documents(&old_doc, &new_doc).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        // Should have an UpdateProps patch with _text change
        assert_eq!(patches.len(), 1);
        match &patches[0] {
            Patch::UpdateProps { path, changes } => {
                assert_eq!(path.0, vec![0, 0]); // text node path
                assert_eq!(changes.len(), 1);
                assert_eq!(changes[0].name, "_text");
                assert_eq!(changes[0].value, Some("New content".to_string()));
            }
            _ => panic!("Expected UpdateProps patch, got {:?}", patches[0]),
        }
    }

    #[test]
    fn test_arena_dom_diff_add_element() {
        let old_html = "<html><body><div>Content</div></body></html>";
        let new_html = "<html><body><div>Content</div><p>New paragraph</p></body></html>";

        let old_doc = arena_dom::parse(old_html);
        let new_doc = arena_dom::parse(new_html);

        let patches = diff_arena_documents(&old_doc, &new_doc).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        // Should have an InsertElement patch
        assert!(!patches.is_empty());
        let has_insert = patches
            .iter()
            .any(|p| matches!(p, Patch::InsertElement { .. }));
        assert!(has_insert, "Should have InsertElement patch");
    }
}
