//! Chawathe edit script generation algorithm.
//!
//! Generates a minimal edit script (INSERT, DELETE, MOVE, UpdateProperty) from a node matching.
//! Based on "Change Detection in Hierarchically Structured Information" (Chawathe et al., 1996).
//!
//! The algorithm has 4 phases:
//! 1. UpdateProperty: Diff properties for matched nodes
//! 2. INSERT: Add nodes that exist only in the destination tree
//! 3. MOVE: Relocate nodes to new parents or positions
//! 4. DELETE: Remove nodes that exist only in the source tree

use crate::{debug, trace};
use core::fmt;

use crate::matching::Matching;
use crate::tree::{DiffTree, Properties, PropertyInFinalState, TreeTypes};
use indextree::NodeId;

/// Type alias for final property states to satisfy clippy::type_complexity
pub type PropChanges<T> = Vec<
    PropertyInFinalState<
        <<T as TreeTypes>::Props as Properties>::Key,
        <<T as TreeTypes>::Props as Properties>::Value,
    >,
>;

/// An edit operation in the diff.
#[derive(Clone, PartialEq, Eq)]
pub enum EditOp<T: TreeTypes> {
    /// Update multiple properties on a matched node.
    UpdateProperties {
        /// The node in tree A
        node_a: NodeId,
        /// The corresponding node in tree B
        node_b: NodeId,
        /// The property changes
        changes: PropChanges<T>,
    },

    /// Insert a new node.
    Insert {
        /// The new node in tree B
        node_b: NodeId,
        /// Parent in tree B
        parent_b: NodeId,
        /// Position among siblings (0-indexed)
        position: usize,
        /// The node's kind
        kind: T::Kind,
    },

    /// Delete a node.
    Delete {
        /// The node in tree A being deleted
        node_a: NodeId,
    },

    /// Move a node to a new location.
    Move {
        /// The node in tree A
        node_a: NodeId,
        /// The corresponding node in tree B
        node_b: NodeId,
        /// New parent in tree B
        new_parent_b: NodeId,
        /// New position among siblings
        new_position: usize,
    },

    /// Update text content on a text/comment node.
    SetText {
        /// The node in tree A
        node_a: NodeId,
        /// The corresponding node in tree B
        node_b: NodeId,
        /// The new text content
        text: T::Text,
    },
}

impl<T: TreeTypes> fmt::Display for EditOp<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditOp::UpdateProperties {
                node_a, changes, ..
            } => {
                write!(f, "UpdateProps(a:{}", usize::from(*node_a))?;
                for change in changes {
                    write!(f, " {}:", change.key)?;
                    match &change.value {
                        crate::tree::PropValue::Same => write!(f, "=")?,
                        crate::tree::PropValue::Different(v) => write!(f, " → {}", v)?,
                    }
                }
                write!(f, ")")
            }
            EditOp::Insert {
                node_b,
                parent_b,
                position,
                kind,
                ..
            } => {
                write!(
                    f,
                    "Insert(b:{} {} @{} under b:{})",
                    usize::from(*node_b),
                    kind,
                    position,
                    usize::from(*parent_b)
                )
            }
            EditOp::Delete { node_a } => {
                write!(f, "Delete(a:{})", usize::from(*node_a))
            }
            EditOp::Move {
                node_a,
                node_b,
                new_parent_b,
                new_position,
            } => {
                write!(
                    f,
                    "Move(a:{} → b:{} @{} under b:{})",
                    usize::from(*node_a),
                    usize::from(*node_b),
                    new_position,
                    usize::from(*new_parent_b)
                )
            }
            EditOp::SetText {
                node_a,
                node_b,
                text,
            } => {
                write!(
                    f,
                    "SetText(a:{} → b:{} text={})",
                    usize::from(*node_a),
                    usize::from(*node_b),
                    text
                )
            }
        }
    }
}

impl<T: TreeTypes> fmt::Debug for EditOp<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Reuse Display implementation for Debug
        fmt::Display::fmt(self, f)
    }
}

/// Wrapper for collecting edit operations with automatic tracing.
struct Ops<T: TreeTypes> {
    inner: Vec<EditOp<T>>,
}

impl<T: TreeTypes> Ops<T> {
    fn new() -> Self {
        Self { inner: Vec::new() }
    }

    fn push(&mut self, op: EditOp<T>) {
        debug!(%op, "emit");
        self.inner.push(op);
    }

    fn into_inner(self) -> Vec<EditOp<T>> {
        self.inner
    }
}

/// Generate an edit script from a matching between two trees.
///
/// The edit script transforms tree A into tree B using INSERT, DELETE, MOVE,
/// and UpdateProperty operations.
///
/// The two trees can have different concrete types as long as they share the same
/// TreeTypes (Kind and Props). This allows mixing different tree representations.
pub fn generate_edit_script<TA, TB>(
    tree_a: &TA,
    tree_b: &TB,
    matching: &Matching,
) -> Vec<EditOp<TA::Types>>
where
    TA: DiffTree,
    TB: DiffTree<Types = TA::Types>,
{
    trace!(matched_pairs = matching.len(), "generate_edit_script start");
    let mut ops = Ops::new();

    // Pre-compute descendants of opaque nodes (excluding the opaque nodes themselves).
    // These nodes are part of atomic subtrees and should not participate in edit generation.
    let mut opaque_descendants_a: std::collections::HashSet<indextree::NodeId> =
        std::collections::HashSet::new();
    for a_id in tree_a.iter() {
        if tree_a.is_opaque(a_id) {
            for desc in tree_a.descendants(a_id) {
                if desc != a_id {
                    opaque_descendants_a.insert(desc);
                }
            }
        }
    }
    let mut opaque_descendants_b: std::collections::HashSet<indextree::NodeId> =
        std::collections::HashSet::new();
    for b_id in tree_b.iter() {
        if tree_b.is_opaque(b_id) {
            for desc in tree_b.descendants(b_id) {
                if desc != b_id {
                    opaque_descendants_b.insert(desc);
                }
            }
        }
    }

    // Phase 1: Text and property changes for matched nodes
    for (a_id, b_id) in matching.pairs() {
        // Skip descendants of opaque nodes — their content is handled separately
        if opaque_descendants_a.contains(&a_id) || opaque_descendants_b.contains(&b_id) {
            continue;
        }
        // Handle text changes (for text/comment nodes)
        let a_text = tree_a.text(a_id);
        let b_text = tree_b.text(b_id);
        // Emit SetText if text changed and there's new text content
        // Note: if b_text is None but a_text is Some, the text is being removed.
        // This shouldn't happen for text nodes (they always have text),
        // but could happen if text is optional. We skip emitting SetText
        // for removal since the node type change should handle it.
        if a_text != b_text
            && let Some(new_text) = b_text
        {
            ops.push(EditOp::SetText {
                node_a: a_id,
                node_b: b_id,
                text: new_text.clone(),
            });
        }

        // Handle property changes (for element nodes)
        let a_props = tree_a.properties(a_id);
        let b_props = tree_b.properties(b_id);

        // Short-circuit: if both have no properties, nothing to do
        if a_props.is_empty() && b_props.is_empty() {
            continue;
        }

        let changes: Vec<_> = a_props.diff(b_props);
        // Generate UpdateProperties only if there's a real change:
        // 1. At least one property value changed (Different), or
        // 2. Properties were removed (old has more than new's final state)
        let has_real_change = changes
            .iter()
            .any(|c| matches!(c.value, crate::tree::PropValue::Different(_)));
        let has_removal = a_props.len() > changes.len();
        if has_real_change || has_removal {
            ops.push(EditOp::UpdateProperties {
                node_a: a_id,
                node_b: b_id,
                changes,
            });
        }
    }

    // Phase 2 & 3: INSERT - nodes in B that are not matched
    // Process in breadth-first order so parents are inserted before children
    for b_id in tree_b.iter() {
        if !matching.contains_b(b_id) {
            let parent_b = tree_b.parent(b_id);

            if let Some(parent_b) = parent_b {
                // Skip descendants of opaque nodes — opaque subtrees are atomic
                if opaque_descendants_b.contains(&b_id) {
                    continue;
                }
                let pos = tree_b.position(b_id);
                ops.push(EditOp::Insert {
                    node_b: b_id,
                    parent_b,
                    position: pos,
                    kind: tree_b.kind(b_id).clone(),
                });
            }
            // Root insertion is a special case - usually trees have matching roots
        }
    }

    // Phase 4: MOVE - matched nodes where parent or position changed
    debug!("Phase 4: MOVE - checking {} matched pairs", matching.len());
    for (a_id, b_id) in matching.pairs() {
        debug!(
            a = usize::from(a_id),
            b = usize::from(b_id),
            "checking matched pair for move"
        );
        // Skip descendants of opaque nodes — opaque subtrees are atomic
        if opaque_descendants_a.contains(&a_id) || opaque_descendants_b.contains(&b_id) {
            continue;
        }
        // Skip root
        let Some(parent_a) = tree_a.parent(a_id) else {
            debug!(
                a = usize::from(a_id),
                "skipping: no parent in tree_a (root)"
            );
            continue;
        };
        let Some(parent_b) = tree_b.parent(b_id) else {
            debug!(
                b = usize::from(b_id),
                "skipping: no parent in tree_b (root)"
            );
            continue;
        };

        // Check if parent changed (even if target parent is unmatched/inserted).
        // When a matched node moves to an unmatched parent, we still need to emit the Move
        // so the node can be relocated. The Insert of the parent won't include matched children.
        let parent_match = matching.get_b(parent_a);
        let parent_changed = parent_match != Some(parent_b);

        // Short-circuit: if parent changed, we'll emit Move anyway - skip position check
        // We always need pos_b for the Move operation, but only need pos_a if parent unchanged
        if parent_changed {
            let pos_b = tree_b.position(b_id);
            trace!(
                a = usize::from(a_id),
                b = usize::from(b_id),
                parent_changed = true,
                pos_b,
                "move phase: parent changed, emitting move"
            );
            ops.push(EditOp::Move {
                node_a: a_id,
                node_b: b_id,
                new_parent_b: parent_b,
                new_position: pos_b,
            });
            continue;
        }

        // Parent unchanged - check if position among siblings changed
        let pos_a = tree_a.position(a_id);
        let pos_b = tree_b.position(b_id);
        let position_changed = pos_a != pos_b;

        trace!(
            a = usize::from(a_id),
            b = usize::from(b_id),
            parent_a = usize::from(parent_a),
            parent_b = usize::from(parent_b),
            ?parent_match,
            pos_a,
            pos_b,
            position_changed,
            "move phase: checking position"
        );

        if position_changed {
            ops.push(EditOp::Move {
                node_a: a_id,
                node_b: b_id,
                new_parent_b: parent_b,
                new_position: pos_b,
            });
        }
    }

    // Phase 5: DELETE - nodes in A that are not matched
    // Process in post-order so children are deleted before parents
    for a_id in tree_a.post_order() {
        if !matching.contains_a(a_id) {
            // Skip descendants of opaque nodes — opaque subtrees are atomic
            if opaque_descendants_a.contains(&a_id) {
                continue;
            }
            ops.push(EditOp::Delete { node_a: a_id });
        }
    }

    debug!(total_ops = ops.inner.len(), "generate_edit_script done");
    ops.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matching::MatchingConfig;
    use crate::matching::compute_matching;
    use crate::tree::{NodeData, PropValue, PropertyInFinalState, SimpleTypes, Tree};
    use facet::Facet;
    use facet_testhelpers::test;

    type TestTypes = SimpleTypes<&'static str>;
    type TestTypesStr = SimpleTypes<&'static str>;

    #[test]
    fn test_no_changes() {
        let mut tree_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
        tree_a.add_child(tree_a.root, NodeData::simple_u64(1, "leaf"));

        let mut tree_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
        tree_b.add_child(tree_b.root, NodeData::simple_u64(1, "leaf"));

        let matching = compute_matching(&tree_a, &tree_b, &MatchingConfig::default());
        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        assert!(ops.is_empty(), "Identical trees should have no edits");
    }

    #[test]
    fn test_matched_nodes_different_hash_no_op() {
        // When matched nodes have different hashes but no properties,
        // no edit ops are emitted (structural differences are handled
        // by Insert/Delete/Move on descendants).
        let mut tree_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
        tree_a.add_child(tree_a.root, NodeData::simple_u64(1, "leaf"));

        let mut tree_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
        tree_b.add_child(tree_b.root, NodeData::simple_u64(2, "leaf"));

        let matching = compute_matching(&tree_a, &tree_b, &MatchingConfig::default());
        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        // No ops - nodes are matched, hash differs but no properties to update
        // (The label change is reflected in the hash, but we don't emit Update ops anymore)
        assert!(
            ops.is_empty(),
            "Matched nodes with different hashes but no properties should have no edits, got {:?}",
            ops
        );
    }

    #[test]
    fn test_insert() {
        let tree_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));

        let mut tree_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
        tree_b.add_child(tree_b.root, NodeData::simple_u64(1, "leaf"));

        let matching = compute_matching(&tree_a, &tree_b, &MatchingConfig::default());
        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, EditOp::Insert { .. }))
            .collect();
        assert_eq!(inserts.len(), 1, "Should have one insert operation");
    }

    #[test]
    fn test_delete() {
        let mut tree_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
        tree_a.add_child(tree_a.root, NodeData::simple_u64(1, "leaf"));

        let tree_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));

        let matching = compute_matching(&tree_a, &tree_b, &MatchingConfig::default());
        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        let deletes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, EditOp::Delete { .. }))
            .collect();
        assert_eq!(deletes.len(), 1, "Should have one delete operation");
    }

    #[test]
    fn test_swap_two_siblings() {
        // Tree A: root -> [child_a at pos 0, child_b at pos 1]
        // Tree B: root -> [child_b at pos 0, child_a at pos 1]
        // This tests the swap scenario to understand Move semantics

        // Root hashes must differ (otherwise top-down recursively matches children BY POSITION).
        // With min_height=0, leaves are included in top-down and matched by hash.
        let mut tree_a: Tree<TestTypesStr> = Tree::new(NodeData::simple_u64(100, "root"));
        let child_a = tree_a.add_child(tree_a.root, NodeData::simple_u64(1, "leaf"));
        let child_b = tree_a.add_child(tree_a.root, NodeData::simple_u64(2, "leaf"));

        let mut tree_b: Tree<TestTypesStr> = Tree::new(NodeData::simple_u64(200, "root")); // Different root hash!
        // Swap order: B first, then A
        let child_b2 = tree_b.add_child(tree_b.root, NodeData::simple_u64(2, "leaf"));
        let child_a2 = tree_b.add_child(tree_b.root, NodeData::simple_u64(1, "leaf"));

        let config = MatchingConfig {
            min_height: 0, // Include leaves in top-down matching
            ..Default::default()
        };
        let matching = compute_matching(&tree_a, &tree_b, &config);

        // Debug: print tree structure and matching
        debug!(?tree_a.root, "tree_a root");
        debug!(
            ?child_a,
            hash = %tree_a.get(child_a).hash,
            pos = tree_a.position(child_a),
            "tree_a child_a"
        );
        debug!(
            ?child_b,
            hash = %tree_a.get(child_b).hash,
            pos = tree_a.position(child_b),
            "tree_a child_b"
        );
        debug!(?tree_b.root, "tree_b root");
        debug!(
            ?child_b2,
            hash = %tree_b.get(child_b2).hash,
            pos = tree_b.position(child_b2),
            "tree_b child_b2"
        );
        debug!(
            ?child_a2,
            hash = %tree_b.get(child_a2).hash,
            pos = tree_b.position(child_a2),
            "tree_b child_a2"
        );
        for (a, b) in matching.pairs() {
            debug!(?a, ?b, "matching pair");
        }

        // Verify matching is correct
        assert_eq!(
            matching.get_b(child_a),
            Some(child_a2),
            "child_a should match child_a2"
        );
        assert_eq!(
            matching.get_b(child_b),
            Some(child_b2),
            "child_b should match child_b2"
        );

        // Verify positions in original trees
        assert_eq!(tree_a.position(child_a), 0, "child_a at pos 0 in tree_a");
        assert_eq!(tree_a.position(child_b), 1, "child_b at pos 1 in tree_a");
        assert_eq!(tree_b.position(child_a2), 1, "child_a2 at pos 1 in tree_b");
        assert_eq!(tree_b.position(child_b2), 0, "child_b2 at pos 0 in tree_b");

        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        for op in &ops {
            debug!(?op, "edit script op");
        }

        // Filter move operations
        let moves: Vec<_> = ops
            .iter()
            .filter_map(|op| match op {
                EditOp::Move {
                    node_a,
                    node_b,
                    new_parent_b,
                    new_position,
                } => Some((*node_a, *node_b, *new_parent_b, *new_position)),
                _ => None,
            })
            .collect();

        // Key question: What does cinereus emit for a swap?
        // - Move for child_a: was at pos 0, should be at pos 1
        // - Move for child_b: was at pos 1, should be at pos 0
        //
        // The new_position field comes from tree_b.position(b_id), which is the
        // FINAL position in the target tree, not an intermediate position.

        assert_eq!(moves.len(), 2, "Should have two move operations for a swap");

        // Find move for child_a (hash 1)
        let move_a = moves.iter().find(|(a, _, _, _)| *a == child_a);
        assert!(move_a.is_some(), "Should have move for child_a");
        let (_, _, _, new_pos_a) = move_a.unwrap();
        assert_eq!(*new_pos_a, 1, "child_a should move to position 1");

        // Find move for child_b (hash 2)
        let move_b = moves.iter().find(|(a, _, _, _)| *a == child_b);
        assert!(move_b.is_some(), "Should have move for child_b");
        let (_, _, _, new_pos_b) = move_b.unwrap();
        assert_eq!(*new_pos_b, 0, "child_b should move to position 0");
    }

    /// Test demonstrating the problem with modeling attributes as children.
    ///
    /// When attributes are modeled as child nodes, nodes with identical values
    /// (like Option::None) get cross-matched regardless of their field names.
    ///
    /// Example: `attrs.onscroll: None` matches `attrs.oncontextmenu: None`
    /// because they have the same hash.
    #[test]
    fn test_attribute_cross_matching_problem() {
        // Model a Div element with two None attributes as children
        // This simulates how facet-diff currently builds trees
        //
        // Tree A: Div
        //   ├── id: None (hash = 0, representing Option::None)
        //   └── class: None (hash = 0, same hash!)
        //
        // Tree B: Div
        //   ├── id: "foo" (hash = 123)
        //   └── class: None (hash = 0)
        //
        // CURRENT BEHAVIOR: id:None might match class:None (same hash)
        // DESIRED BEHAVIOR: id:None should match id:"foo" (same field)

        let mut tree_a: Tree<TestTypesStr> = Tree::new(NodeData::simple_u64(100, "div"));
        let id_a = tree_a.add_child(tree_a.root, NodeData::simple_u64(0, "option")); // id: None
        let class_a = tree_a.add_child(tree_a.root, NodeData::simple_u64(0, "option")); // class: None

        let mut tree_b: Tree<TestTypesStr> = Tree::new(NodeData::simple_u64(200, "div"));
        let id_b = tree_b.add_child(tree_b.root, NodeData::simple_u64(123, "option")); // id: "foo"
        let class_b = tree_b.add_child(tree_b.root, NodeData::simple_u64(0, "option")); // class: None

        let config = MatchingConfig {
            min_height: 0,
            ..Default::default()
        };
        let matching = compute_matching(&tree_a, &tree_b, &config);

        // Log what got matched
        debug!("id_a={:?}, class_a={:?}", id_a, class_a);
        debug!("id_b={:?}, class_b={:?}", id_b, class_b);
        for (a, b) in matching.pairs() {
            debug!("matched: {:?} -> {:?}", a, b);
        }

        // CURRENT (BROKEN) BEHAVIOR:
        // - id_a (None) matches class_b (None) because same hash
        // - class_a (None) is orphaned or matches something random
        // - id_b ("foo") is unmatched → Insert
        // - One of the Nones is unmatched → Delete
        //
        // This results in Insert + Delete instead of Update for the id field!

        // Check what actually got matched
        let id_a_match = matching.get_b(id_a);
        let class_a_match = matching.get_b(class_a);

        debug!("id_a matched to: {:?}", id_a_match);
        debug!("class_a matched to: {:?}", class_a_match);

        // The problem: with identical hashes, we can't guarantee correct matching
        // One of these assertions will likely fail or show cross-matching:
        //
        // DESIRED: id_a should match id_b (same logical field)
        // DESIRED: class_a should match class_b (same logical field)
        //
        // But without field name information in the hash, cinereus can't know this.

        // For now, just document that the current behavior is problematic
        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        debug!("Edit ops:");
        for op in &ops {
            debug!("  {:?}", op);
        }

        // Count the ops - with cross-matching we get Insert+Delete
        let inserts = ops
            .iter()
            .filter(|op| matches!(op, EditOp::Insert { .. }))
            .count();
        let deletes = ops
            .iter()
            .filter(|op| matches!(op, EditOp::Delete { .. }))
            .count();

        debug!("inserts={}, deletes={}", inserts, deletes);

        // IDEAL: 0 inserts, 0 deletes (with properties, the id field would get UpdateProperty)
        // ACTUAL: likely 1 insert, 1 delete
        // This test documents the problem - it may pass or fail depending on
        // which None gets matched to which.
    }

    /// Test properties implementation for HTML-like attributes
    #[derive(Debug, Clone, PartialEq, Eq, Facet)]
    struct HtmlAttrs {
        id: Option<String>,
        class: Option<String>,
    }

    impl HtmlAttrs {
        fn new() -> Self {
            Self {
                id: None,
                class: None,
            }
        }

        fn with_id(mut self, id: &str) -> Self {
            self.id = Some(id.to_string());
            self
        }

        fn with_class(mut self, class: &str) -> Self {
            self.class = Some(class.to_string());
            self
        }
    }

    impl Properties for HtmlAttrs {
        type Key = &'static str;
        type Value = String;

        fn similarity(&self, other: &Self) -> f64 {
            let mut matches = 0;
            let mut total = 0;

            // Compare id
            if self.id.is_some() || other.id.is_some() {
                total += 1;
                if self.id == other.id {
                    matches += 1;
                }
            }

            // Compare class
            if self.class.is_some() || other.class.is_some() {
                total += 1;
                if self.class == other.class {
                    matches += 1;
                }
            }

            if total == 0 {
                1.0
            } else {
                matches as f64 / total as f64
            }
        }

        fn diff(&self, other: &Self) -> Vec<PropertyInFinalState<Self::Key, Self::Value>> {
            let mut result = vec![];

            // Always include properties in a consistent order: id, then class
            if let Some(id) = &other.id {
                result.push(PropertyInFinalState {
                    key: "id",
                    value: if self.id.as_ref() == Some(id) {
                        PropValue::Same
                    } else {
                        PropValue::Different(id.clone())
                    },
                });
            }

            if let Some(class) = &other.class {
                result.push(PropertyInFinalState {
                    key: "class",
                    value: if self.class.as_ref() == Some(class) {
                        PropValue::Same
                    } else {
                        PropValue::Different(class.clone())
                    },
                });
            }

            result
        }

        fn is_empty(&self) -> bool {
            self.id.is_none() && self.class.is_none()
        }

        fn len(&self) -> usize {
            (if self.id.is_some() { 1 } else { 0 }) + (if self.class.is_some() { 1 } else { 0 })
        }
    }

    type HtmlTypes = SimpleTypes<&'static str, HtmlAttrs>;

    #[test]
    fn test_properties_emit_update_property_ops() {
        // Tree A: root -> div (id="foo", class=None)
        let mut tree_a: Tree<HtmlTypes> =
            Tree::new(NodeData::element(0.into(), "root", HtmlAttrs::new()));
        let div_a = tree_a.add_child(
            tree_a.root,
            NodeData::element(1.into(), "div", HtmlAttrs::new().with_id("foo")),
        );

        // Tree B: root -> div (id="bar", class="container")
        // Same structure, different properties
        let mut tree_b: Tree<HtmlTypes> =
            Tree::new(NodeData::element(0.into(), "root", HtmlAttrs::new()));
        let div_b = tree_b.add_child(
            tree_b.root,
            NodeData::element(
                2.into(), // Different hash (properties differ)
                "div",
                HtmlAttrs::new().with_id("bar").with_class("container"),
            ),
        );

        // Match trees
        let matching = compute_matching(&tree_a, &tree_b, &MatchingConfig::default());

        // The divs should match (same kind, same position)
        assert!(matching.contains_a(div_a), "div_a should be matched");
        assert_eq!(
            matching.get_b(div_a),
            Some(div_b),
            "div_a should match div_b"
        );

        // Generate edit script
        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        // Should get UpdateProperties op, NOT Insert+Delete
        let update_props_ops: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, EditOp::UpdateProperties { .. }))
            .collect();

        let insert_ops: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, EditOp::Insert { .. }))
            .collect();

        let delete_ops: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, EditOp::Delete { .. }))
            .collect();

        debug!("All ops: {:#?}", ops);

        // We should have 1 UpdateProperties op with 2 changes (id changed, class added)
        assert_eq!(
            update_props_ops.len(),
            1,
            "Expected 1 UpdateProperties op, got {:?}",
            update_props_ops
        );

        // We should NOT have Insert or Delete ops
        assert!(
            insert_ops.is_empty(),
            "Should not have Insert ops, got {:?}",
            insert_ops
        );
        assert!(
            delete_ops.is_empty(),
            "Should not have Delete ops, got {:?}",
            delete_ops
        );

        // Verify the specific property changes
        if let Some(EditOp::UpdateProperties { changes, .. }) = update_props_ops.first() {
            assert_eq!(changes.len(), 2, "Should have 2 property changes");

            let id_change = changes.iter().find(|c| c.key == "id");
            assert!(id_change.is_some(), "Should have change for 'id'");
            if let Some(change) = id_change {
                assert_eq!(change.value, PropValue::Different("bar".to_string()));
            }

            let class_change = changes.iter().find(|c| c.key == "class");
            assert!(class_change.is_some(), "Should have change for 'class'");
            if let Some(change) = class_change {
                assert_eq!(change.value, PropValue::Different("container".to_string()));
            }
        }
    }

    #[test]
    fn test_properties_no_cross_matching() {
        // This test verifies that we don't have the cross-matching problem
        // when properties are NOT tree children.
        //
        // The old approach modeled attributes as tree children:
        //   div -> [id: None, class: None, onclick: None, ...]
        //
        // This caused None values to cross-match (they all hash the same).
        //
        // With properties, each attribute stays with its key, so
        // id=None in tree_a maps to id=Some("x") in tree_b correctly.

        // Tree A: root -> div (id=None, class=None)
        let mut tree_a: Tree<HtmlTypes> =
            Tree::new(NodeData::element(0.into(), "root", HtmlAttrs::new()));
        let _div_a = tree_a.add_child(
            tree_a.root,
            NodeData::element(1.into(), "div", HtmlAttrs::new()), // Both None
        );

        // Tree B: root -> div (id="myid", class=None)
        let mut tree_b: Tree<HtmlTypes> =
            Tree::new(NodeData::element(0.into(), "root", HtmlAttrs::new()));
        let _div_b = tree_b.add_child(
            tree_b.root,
            NodeData::element(2.into(), "div", HtmlAttrs::new().with_id("myid")),
        );

        let matching = compute_matching(&tree_a, &tree_b, &MatchingConfig::default());
        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        // Should get exactly 1 UpdateProperties op with 1 change for id
        let update_props_ops: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, EditOp::UpdateProperties { .. }))
            .collect();

        assert_eq!(
            update_props_ops.len(),
            1,
            "Expected 1 UpdateProperties op, got {:?}",
            update_props_ops
        );

        // Verify only id changed, not class
        if let Some(EditOp::UpdateProperties { changes, .. }) = update_props_ops.first() {
            assert_eq!(changes.len(), 1, "Should have 1 property change (id only)");
            assert_eq!(changes[0].key, "id", "The change should be for 'id'");
        }
    }

    #[test]
    #[ignore] // TODO: Need to create proper test with Properties that differentiate text content
    fn test_move_from_deleted_parent() {
        // Test case: child node matched, but its parent is unmatched (will be deleted).
        // The child should be MOVEd to its new location.
        //
        // This matches the HTML fuzzer scenario:
        // Old: strong#1(text "old"), strong#2(text "foo")
        // New: strong(text "foo")
        // Where strong#1 matches new strong, and text "foo" from strong#2 should move to it.
        //
        // Old tree:
        // root
        //   ├─ elem1 (matched)
        //   │   └─ text "old"
        //   └─ elem2 (unmatched - will be deleted)
        //       └─ text "foo" (matched - should move to elem1)
        //
        // New tree:
        // root
        //   └─ elem1 (matched)
        //       └─ text "foo" (matched)

        let mut tree_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(1, "root"));
        let elem1 = tree_a.add_child(tree_a.root, NodeData::simple_u64(2, "elem"));
        let _text_old = tree_a.add_child(elem1, NodeData::simple_u64(3, "text"));
        let elem2 = tree_a.add_child(tree_a.root, NodeData::simple_u64(4, "elem"));
        let text_foo = tree_a.add_child(elem2, NodeData::simple_u64(5, "text"));

        let mut tree_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(10, "root"));
        let elem1_b = tree_b.add_child(tree_b.root, NodeData::simple_u64(20, "elem"));
        let _text_foo_b = tree_b.add_child(elem1_b, NodeData::simple_u64(30, "text"));

        let config = MatchingConfig {
            min_height: 0, // Include leaves in matching
            ..MatchingConfig::default()
        };
        let matching = compute_matching(&tree_a, &tree_b, &config);
        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        debug!(?ops, "Generated operations");

        // Check if there's a Move operation for text_foo
        let move_ops: Vec<_> = ops
            .iter()
            .filter_map(|op| {
                if let EditOp::Move { node_a, .. } = op {
                    Some(node_a)
                } else {
                    None
                }
            })
            .collect();

        debug!(?move_ops, text_foo_id = ?text_foo, "Move operations");

        let has_move = move_ops.iter().any(|&&node_a| node_a == text_foo);

        assert!(
            has_move,
            "Expected Move operation for text node moving from deleted parent.\nOps: {:#?}",
            ops
        );
    }

    /// Wrapper around Tree that marks specific nodes as opaque.
    struct OpaqueTree<T: TreeTypes> {
        inner: Tree<T>,
        opaque_nodes: std::collections::HashSet<NodeId>,
    }

    impl<T: TreeTypes> OpaqueTree<T> {
        fn new(inner: Tree<T>) -> Self {
            Self {
                inner,
                opaque_nodes: std::collections::HashSet::default(),
            }
        }

        fn mark_opaque(&mut self, id: NodeId) {
            self.opaque_nodes.insert(id);
        }
    }

    impl<T: TreeTypes> crate::tree::DiffTree for OpaqueTree<T> {
        type Types = T;

        fn root(&self) -> NodeId {
            self.inner.root()
        }

        fn node_count(&self) -> usize {
            self.inner.node_count()
        }

        fn hash(&self, id: NodeId) -> crate::tree::NodeHash {
            self.inner.hash(id)
        }

        fn kind(&self, id: NodeId) -> &T::Kind {
            self.inner.kind(id)
        }

        fn properties(&self, id: NodeId) -> &T::Props {
            self.inner.properties(id)
        }

        fn text(&self, id: NodeId) -> Option<&T::Text> {
            self.inner.text(id)
        }

        fn parent(&self, id: NodeId) -> Option<NodeId> {
            self.inner.parent(id)
        }

        fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
            self.inner.children(id)
        }

        fn child_count(&self, id: NodeId) -> usize {
            self.inner.child_count(id)
        }

        fn position(&self, id: NodeId) -> usize {
            self.inner.position(id)
        }

        fn height(&self, id: NodeId) -> usize {
            self.inner.height(id)
        }

        fn iter(&self) -> impl Iterator<Item = NodeId> + '_ {
            self.inner.iter()
        }

        fn post_order(&self) -> impl Iterator<Item = NodeId> + '_ {
            self.inner.post_order()
        }

        fn descendants(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
            self.inner.descendants(id)
        }

        fn is_opaque(&self, id: NodeId) -> bool {
            self.opaque_nodes.contains(&id)
        }
    }

    #[test]
    fn test_opaque_no_edits_for_children() {
        // Tree A: root -> opaque_div -> [leaf_a]
        // Tree B: root -> opaque_div -> [leaf_b]  (different child)
        //
        // Even though children differ, the edit script should not contain
        // Insert/Delete/Move for children of opaque nodes.
        let mut raw_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
        let opaque_a = raw_a.add_child(raw_a.root, NodeData::simple_u64(10, "div"));
        raw_a.add_child(opaque_a, NodeData::simple_u64(1, "text"));

        let mut tree_a = OpaqueTree::new(raw_a);
        tree_a.mark_opaque(opaque_a);

        let mut raw_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(200, "root"));
        let opaque_b = raw_b.add_child(raw_b.root, NodeData::simple_u64(20, "div"));
        raw_b.add_child(opaque_b, NodeData::simple_u64(2, "text"));

        let mut tree_b = OpaqueTree::new(raw_b);
        tree_b.mark_opaque(opaque_b);

        let matching = compute_matching(&tree_a, &tree_b, &MatchingConfig::default());
        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        // Should have NO Insert, Delete, or Move operations for opaque children
        let structural_ops: Vec<_> = ops
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    EditOp::Insert { .. } | EditOp::Delete { .. } | EditOp::Move { .. }
                )
            })
            .collect();

        assert!(
            structural_ops.is_empty(),
            "Edit script should not contain structural ops for opaque children, got: {:?}",
            structural_ops
        );
    }

    #[test]
    fn test_opaque_deeply_nested_no_edits() {
        // Tree A: root -> opaque_div -> wrapper -> [deep1, deep2]
        // Tree B: root -> opaque_div -> wrapper -> [deep3]
        //
        // Deeply nested children should also be skipped.
        let mut raw_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
        let opaque_a = raw_a.add_child(raw_a.root, NodeData::simple_u64(10, "div"));
        let wrapper_a = raw_a.add_child(opaque_a, NodeData::simple_u64(5, "span"));
        raw_a.add_child(wrapper_a, NodeData::simple_u64(1, "text"));
        raw_a.add_child(wrapper_a, NodeData::simple_u64(2, "text"));

        let mut tree_a = OpaqueTree::new(raw_a);
        tree_a.mark_opaque(opaque_a);

        let mut raw_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(200, "root"));
        let opaque_b = raw_b.add_child(raw_b.root, NodeData::simple_u64(20, "div"));
        let wrapper_b = raw_b.add_child(opaque_b, NodeData::simple_u64(6, "span"));
        raw_b.add_child(wrapper_b, NodeData::simple_u64(3, "text"));

        let mut tree_b = OpaqueTree::new(raw_b);
        tree_b.mark_opaque(opaque_b);

        let matching = compute_matching(&tree_a, &tree_b, &MatchingConfig::default());
        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        let structural_ops: Vec<_> = ops
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    EditOp::Insert { .. } | EditOp::Delete { .. } | EditOp::Move { .. }
                )
            })
            .collect();

        assert!(
            structural_ops.is_empty(),
            "Deeply nested opaque children should not produce structural ops, got: {:?}",
            structural_ops
        );
    }

    #[test]
    fn test_opaque_sibling_still_edited() {
        // Tree A: root -> [opaque_div -> [old_text], normal_span]
        // Tree B: root -> [opaque_div -> [new_text], normal_span, new_leaf]
        //
        // The opaque div's children change should produce no ops,
        // but the new_leaf sibling should produce an Insert.
        let mut raw_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
        let opaque_a = raw_a.add_child(raw_a.root, NodeData::simple_u64(10, "div"));
        raw_a.add_child(opaque_a, NodeData::simple_u64(1, "text"));
        raw_a.add_child(raw_a.root, NodeData::simple_u64(20, "span"));

        let mut tree_a = OpaqueTree::new(raw_a);
        tree_a.mark_opaque(opaque_a);

        let mut raw_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(200, "root"));
        let opaque_b = raw_b.add_child(raw_b.root, NodeData::simple_u64(20, "div"));
        raw_b.add_child(opaque_b, NodeData::simple_u64(2, "text"));
        raw_b.add_child(raw_b.root, NodeData::simple_u64(20, "span"));
        raw_b.add_child(raw_b.root, NodeData::simple_u64(30, "p"));

        let mut tree_b = OpaqueTree::new(raw_b);
        tree_b.mark_opaque(opaque_b);

        let matching = compute_matching(&tree_a, &tree_b, &MatchingConfig::default());
        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        // Should have an Insert for the new <p> sibling
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, EditOp::Insert { .. }))
            .collect();

        assert_eq!(
            inserts.len(),
            1,
            "Should have exactly one insert for the new sibling, got: {:?}",
            ops
        );
    }

    #[test]
    fn test_opaque_identical_no_ops() {
        // When opaque nodes are identical (same hash), no ops at all.
        let mut raw_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
        let opaque_a = raw_a.add_child(raw_a.root, NodeData::simple_u64(10, "div"));
        raw_a.add_child(opaque_a, NodeData::simple_u64(1, "text"));

        let mut tree_a = OpaqueTree::new(raw_a);
        tree_a.mark_opaque(opaque_a);

        let mut raw_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
        let opaque_b = raw_b.add_child(raw_b.root, NodeData::simple_u64(10, "div"));
        raw_b.add_child(opaque_b, NodeData::simple_u64(1, "text"));

        let mut tree_b = OpaqueTree::new(raw_b);
        tree_b.mark_opaque(opaque_b);

        let matching = compute_matching(&tree_a, &tree_b, &MatchingConfig::default());
        let ops = generate_edit_script(&tree_a, &tree_b, &matching);

        assert!(
            ops.is_empty(),
            "Identical opaque trees should produce no edit ops, got: {:?}",
            ops
        );
    }
}
