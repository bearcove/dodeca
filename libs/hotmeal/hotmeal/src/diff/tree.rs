//! Direct cinereus tree implementation for HTML DOM.
//!
//! This module implements cinereus traits directly on hotmeal's DOM types,
//! bypassing the facet reflection layer for simpler, more efficient diffing.

use cinereus::{
    EditOp, Matching, MatchingConfig, NodeData, NodeHash, Properties, PropertyChange, Tree,
    TreeTypes, diff_trees_with_matching,
    indextree::{self, NodeId},
};
use rapidhash::RapidHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use super::apply::{Content, Element};
use super::{InsertContent, NodePath, NodeRef, Patch, PropChange};

/// Node kind in the HTML tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HtmlNodeKind {
    /// An element node with a tag name
    Element(String),
    /// A text node
    Text,
}

impl std::fmt::Display for HtmlNodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HtmlNodeKind::Element(tag) => write!(f, "<{}>", tag),
            HtmlNodeKind::Text => write!(f, "#text"),
        }
    }
}

/// HTML element properties (attributes + text content).
#[derive(Debug, Clone, Default)]
pub struct HtmlProps {
    /// Element attributes
    pub attrs: HashMap<String, String>,
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

    fn diff(&self, other: &Self) -> Vec<PropertyChange<Self::Key, Self::Value>> {
        let mut changes = Vec::new();

        // Diff text content
        if self.text != other.text {
            changes.push(PropertyChange {
                key: "_text".to_string(),
                old_value: self.text.clone(),
                new_value: other.text.clone(),
            });
        }

        // Diff attributes
        // Added or changed
        for (key, new_val) in &other.attrs {
            let old_val = self.attrs.get(key);
            if old_val != Some(new_val) {
                changes.push(PropertyChange {
                    key: key.clone(),
                    old_value: old_val.cloned(),
                    new_value: Some(new_val.clone()),
                });
            }
        }

        // Removed
        for key in self.attrs.keys() {
            if !other.attrs.contains_key(key) {
                changes.push(PropertyChange {
                    key: key.clone(),
                    old_value: self.attrs.get(key).cloned(),
                    new_value: None,
                });
            }
        }

        changes
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
        attrs: HashMap::new(),
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
        eprintln!(
            "tree_a: root hash={:?}, kind={:?}",
            tree_a.get(tree_a.root).hash,
            tree_a.get(tree_a.root).kind
        );
        eprintln!(
            "tree_b: root hash={:?}, kind={:?}",
            tree_b.get(tree_b.root).hash,
            tree_b.get(tree_b.root).kind
        );
    }

    let config = MatchingConfig {
        min_height: 0, // Include all nodes including leaves in top-down matching
        ..MatchingConfig::default()
    };
    let (edit_ops, matching) = diff_trees_with_matching(&tree_a, &tree_b, &config);

    #[cfg(test)]
    {
        eprintln!("matching pairs: {}", matching.len());
        for (a, b) in matching.pairs() {
            eprintln!("  matched: {:?} <-> {:?}", a, b);
        }
        eprintln!("edit_ops: {:?}", edit_ops);
    }

    debug!(
        ops_count = edit_ops.len(),
        matched_pairs = matching.len(),
        "cinereus diff complete"
    );

    // Convert cinereus ops to patches using shadow tree approach
    convert_ops_with_shadow(edit_ops, &tree_a, &tree_b, &matching)
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
    // Shadow tree: mutable clone of tree_a's structure
    let mut shadow_arena = tree_a.arena.clone();
    let shadow_root = tree_a.root;

    // Map from tree_b NodeIds to shadow tree NodeIds
    // Initially populated from matching (matched nodes)
    let mut b_to_shadow: HashMap<NodeId, NodeId> = HashMap::new();
    for (a_id, b_id) in matching.pairs() {
        b_to_shadow.insert(b_id, a_id);
    }

    let mut result = Vec::new();

    // Track detached nodes: NodeId -> slot number
    // When an Insert/Move places a node at a position occupied by another node,
    // the occupant is detached and stored in a slot for later reinsertion.
    let mut detached_nodes: HashMap<NodeId, u32> = HashMap::new();
    let mut next_slot: u32 = 0;

    // Process operations in cinereus order.
    // For each op: update shadow tree, THEN compute paths from updated state.
    for op in ops {
        match op {
            EditOp::UpdateProperties {
                node_a,
                node_b: _,
                changes,
            } => {
                // Path to the node containing the attributes
                let path = compute_path_in_shadow(&shadow_arena, shadow_root, node_a);

                // Convert cinereus PropertyChange to our PropChange
                let prop_changes: Vec<PropChange> = changes
                    .into_iter()
                    .map(|c| PropChange {
                        name: c.key,
                        value: c.new_value,
                    })
                    .collect();

                if !prop_changes.is_empty() {
                    result.push(Patch::UpdateProps {
                        path: NodePath(path),
                        changes: prop_changes,
                    });
                }
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
                let shadow_parent = b_to_shadow.get(&parent_b).copied().unwrap_or(shadow_root);

                // Create a new node in shadow tree (placeholder for structure tracking)
                let new_data: NodeData<HtmlTreeTypes> = NodeData {
                    hash: NodeHash(0),
                    kind: kind.clone(),
                    label: tree_b.get(node_b).label.clone(),
                    properties: tree_b.get(node_b).properties.clone(),
                };
                let new_node = shadow_arena.new_node(new_data);

                // In Chawathe semantics, Insert does NOT shift - it places at position
                // and whatever was there gets displaced (detached to a slot).
                let children: Vec<_> = shadow_parent.children(&shadow_arena).collect();
                let detach_to_slot = if position < children.len() {
                    let occupant = children[position];
                    let occupant_slot = next_slot;
                    next_slot += 1;
                    // Insert new_node before occupant, then detach occupant
                    occupant.insert_before(new_node, &mut shadow_arena);
                    occupant.detach(&mut shadow_arena);
                    detached_nodes.insert(occupant, occupant_slot);
                    Some(occupant_slot)
                } else {
                    // No occupant, just append
                    shadow_parent.append(new_node, &mut shadow_arena);
                    None
                };

                b_to_shadow.insert(node_b, new_node);

                // Determine the parent reference - either a path or a slot
                let parent = if let Some(&slot) = detached_nodes.get(&shadow_parent) {
                    // Parent is directly in a slot
                    NodeRef::Slot(slot, None)
                } else if let Some((slot, relative_path)) =
                    find_detached_ancestor(&shadow_arena, shadow_parent, &detached_nodes)
                {
                    // An ancestor is in a slot - include the relative path to the parent
                    NodeRef::Slot(slot, relative_path)
                } else {
                    // Parent is in the tree - compute its path
                    let parent_path =
                        compute_path_in_shadow(&shadow_arena, shadow_root, shadow_parent);
                    NodeRef::Path(NodePath(parent_path))
                };

                // Create the patch based on node kind
                match kind {
                    HtmlNodeKind::Element(tag) => {
                        let (attrs, children) = extract_content_from_tree_b(node_b, tree_b);
                        result.push(Patch::InsertElement {
                            parent,
                            position,
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
                            parent,
                            position,
                            text,
                            detach_to_slot,
                        });
                    }
                }
            }

            EditOp::Delete { node_a } => {
                // Check if the node or any ancestor is currently detached (in a slot)
                let node = if let Some(slot) = detached_nodes.remove(&node_a) {
                    // Node is directly in a slot - delete from slot
                    NodeRef::Slot(slot, None)
                } else if let Some((slot, relative_path)) =
                    find_detached_ancestor(&shadow_arena, node_a, &detached_nodes)
                {
                    // An ancestor is in a slot - the node is inside a detached subtree
                    NodeRef::Slot(slot, relative_path)
                } else {
                    // Node is in the tree - delete from path
                    let path = compute_path_in_shadow(&shadow_arena, shadow_root, node_a);
                    // Swap with placeholder (insert placeholder before, then detach)
                    // This prevents shifting of siblings.
                    let placeholder_data: NodeData<HtmlTreeTypes> = NodeData {
                        hash: NodeHash(0),
                        kind: HtmlNodeKind::Text,
                        label: None,
                        properties: HtmlProps::default(),
                    };
                    let placeholder = shadow_arena.new_node(placeholder_data);
                    node_a.insert_before(placeholder, &mut shadow_arena);
                    node_a.detach(&mut shadow_arena);
                    NodeRef::Path(NodePath(path))
                };

                result.push(Patch::Remove { node });
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
                    .unwrap_or(shadow_root);

                // Check if the node is currently detached (in limbo)
                let is_detached = detached_nodes.contains_key(&node_a);

                // Determine the source for the Move
                let from = if is_detached {
                    let slot = detached_nodes.remove(&node_a).unwrap();
                    NodeRef::Slot(slot, None)
                } else {
                    let old_path = compute_path_in_shadow(&shadow_arena, shadow_root, node_a);
                    // Swap node_a with a placeholder (insert placeholder before, then detach)
                    // This prevents shifting of siblings.
                    let placeholder_data: NodeData<HtmlTreeTypes> = NodeData {
                        hash: NodeHash(0),
                        kind: HtmlNodeKind::Text,
                        label: None,
                        properties: HtmlProps::default(),
                    };
                    let placeholder = shadow_arena.new_node(placeholder_data);
                    node_a.insert_before(placeholder, &mut shadow_arena);
                    node_a.detach(&mut shadow_arena);
                    NodeRef::Path(NodePath(old_path))
                };

                // Check if something is at the target position that needs to be detached
                let children: Vec<_> = shadow_new_parent.children(&shadow_arena).collect();
                let detach_to_slot = if new_position < children.len() {
                    let occupant = children[new_position];
                    // Don't detach ourselves (shouldn't happen since we already detached)
                    if occupant != node_a {
                        let occupant_slot = next_slot;
                        next_slot += 1;
                        // Insert node_a before occupant, then detach occupant
                        occupant.insert_before(node_a, &mut shadow_arena);
                        occupant.detach(&mut shadow_arena);
                        detached_nodes.insert(occupant, occupant_slot);
                        Some(occupant_slot)
                    } else {
                        // node_a is already at the target position, nothing to do
                        None
                    }
                } else {
                    // No occupant, just append
                    shadow_new_parent.append(node_a, &mut shadow_arena);
                    None
                };

                // Compute the target path
                let to = if let Some(&slot) = detached_nodes.get(&shadow_new_parent) {
                    // Parent is directly in a slot
                    NodeRef::Slot(slot, Some(NodePath(vec![new_position])))
                } else if let Some((slot, relative_path)) =
                    find_detached_ancestor(&shadow_arena, shadow_new_parent, &detached_nodes)
                {
                    // An ancestor is in a slot - extend the relative path
                    let mut rel_path = relative_path.map(|p| p.0).unwrap_or_default();
                    rel_path.push(new_position);
                    NodeRef::Slot(slot, Some(NodePath(rel_path)))
                } else {
                    // Parent is in the tree - compute its path
                    let mut parent_path =
                        compute_path_in_shadow(&shadow_arena, shadow_root, shadow_new_parent);
                    parent_path.push(new_position);
                    NodeRef::Path(NodePath(parent_path))
                };

                result.push(Patch::Move {
                    from,
                    to,
                    detach_to_slot,
                });

                // Update b_to_shadow
                b_to_shadow.insert(node_b, node_a);
            }
        }
    }

    Ok(result)
}

/// Check if any ancestor of a node is in the detached_nodes map.
/// Returns (slot_number, relative_path) if an ancestor is detached, None otherwise.
fn find_detached_ancestor(
    shadow_arena: &indextree::Arena<NodeData<HtmlTreeTypes>>,
    node: NodeId,
    detached_nodes: &HashMap<NodeId, u32>,
) -> Option<(u32, Option<NodePath>)> {
    let mut current = node;
    // Collect (child_node, parent_node) pairs as we traverse up
    let mut traversal: Vec<(NodeId, NodeId)> = Vec::new();

    loop {
        // Check if current node is detached
        if let Some(&slot) = detached_nodes.get(&current) {
            // Build the relative path from slot root to the original node
            let relative_path = if traversal.is_empty() {
                None // Node is directly the slot root
            } else {
                // Get the slot root's path length
                let slot_root_path_len = shadow_arena
                    .get(current)
                    .and_then(|n| n.get().label.as_ref())
                    .map(|l| l.0.len())
                    .unwrap_or(0);

                // Get the target node's full path
                let target_path = shadow_arena
                    .get(node)
                    .and_then(|n| n.get().label.as_ref())
                    .map(|l| l.0.clone());

                if let Some(full_path) = target_path {
                    // The relative path is the suffix after the slot root's path
                    if full_path.len() > slot_root_path_len {
                        let relative_segments = full_path[slot_root_path_len..].to_vec();
                        Some(NodePath(relative_segments))
                    } else {
                        None
                    }
                } else {
                    // Fallback: use position indices if labels aren't available
                    let mut path_indices: Vec<usize> = Vec::new();
                    for (child, parent) in traversal.iter().rev() {
                        let pos = parent
                            .children(shadow_arena)
                            .position(|c| c == *child)
                            .unwrap_or(0);
                        path_indices.push(pos);
                    }
                    Some(NodePath(path_indices))
                }
            };
            return Some((slot, relative_path));
        }
        // Move to parent, recording the traversal
        if let Some(parent_id) = shadow_arena.get(current).and_then(|n| n.parent()) {
            traversal.push((current, parent_id));
            current = parent_id;
        } else {
            // No more parents
            break;
        }
    }
    None
}

/// Compute path for a node in the shadow tree by walking up to root.
fn compute_path_in_shadow(
    shadow_arena: &indextree::Arena<NodeData<HtmlTreeTypes>>,
    shadow_root: NodeId,
    node: NodeId,
) -> Vec<usize> {
    // If this node has a label, we could use it, but we need to account for
    // any index shifts from insertions/deletions. So we compute by walking.
    let mut segments = Vec::new();
    let mut current = node;

    while current != shadow_root {
        if let Some(parent_id) = shadow_arena.get(current).and_then(|n| n.parent()) {
            let pos = parent_id
                .children(shadow_arena)
                .position(|c| c == current)
                .unwrap_or(0);
            segments.push(pos);
            current = parent_id;
        } else {
            break;
        }
    }

    segments.reverse();
    segments
}

/// Extract attributes and children from a node in tree_b.
fn extract_content_from_tree_b(
    node_b: NodeId,
    tree_b: &Tree<HtmlTreeTypes>,
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
        let child_data = tree_b.get(child_id);
        match &child_data.kind {
            HtmlNodeKind::Element(tag) => {
                let (child_attrs, child_children) = extract_content_from_tree_b(child_id, tree_b);
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
        }
    }

    (attrs, children)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_tree_simple() {
        let elem = Element {
            tag: "div".to_string(),
            attrs: HashMap::new(),
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
            attrs: HashMap::new(),
            children: vec![Content::Text("old".to_string())],
        };
        let new = Element {
            tag: "div".to_string(),
            attrs: HashMap::new(),
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
        let mut old_attrs = HashMap::new();
        old_attrs.insert("class".to_string(), "foo".to_string());

        let mut new_attrs = HashMap::new();
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
}
