//! HTML diffing with DOM patch generation.
//!
//! This module uses cinereus (GumTree/Chawathe) to compute tree diffs and translates
//! them into DOM patches that can be applied to update an HTML document incrementally.

use crate::{
    Stem, debug,
    dom::{self, Document, Namespace, NodeKind},
};
use cinereus::{
    DiffTree, EditOp, Matching, MatchingConfig, NodeData, NodeHash, PropValue, Properties,
    PropertyInFinalState, Tree, TreeTypes,
    indextree::{self, NodeId},
};
use facet::Facet;
use html5ever::{LocalName, QualName};
use rapidhash::RapidHasher;
use smallvec::SmallVec;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

#[allow(unused_imports)]
use crate::trace;

/// Proxy for LocalName serialization
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[facet(transparent)]
pub struct LocalNameProxy(pub String);

impl From<LocalNameProxy> for LocalName {
    fn from(proxy: LocalNameProxy) -> Self {
        LocalName::from(proxy.0)
    }
}

impl From<&LocalName> for LocalNameProxy {
    fn from(local: &LocalName) -> Self {
        LocalNameProxy(local.to_string())
    }
}

/// Proxy for QualName serialization (prefix, namespace, local_name)
/// TODO: This string conversion is inefficient - consider interning namespaces or using indices
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct QualNameProxy {
    pub prefix: Option<String>,
    pub ns: String,
    pub local: String,
}

impl From<QualNameProxy> for QualName {
    fn from(proxy: QualNameProxy) -> Self {
        use html5ever::{Namespace, Prefix};
        QualName {
            prefix: proxy.prefix.map(Prefix::from),
            ns: Namespace::from(proxy.ns),
            local: LocalName::from(proxy.local),
        }
    }
}

impl From<&QualName> for QualNameProxy {
    fn from(qual: &QualName) -> Self {
        QualNameProxy {
            prefix: qual.prefix.as_ref().map(|p| p.to_string()),
            ns: qual.ns.to_string(),
            local: qual.local.to_string(),
        }
    }
}

/// An attribute name-value pair
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct AttrPair {
    #[facet(opaque, proxy = QualNameProxy)]
    pub name: QualName,
    pub value: Stem,
}

impl From<(QualName, Stem)> for AttrPair {
    fn from((name, value): (QualName, Stem)) -> Self {
        AttrPair { name, value }
    }
}

impl From<AttrPair> for (QualName, Stem) {
    fn from(pair: AttrPair) -> Self {
        (pair.name, pair.value)
    }
}

/// Property key - either text content or an attribute
#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum PropKey {
    /// Text content
    Text,
    /// Attribute with qualified name
    Attr(#[facet(opaque, proxy = QualNameProxy)] QualName),
}

impl std::fmt::Display for PropKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PropKey::Text => write!(f, "_text"),
            PropKey::Attr(qual) => {
                if let Some(ref prefix) = qual.prefix {
                    write!(f, "{}:", prefix)?;
                }
                write!(f, "{}", qual.local)
            }
        }
    }
}

/// Errors that can occur during diffing or patch application.
#[derive(Facet, Debug)]
#[facet(derive(Error))]
#[repr(u8)]
pub enum DiffError {
    /// no body element found in document
    NoBody,

    /// path index {index} out of bounds
    PathOutOfBounds { index: usize },

    /// cannot get parent of empty path
    EmptyPath,

    /// slot {slot} not found
    SlotNotFound { slot: u32 },

    /// cannot insert at slot without relative path
    SlotMissingRelativePath,

    /// node is not a text node
    NotATextNode,

    /// node is not an element
    NotAnElement,

    /// node is not a comment
    NotAComment,
}

/// A path to a node in the DOM tree.
///
/// Uses SmallVec<[u32; 16]> to avoid heap allocations for typical DOM depths.
/// Most HTML documents have paths shorter than 16 elements, and u32 is plenty
/// for child indices (no element has billions of children).
#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
#[facet(transparent)]
pub struct NodePath(pub SmallVec<[u32; 16]>);

impl std::fmt::Display for NodePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, idx) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{}", idx)?;
        }
        Ok(())
    }
}

/// Reference to a node via a path.
///
/// The first element of the path is always the slot number:
/// - `[0, 1, 2]` = slot 0 (main tree), child 1, child 2
/// - `[1, 0, 3]` = slot 1 (displaced content), child 0, child 3
///
/// Slot 0 is special - it always contains the original tree (body).
/// Slots 1+ are created when content is displaced during Insert/Move operations.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
#[facet(transparent)]
pub struct NodeRef(pub NodePath);

/// Content that can be inserted as part of a new subtree.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum InsertContent {
    /// An element with its tag, attributes, and nested children
    Element {
        #[facet(opaque, proxy = LocalNameProxy)]
        tag: LocalName,
        attrs: Vec<AttrPair>,
        children: Vec<InsertContent>,
    },
    /// A text node
    Text(Stem),
    /// A comment node
    Comment(Stem),
}

/// A property in the final state within an UpdateProps operation.
/// The vec position determines the final ordering.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct PropChange {
    /// The property key (Text or Attr(QualName))
    pub name: PropKey,
    /// The value: None means "keep existing value", Some means "update to this value".
    /// Properties not in the list are implicitly removed.
    pub value: Option<Stem>,
}

/// Operations to transform the DOM.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum Patch {
    /// Insert an element at a position.
    /// The `at` NodeRef path includes the position as the last segment.
    /// Path `[slot, a, b, c]` means: in slot, navigate to a, then b, then insert at position c.
    InsertElement {
        at: NodeRef,
        #[facet(opaque, proxy = LocalNameProxy)]
        tag: LocalName,
        attrs: Vec<AttrPair>,
        children: Vec<InsertContent>,
        detach_to_slot: Option<u32>,
    },

    /// Insert a text node at a position.
    InsertText {
        at: NodeRef,
        text: Stem,
        detach_to_slot: Option<u32>,
    },

    /// Insert a comment node at a position.
    InsertComment {
        at: NodeRef,
        text: Stem,
        detach_to_slot: Option<u32>,
    },

    /// Remove a node
    Remove { node: NodeRef },

    /// Update text content of a text node at path.
    SetText { path: NodePath, text: Stem },

    /// Set attribute on element at path
    SetAttribute {
        path: NodePath,
        #[facet(opaque, proxy = QualNameProxy)]
        name: QualName,
        value: Stem,
    },

    /// Remove attribute from element at path
    RemoveAttribute {
        path: NodePath,
        #[facet(opaque, proxy = QualNameProxy)]
        name: QualName,
    },

    /// Move a node from one location to another.
    Move {
        from: NodeRef,
        to: NodeRef,
        detach_to_slot: Option<u32>,
    },

    /// Update multiple properties on an element.
    UpdateProps {
        path: NodePath,
        changes: Vec<PropChange>,
    },
}

/// Node kind in the HTML tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HtmlNodeKind {
    /// An element node with a tag name and namespace
    /// LocalName is interned via string_cache, Namespace distinguishes HTML/SVG/MathML
    Element(LocalName, Namespace),
    /// A text node
    Text,
    /// A comment node
    Comment,
}

impl std::fmt::Display for HtmlNodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HtmlNodeKind::Element(tag, ns) => match ns {
                Namespace::Html => write!(f, "<{}>", tag),
                Namespace::Svg => write!(f, "<svg:{}>", tag),
                Namespace::MathMl => write!(f, "<math:{}>", tag),
            },
            HtmlNodeKind::Text => write!(f, "#text"),
            HtmlNodeKind::Comment => write!(f, "#comment"),
        }
    }
}

/// HTML element properties (attributes + text content).
#[derive(Debug, Clone, Default)]
pub struct HtmlProps {
    /// Attributes - Vec preserves insertion order for consistent serialization
    /// Keys are QualName (preserves namespace for xlink:href, xml:lang, etc.)
    pub attrs: Vec<(QualName, Stem)>,

    /// Text content (atomic Tendril is refcounted + Sync - cheap to clone)
    pub text: Option<Stem>,
}

impl Properties for HtmlProps {
    type Key = PropKey;
    type Value = Stem;

    #[allow(clippy::mutable_key_type)]
    fn similarity(&self, other: &Self) -> f64 {
        // Text nodes: compare text content
        if let (Some(t1), Some(t2)) = (&self.text, &other.text) {
            return if t1 == t2 { 1.0 } else { 0.0 };
        }

        // Element nodes: compare attributes using Dice coefficient
        if self.attrs.is_empty() && other.attrs.is_empty() {
            return 1.0;
        }

        let self_keys: std::collections::HashSet<_> = self.attrs.iter().map(|(k, _)| k).collect();
        let other_keys: std::collections::HashSet<_> = other.attrs.iter().map(|(k, _)| k).collect();

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
                key: PropKey::Text,
                value: if self.text.as_ref() == Some(text) {
                    PropValue::Same
                } else {
                    PropValue::Different(text.clone())
                },
            });
        }

        // Include all attributes from the final state in order
        for (key, new_val) in &other.attrs {
            let old_val = self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v);
            result.push(PropertyInFinalState {
                key: PropKey::Attr(key.clone()),
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

    fn len(&self) -> usize {
        self.attrs.len()
    }
}

/// Tree types marker for HTML DOM.
pub struct HtmlTreeTypes;

impl TreeTypes for HtmlTreeTypes {
    type Kind = HtmlNodeKind;
    type Props = HtmlProps;
}

/// Pre-computed diff data for a node.
struct DiffNodeData {
    hash: NodeHash,
    kind: HtmlNodeKind,
    props: HtmlProps,
    height: usize,
    /// Cached position among siblings (0-indexed), computed on-demand
    position: Cell<Option<u32>>,
}

/// A wrapper around Document that implements DiffTree.
///
/// This allows diffing Documents directly without building a separate cinereus Tree.
/// The wrapper pre-computes hashes and caches kind/props for each node.
pub struct DiffableDocument<'a> {
    doc: &'a Document,
    /// The root for diffing (body element)
    body_id: NodeId,
    /// Pre-computed diff data indexed by NodeId
    nodes: HashMap<NodeId, DiffNodeData>,
}

impl<'a> DiffableDocument<'a> {
    /// Create a new DiffableDocument from a Document.
    ///
    /// Pre-computes hashes and caches kind/props for all body descendants.
    pub fn new(doc: &'a Document) -> Self {
        let body_id = doc.body().expect("document must have body");
        // Pre-allocate based on arena size (upper bound for descendants)
        let mut nodes = HashMap::with_capacity(doc.arena.count());

        // First pass: compute kind, props for all nodes
        for node_id in body_id.descendants(&doc.arena) {
            let node = doc.get(node_id);
            let (kind, props) = match &node.kind {
                NodeKind::Element(elem) => {
                    let kind = HtmlNodeKind::Element(elem.tag.clone(), node.ns);
                    let props = HtmlProps {
                        attrs: elem
                            .attrs
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                        text: None,
                    };
                    (kind, props)
                }
                NodeKind::Text(text) => {
                    let kind = HtmlNodeKind::Text;
                    let props = HtmlProps {
                        attrs: Vec::new(),
                        text: Some(text.clone()),
                    };
                    (kind, props)
                }
                NodeKind::Comment(text) => {
                    let kind = HtmlNodeKind::Comment;
                    let props = HtmlProps {
                        attrs: Vec::new(),
                        text: Some(text.clone()),
                    };
                    (kind, props)
                }
                NodeKind::Document => continue, // Skip document nodes
            };

            nodes.insert(
                node_id,
                DiffNodeData {
                    hash: NodeHash(0), // Will be computed in second pass
                    kind,
                    props,
                    height: 0,                 // Will be computed in second pass
                    position: Cell::new(None), // Computed on-demand
                },
            );
        }

        // Second pass: compute heights and hashes bottom-up (post-order)
        // We collect updates separately to avoid borrow conflicts
        let post_order: Vec<_> = PostOrderIterator::new(body_id, &doc.arena).collect();
        for node_id in post_order {
            let children: Vec<_> = node_id.children(&doc.arena).collect();

            // Compute height from children (already processed in post-order)
            let height = if children.is_empty() {
                0
            } else {
                1 + children
                    .iter()
                    .filter_map(|&c| nodes.get(&c))
                    .map(|d| d.height)
                    .max()
                    .unwrap_or(0)
            };

            // Compute hash (Merkle-tree style)
            let hash = if let Some(data) = nodes.get(&node_id) {
                let mut hasher = RapidHasher::default();
                data.kind.hash(&mut hasher);
                for child_id in &children {
                    if let Some(child_data) = nodes.get(child_id) {
                        child_data.hash.0.hash(&mut hasher);
                    }
                }
                NodeHash(hasher.finish())
            } else {
                NodeHash(0)
            };

            // Now update
            if let Some(data) = nodes.get_mut(&node_id) {
                data.height = height;
                data.hash = hash;
            }
        }

        Self {
            doc,
            body_id,
            nodes,
        }
    }
}

impl DiffTree for DiffableDocument<'_> {
    type Types = HtmlTreeTypes;

    fn root(&self) -> NodeId {
        self.body_id
    }

    fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn hash(&self, id: NodeId) -> NodeHash {
        self.nodes.get(&id).map(|d| d.hash).unwrap_or_default()
    }

    fn kind(&self, id: NodeId) -> &HtmlNodeKind {
        self.nodes
            .get(&id)
            .map(|d| &d.kind)
            .expect("node must exist")
    }

    fn properties(&self, id: NodeId) -> &HtmlProps {
        self.nodes
            .get(&id)
            .map(|d| &d.props)
            .expect("node must exist")
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.doc.arena.get(id).and_then(|n| n.parent())
    }

    fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        id.children(&self.doc.arena)
    }

    fn child_count(&self, id: NodeId) -> usize {
        id.children(&self.doc.arena).count()
    }

    fn position(&self, id: NodeId) -> usize {
        if let Some(data) = self.nodes.get(&id) {
            // Check cache first
            if let Some(pos) = data.position.get() {
                return pos as usize;
            }
            // Compute and cache
            let pos = if let Some(parent) = self.parent(id) {
                parent
                    .children(&self.doc.arena)
                    .position(|c| c == id)
                    .unwrap_or(0) as u32
            } else {
                0
            };
            data.position.set(Some(pos));
            pos as usize
        } else {
            0
        }
    }

    fn height(&self, id: NodeId) -> usize {
        self.nodes.get(&id).map(|d| d.height).unwrap_or(0)
    }

    fn iter(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.body_id.descendants(&self.doc.arena)
    }

    fn post_order(&self) -> impl Iterator<Item = NodeId> + '_ {
        PostOrderIterator::new(self.body_id, &self.doc.arena)
    }

    fn descendants(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        id.descendants(&self.doc.arena)
    }
}

/// Post-order iterator over tree nodes.
struct PostOrderIterator<'a> {
    arena: &'a indextree::Arena<dom::NodeData>,
    stack: Vec<(NodeId, bool)>,
}

impl<'a> PostOrderIterator<'a> {
    fn new(root: NodeId, arena: &'a indextree::Arena<dom::NodeData>) -> Self {
        Self {
            arena,
            stack: vec![(root, false)],
        }
    }
}

impl Iterator for PostOrderIterator<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((id, children_visited)) = self.stack.pop() {
            if children_visited {
                return Some(id);
            }
            self.stack.push((id, true));
            let children: Vec<_> = id.children(self.arena).collect();
            for child in children.into_iter().rev() {
                self.stack.push((child, false));
            }
        }
        None
    }
}

/// Build a cinereus tree from an arena_dom::Document (body content only).
pub fn build_tree_from_arena(doc: &Document) -> Tree<HtmlTreeTypes> {
    // Find body element
    let body_id = doc.body().expect("document must have body");
    let body_node = doc.get(body_id);

    // Create root as body element
    let (body_tag, body_ns) = if let NodeKind::Element(elem) = &body_node.kind {
        (elem.tag.clone(), body_node.ns)
    } else {
        panic!("body must be an element");
    };

    let root_data = NodeData {
        hash: NodeHash(0),
        kind: HtmlNodeKind::Element(body_tag, body_ns),
        properties: HtmlProps {
            attrs: Vec::new(),
            text: None,
        },
    };

    let mut tree = Tree::new(root_data);
    let tree_root = tree.root;

    // Add children from body
    add_arena_children(&mut tree, tree_root, doc, body_id);

    // Recompute hashes bottom-up
    recompute_hashes(&mut tree);

    tree
}

fn add_arena_children(
    tree: &mut Tree<HtmlTreeTypes>,
    parent: indextree::NodeId,
    doc: &Document,
    arena_parent: indextree::NodeId,
) {
    let children: Vec<_> = arena_parent.children(&doc.arena).collect();

    for child_id in children.into_iter() {
        let child_node = doc.get(child_id);
        match &child_node.kind {
            NodeKind::Element(elem) => {
                let kind = HtmlNodeKind::Element(elem.tag.clone(), child_node.ns);
                let props = HtmlProps {
                    attrs: elem
                        .attrs
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    text: None,
                };
                let data = NodeData {
                    hash: NodeHash(0),
                    kind,
                    properties: props,
                };
                let node_id = tree.add_child(parent, data);
                add_arena_children(tree, node_id, doc, child_id);
            }
            NodeKind::Text(text) => {
                let kind = HtmlNodeKind::Text;
                let props = HtmlProps {
                    attrs: Vec::new(),
                    text: Some(text.clone()),
                };
                let data = NodeData {
                    hash: NodeHash(0),
                    kind,
                    properties: props,
                };
                tree.add_child(parent, data);
            }
            NodeKind::Comment(text) => {
                let kind = HtmlNodeKind::Comment;
                let props = HtmlProps {
                    attrs: Vec::new(),
                    text: Some(text.clone()),
                };
                let data = NodeData {
                    hash: NodeHash(0),
                    kind,
                    properties: props,
                };
                tree.add_child(parent, data);
            }
            NodeKind::Document => {
                // Skip document nodes
            }
        }
    }
}

/// Recompute hashes for all nodes in bottom-up order.
///
/// IMPORTANT: Properties (attributes, text content) are NOT included in the hash.
/// The hash only captures the structural identity: node kind + children structure.
/// Properties are compared separately via the Properties trait after matching.
/// FIXME: I'm not sure that's such a good idea — amos
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

/// Diff two HTML strings and return DOM patches.
///
/// Parses both HTML strings and diffs them.
pub fn diff_html(old_html: &str, new_html: &str) -> Result<Vec<Patch>, DiffError> {
    let old_doc = dom::parse(old_html);
    let new_doc = dom::parse(new_html);
    diff(&old_doc, &new_doc)
}

/// Diff two arena documents and return DOM patches.
///
/// This is the primary diffing API for arena_dom documents.
pub fn diff(old: &Document, new: &Document) -> Result<Vec<Patch>, DiffError> {
    // Build cinereus Tree for old (needed for shadow tree mutation)
    let tree_a = build_tree_from_arena(old);
    // Use DiffableDocument for new (avoids second tree allocation)
    let diff_b = DiffableDocument::new(new);

    #[cfg(test)]
    {
        trace!(
            "tree_a: root hash={:?}, kind={:?}",
            tree_a.get(tree_a.root).hash,
            tree_a.get(tree_a.root).kind
        );
        trace!(
            "diff_b: root hash={:?}, kind={:?}",
            diff_b.hash(diff_b.root()),
            diff_b.kind(diff_b.root())
        );
    }

    let config = MatchingConfig {
        min_height: 0,
        ..MatchingConfig::default()
    };

    let mut matching = cinereus::compute_matching(&tree_a, &diff_b, &config);

    // Force root match if same tag
    let root_a_kind = tree_a.get(tree_a.root).kind.clone();
    let root_b_kind = diff_b.kind(diff_b.root()).clone();
    if root_a_kind == root_b_kind && !matching.contains_a(tree_a.root) {
        matching.add(tree_a.root, diff_b.root());
    }

    let edit_ops = cinereus::generate_edit_script(&tree_a, &diff_b, &matching);

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

    convert_ops_with_shadow(edit_ops, &tree_a, &diff_b, &matching)
}

/// Encapsulates the shadow tree with slot-based path computation.
///
/// The arena has a "super root" whose children are slot nodes:
/// - Slot 0: The original tree (body element)
/// - Slot 1+: Displaced content from Insert/Move operations
///
/// All paths start with the slot number: [0, 1, 2] means slot 0, child 1, child 2.
/// This eliminates the need for separate tracking of detached nodes.
struct ShadowTree {
    arena: indextree::Arena<NodeData<HtmlTreeTypes>>,
    /// The super root - its children are slot nodes
    super_root: NodeId,
    /// Number of slots (slot 0 always exists, created in new())
    next_slot: u32,
}

impl ShadowTree {
    fn new(mut arena: indextree::Arena<NodeData<HtmlTreeTypes>>, original_root: NodeId) -> Self {
        // Create the super root (a meta node, not a real DOM node)
        let super_root = arena.new_node(NodeData {
            hash: NodeHash(0),
            kind: HtmlNodeKind::Comment, // Just a marker, not used
            properties: HtmlProps::default(),
        });

        // Create slot 0 and reparent the original tree under it
        let slot0 = arena.new_node(NodeData {
            hash: NodeHash(0),
            kind: HtmlNodeKind::Comment, // Slot marker
            properties: HtmlProps::default(),
        });
        super_root.append(slot0, &mut arena);

        // Move original_root under slot 0
        original_root.detach(&mut arena);
        slot0.append(original_root, &mut arena);

        Self {
            arena,
            super_root,
            next_slot: 1, // Slot 0 already exists
        }
    }

    /// Get the slot node for a given slot number.
    fn get_slot(&self, slot: u32) -> Option<NodeId> {
        self.super_root.children(&self.arena).nth(slot as usize)
    }

    /// Get the content root for slot 0 (the original tree root, e.g., body).
    fn slot0_content(&self) -> NodeId {
        let slot0 = self.get_slot(0).expect("slot 0 must exist");
        slot0
            .children(&self.arena)
            .next()
            .expect("slot 0 must have content")
    }

    /// Compute the full path for any node. The first element is always the slot number.
    ///
    /// For a node at super_root → slot0 → body → div → text, the path is `[0, 0, 0]`:
    /// - slot number 0
    /// - div is child 0 of body (the slot content root)
    /// - text is child 0 of div
    ///
    /// Note: The slot content root's position within the slot node is NOT included.
    fn compute_path(&self, node: NodeId) -> SmallVec<[u32; 16]> {
        let mut path = SmallVec::new();
        let mut current = node;

        while let Some(parent_id) = self.arena.get(current).and_then(|n| n.parent()) {
            // Check if parent is a slot node (grandparent is super_root)
            let grandparent = self.arena.get(parent_id).and_then(|n| n.parent());
            if grandparent == Some(self.super_root) {
                // parent_id is a slot node, current is the slot content root (e.g., body)
                // Get the slot number and stop - don't include current's position
                let slot = self
                    .super_root
                    .children(&self.arena)
                    .position(|c| c == parent_id)
                    .unwrap_or(0) as u32;
                path.push(slot);
                break;
            }

            // Normal case: add position and continue up
            let position = parent_id
                .children(&self.arena)
                .position(|c| c == current)
                .unwrap_or(0) as u32;
            path.push(position);
            current = parent_id;
        }

        path.reverse();
        path
    }

    /// Get a NodeRef for any node.
    fn get_node_ref(&self, node: NodeId) -> NodeRef {
        let path = self.compute_path(node);
        debug!(?node, ?path, "get_node_ref: computed path");
        NodeRef(NodePath(path))
    }

    /// Get a NodeRef with a specific position appended (for insert/move targets).
    fn get_node_ref_with_position(&self, parent: NodeId, position: usize) -> NodeRef {
        let mut path = self.compute_path(parent);
        path.push(position as u32);
        NodeRef(NodePath(path))
    }

    /// Create a new slot and return its number.
    fn create_slot(&mut self) -> u32 {
        let slot_num = self.next_slot;
        self.next_slot += 1;

        let slot_node = self.arena.new_node(NodeData {
            hash: NodeHash(0),
            kind: HtmlNodeKind::Comment, // Slot marker
            properties: HtmlProps::default(),
        });
        self.super_root.append(slot_node, &mut self.arena);

        debug!(slot_num, "created new slot");
        slot_num
    }

    /// Detach a node to a new slot, returning the slot number.
    fn detach_to_slot(&mut self, node: NodeId) -> u32 {
        let slot_num = self.create_slot();
        let slot_node = self.get_slot(slot_num).expect("just created");

        node.detach(&mut self.arena);
        slot_node.append(node, &mut self.arena);

        debug!(?node, slot_num, "detached node to slot");
        slot_num
    }

    /// Detach a node with a placeholder to prevent sibling shifts.
    fn detach_with_placeholder(&mut self, node: NodeId) {
        let placeholder = self.arena.new_node(NodeData {
            hash: NodeHash(0),
            kind: HtmlNodeKind::Text,
            properties: HtmlProps::default(),
        });
        node.insert_before(placeholder, &mut self.arena);
        node.detach(&mut self.arena);
    }

    /// Pretty-print the shadow tree for debugging.
    #[allow(dead_code)]
    fn debug_print_tree(&self, _title: &str) {
        debug!("=== {} ===", _title);
        for (_slot_num, slot_node) in self.super_root.children(&self.arena).enumerate() {
            debug!("Slot {}:", _slot_num);
            for content in slot_node.children(&self.arena) {
                self.debug_print_node(content, 1);
            }
        }
        debug!("===");
    }

    fn debug_print_node(&self, node: NodeId, depth: usize) {
        let _indent = "  ".repeat(depth);
        let data = &self.arena[node].get();
        let _kind_str = match &data.kind {
            HtmlNodeKind::Element(tag, _ns) => format!("<{}>", tag),
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
                    properties: HtmlProps {
                        text: Some(Stem::new()),
                        attrs: Vec::new(),
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
        // Check if node is a direct child of a slot (i.e., a slot root).
        // In that case, we just detach it without a placeholder.
        let parent = self.arena.get(node).and_then(|n| n.parent());
        let is_slot_root = parent
            .is_some_and(|p| self.arena.get(p).and_then(|n| n.parent()) == Some(self.super_root));

        if !is_slot_root {
            // Node is deeper in the tree - detach with placeholder to prevent shifts
            self.detach_with_placeholder(node);
        } else {
            // Node is a slot root - just detach it
            node.detach(&mut self.arena);
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
            // Fill gaps with placeholder text nodes (per Chawathe semantics)
            while new_parent.children(&self.arena).count() < position {
                let placeholder = self.arena.new_node(NodeData {
                    hash: NodeHash(0),
                    kind: HtmlNodeKind::Text,
                    properties: HtmlProps {
                        text: Some(Stem::new()),
                        attrs: Vec::new(),
                    },
                });
                new_parent.append(placeholder, &mut self.arena);
            }

            // Re-check after filling gaps
            let current_children: Vec<_> = new_parent.children(&self.arena).collect();
            if position < current_children.len() {
                let occupant = current_children[position];
                if occupant != node {
                    occupant.insert_before(node, &mut self.arena);
                    let slot = self.detach_to_slot(occupant);
                    debug!(?occupant, slot, "Move: detached gap-filler to slot");
                    Some(slot)
                } else {
                    None
                }
            } else {
                debug!("Move: appending (no occupant)");
                new_parent.append(node, &mut self.arena);
                None
            }
        }
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
fn convert_ops_with_shadow<T: DiffTree<Types = HtmlTreeTypes>>(
    ops: Vec<EditOp<HtmlTreeTypes>>,
    tree_a: &Tree<HtmlTreeTypes>,
    tree_b: &T,
    matching: &Matching,
) -> Result<Vec<Patch>, DiffError> {
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
                // The changes vec represents the complete final attribute state
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

                // cinereus only emits UpdateProperties when there's a real change or full removal
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
                let shadow_parent = b_to_shadow
                    .get(&parent_b)
                    .copied()
                    .unwrap_or_else(|| shadow.slot0_content());

                // Create a new node in shadow tree (placeholder for structure tracking)
                let new_data: NodeData<HtmlTreeTypes> = NodeData {
                    hash: NodeHash(0),
                    kind: kind.clone(),
                    properties: tree_b.properties(node_b).clone(),
                };
                let new_node = shadow.arena.new_node(new_data);

                // Insert and handle displacement automatically
                let detach_to_slot = shadow.insert_at_position(shadow_parent, position, new_node);

                b_to_shadow.insert(node_b, new_node);

                // Get reference with position included - this makes Insert consistent with Move!
                let at = shadow.get_node_ref_with_position(shadow_parent, position);

                // Create the patch based on node kind
                match kind {
                    HtmlNodeKind::Element(tag, _ns) => {
                        let (attrs, children) = extract_content_from_tree_b(
                            node_b,
                            tree_b,
                            &b_to_shadow,
                            &nodes_with_insert_ops,
                        );
                        result.push(Patch::InsertElement {
                            at,
                            tag: tag.clone(),
                            attrs,
                            children,
                            detach_to_slot,
                        });
                    }
                    HtmlNodeKind::Text => {
                        let text = tree_b.properties(node_b).text.clone().unwrap_or_default();
                        result.push(Patch::InsertText {
                            at,
                            text,
                            detach_to_slot,
                        });
                    }
                    HtmlNodeKind::Comment => {
                        let text = tree_b.properties(node_b).text.clone().unwrap_or_default();
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

                // Get the node reference (path starts with slot number)
                let node = shadow.get_node_ref(node_a);

                // Detach from tree with placeholder to preserve sibling positions
                shadow.detach_with_placeholder(node_a);

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
                    .unwrap_or_else(|| shadow.slot0_content());

                debug!(
                    ?node_a,
                    ?new_parent_b,
                    ?shadow_new_parent,
                    ?new_position,
                    "Move: starting"
                );

                // Get source reference
                debug!(?node_a, "Move: computing from reference for node");
                let from = shadow.get_node_ref(node_a);
                debug!(?node_a, ?from, "Move: computed from reference");

                // Debug: check what's at the target position BEFORE the move
                #[cfg(test)]
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

                // Get target reference with position
                let to = shadow.get_node_ref_with_position(shadow_new_parent, new_position);

                debug!(?node_a, ?from, ?to, ?detach_to_slot, "Generated Move patch");

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
fn extract_content_from_tree_b<T: DiffTree<Types = HtmlTreeTypes>>(
    node_b: NodeId,
    tree_b: &T,
    b_to_shadow: &HashMap<NodeId, NodeId>,
    nodes_with_insert_ops: &HashSet<NodeId>,
) -> (Vec<AttrPair>, Vec<InsertContent>) {
    let props = tree_b.properties(node_b);
    let attrs: Vec<_> = props
        .attrs
        .iter()
        .map(|(k, v)| AttrPair {
            name: k.clone(),
            value: v.clone(),
        })
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

        let child_kind = tree_b.kind(child_id);
        let child_props = tree_b.properties(child_id);
        match child_kind {
            HtmlNodeKind::Element(tag, _ns) => {
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
                let text = child_props.text.clone().unwrap_or_default();
                children.push(InsertContent::Text(text));
            }
            HtmlNodeKind::Comment => {
                let text = child_props.text.clone().unwrap_or_default();
                children.push(InsertContent::Comment(text));
            }
        }
    }

    (attrs, children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom;
    use facet_testhelpers::test;

    #[test]
    fn test_build_tree_simple() {
        let doc = dom::parse("<html><body><div>hello</div></body></html>");
        let tree = build_tree_from_arena(&doc);

        // Root is body element (build_tree_from_arena uses body as root)
        let root_data = tree.get(tree.root);
        assert!(matches!(&root_data.kind, HtmlNodeKind::Element(t, _) if t.as_ref() == "body"));

        // One child (div)
        assert_eq!(tree.child_count(tree.root), 1);
    }

    #[test]
    fn test_diff_text_change() {
        let old_html = "<html><body><div>old</div></body></html>";
        let new_html = "<html><body><div>new</div></body></html>";

        let old = dom::parse(old_html);
        let new = dom::parse(new_html);
        let patches = diff(&old, &new).unwrap();

        // Should have an UpdateProps patch for the text change
        let has_text_update = patches.iter().any(|p| {
            matches!(p, Patch::UpdateProps { changes, .. }
                if changes.iter().any(|c| matches!(c.name, PropKey::Text)))
        });
        assert!(
            has_text_update,
            "Expected text update patch, got: {:?}",
            patches
        );
    }

    #[test]
    fn test_diff_attr_change() {
        let old_html = r#"<html><body><div class="foo"></div></body></html>"#;
        let new_html = r#"<html><body><div class="bar"></div></body></html>"#;

        let old = dom::parse(old_html);
        let new = dom::parse(new_html);
        let patches = diff(&old, &new).unwrap();

        let has_attr_update = patches.iter().any(|p| {
            matches!(p, Patch::UpdateProps { changes, .. }
                if changes.iter().any(|c| matches!(c.name, PropKey::Attr(ref q) if q.local.as_ref() == "class")))
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
        let old_html = "<html><body><span></span></body></html>";
        let new_html = "<html><body></body></html>";

        let old = dom::parse(old_html);
        let new = dom::parse(new_html);
        let patches = diff(&old, &new).unwrap();
        debug!("Patches: {:#?}", patches);

        // Should be able to apply the patches
        let mut doc = dom::parse(old_html);
        doc.apply_patches(patches).expect("apply should succeed");

        let result = doc.to_html();
        let expected = dom::parse(new_html).to_html();
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_diff_complex_fuzzer_case() {
        // Fuzzer found: <body><strong>old_text</strong></body> -> <body>new_text<strong>updated</strong></body>
        let old_html = "<html><body><strong>old</strong></body></html>";
        let new_html = "<html><body>new_text<strong>updated</strong></body></html>";

        let old = dom::parse(old_html);
        let new = dom::parse(new_html);
        let patches = diff(&old, &new).unwrap();
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(old_html);
        doc.apply_patches(patches).expect("apply should succeed");

        let result = doc.to_html();
        let expected = dom::parse(new_html).to_html();
        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_diff_actual_fuzzer_crash() {
        // Actual fuzzer crash case (simplified):
        // Old: <strong>text1</strong><strong>text2</strong><img>
        // New: text3<strong>text4</strong>
        let old_html =
            "<html><body><strong>text1</strong><strong>text2</strong><img></body></html>";
        let new_html = "<html><body>text3<strong>text4</strong></body></html>";

        let old = dom::parse(old_html);
        let new = dom::parse(new_html);
        let patches = diff(&old, &new).unwrap();
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(old_html);
        doc.apply_patches(patches).expect("apply should succeed");

        let result = doc.to_html();
        let expected = dom::parse(new_html).to_html();
        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
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

        let patches = super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(old_html);
        trace!("Old tree: {:#?}", doc);

        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected = dom::parse(new_html).to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_img_li_roundtrip() {
        // Fuzzer found roundtrip failure with img and li elements
        let old_html = r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://www.w3.org/TR/html4/strict.dtd"><html><body><p>Unclosed paragraph<span class=""></span><div>Inside P which browsers will auto-close</div><span>Unclosed span<div>Block in span</div></body></html>"#;
        let new_html = r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://www.w3.org/TR/html4/strict.dtd"><html><body><img src="vvv" alt="ttt">d <li>d<<<&<<a"d <<<</li></body></html>"#;

        let patches = super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(old_html);
        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected_doc = dom::parse(new_html);
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

        let patches = super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(old_html);
        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected_doc = dom::parse(new_html);
        let expected = expected_doc.to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_em_li_navigate_text() {
        // Fuzzer found "Insert: cannot navigate through text node" error
        // The fuzzer generates unescaped HTML which html5ever parses as actual elements
        let old_html = r#"<!DOCTYPE html><html><body><em> <v<      << v</em></body></html>"#;
        let new_html =
            r#"<!DOCTYPE html><html><body><li>a< <v<      <<</li><img src=""></body></html>"#;

        let old_doc = dom::parse(old_html);
        let new_doc = dom::parse(new_html);
        debug!("Old HTML parsed: {:#?}", old_doc);
        debug!("New HTML parsed: {:#?}", new_doc);

        let patches = super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(old_html);
        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected = dom::parse(new_html).to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_nested_ol_patch_order() {
        // Fuzzer found "patch at index 4 is out of order" error
        let old_html = r#"<!DOCTYPE html><html><body><ol start="0"></ol></body></html>"#;
        let new_html = r#"<!DOCTYPE html><html><body><ol start="255"></ol><ol start="93"></ol><ol start="91"><ol start="1"><a href="vaaaaaaaaaaaaa"></a></ol></ol></body></html>"#;

        let patches = super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(old_html);
        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected_doc = dom::parse(new_html);
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

        let patches = super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(old_html);
        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected = dom::parse(new_html).to_html();

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

        let patches = super::diff_html(old_html, new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(old_html);
        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected = dom::parse(new_html).to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_arena_dom_diff() {
        // Test diffing with arena_dom documents
        let old_html = "<html><body><div>Old content</div></body></html>";
        let new_html = "<html><body><div>New content</div></body></html>";

        let old_doc = dom::parse(old_html);
        let new_doc = dom::parse(new_html);

        let patches = diff(&old_doc, &new_doc).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        // Should have an UpdateProps patch with _text change
        assert_eq!(patches.len(), 1);
        match &patches[0] {
            Patch::UpdateProps { path, changes } => {
                // Path: [slot=0, div=0, text=0] - slot 0, first child of body (div), first child of div (text)
                assert_eq!(path.0.as_slice(), &[0, 0, 0]);
                assert_eq!(changes.len(), 1);
                assert!(matches!(changes[0].name, PropKey::Text));
                assert_eq!(changes[0].value, Some(Stem::from("New content")));
            }
            _ => panic!("Expected UpdateProps patch, got {:?}", patches[0]),
        }
    }

    #[test]
    fn test_arena_dom_diff_add_element() {
        let old_html = "<html><body><div>Content</div></body></html>";
        let new_html = "<html><body><div>Content</div><p>New paragraph</p></body></html>";

        let old_doc = dom::parse(old_html);
        let new_doc = dom::parse(new_html);

        let patches = diff(&old_doc, &new_doc).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        // Should have an InsertElement patch
        assert!(!patches.is_empty());
        let has_insert = patches
            .iter()
            .any(|p| matches!(p, Patch::InsertElement { .. }));
        assert!(has_insert, "Should have InsertElement patch");
    }

    #[test]
    fn test_patch_serialization() {
        use crate::diff_html;

        let old_html = r#"<html><body><div>Content</div></body></html>"#;
        let new_html = r#"<html><body><div class="highlight">Content</div></body></html>"#;

        let patches = diff_html(old_html, new_html).expect("diff should work");

        let json = facet_json::to_string(&patches).expect("serialization should work");
        let roundtrip: Vec<Patch> =
            facet_json::from_str(&json).expect("deserialization should work");
        assert_eq!(patches, roundtrip);
    }

    #[test]
    fn test_fuzz_seed_0_template_0() {
        use crate::diff_html;

        let old_html = r##"<article>
    <h1>Article Title</h1>
    <p>First paragraph with <strong>bold</strong> and <em>italic</em> text.</p>
    <p>Second paragraph with a <a href="#">link</a>.</p>
  </article>"##;

        let new_html = r##"<article>

    <p>First paragraph with <strong>bold</strong> and content</p>
    <p data-test="hidden">Second paragraph with a .</p>
  </article>"##;

        let patches = diff_html(old_html, new_html).expect("diff should work");
        debug!("Patches: {:#?}", patches);

        // Check for slot references (first path element > 0 means it's in a non-main slot)
        for (i, patch) in patches.iter().enumerate() {
            debug!("Patch {}: {:?}", i, patch);
            if let Patch::Move { from, to, .. } = patch {
                let from_slot = from.0.0.first().copied().unwrap_or(0);
                let to_slot = to.0.0.first().copied().unwrap_or(0);
                if from_slot > 0 {
                    debug!("  -> Move FROM slot {}", from_slot);
                }
                if to_slot > 0 {
                    debug!("  -> Move TO slot {}", to_slot);
                }
            }
        }
    }

    #[test]
    fn test_fuzz_seed_27_template_4() {
        use crate::diff_html;
        use crate::dom;

        // This test reproduces a bug where the diff algorithm generates paths like [0,2,3,0]
        // but after all Move operations, [0,2] is a text node (which has no children).
        // The Delete operation tries to navigate to child 3 of a text node and fails.

        let old_html = r##"<div>
    <h3>Features</h3>
    <ul>
      <li>Feature one with <code>code</code></li>
      <li>Feature two with <strong>emphasis</strong></li>
      <li>Feature three</li>
    </ul>
  </div>"##;

        let new_html = r##"<div title="primary">
    <h3>Features</h3>
    <ul>
      <li>Feature one with <code>item</code></li>


    </ul>
  </div>"##;

        let patches = diff_html(old_html, new_html).expect("diff should work");
        debug!("Patches: {:#?}", patches);

        for (i, patch) in patches.iter().enumerate() {
            debug!("Patch {}: {:?}", i, patch);
        }

        // Apply all patches at once
        let mut doc = dom::parse(&format!("<html><body>{}</body></html>", old_html));
        if let Err(e) = doc.apply_patches(patches.clone()) {
            panic!("Patches failed: {:?}", e);
        }
    }

    #[test]
    fn measure_position_calls_xxl() {
        use cinereus::{get_position_stats, reset_position_counters};

        let xxl_html = include_str!("../tests/fixtures/xxl.html");
        let modified = xxl_html.replacen("<div", "<div class=\"modified\"", 1);

        // Reset counters
        reset_position_counters();

        // Do the diff
        let old = dom::parse(xxl_html);
        let new = dom::parse(&modified);
        let _patches = diff(&old, &new).expect("diff failed");

        // Get stats
        let (calls, scanned) = get_position_stats();

        println!("\n=== XXL document diff position() stats ===");
        println!("  position() calls: {}", calls);
        println!("  siblings scanned: {}", scanned);
        if calls > 0 {
            println!(
                "  avg siblings per call: {:.2}",
                scanned as f64 / calls as f64
            );
        }
        println!("===========================================\n");
    }
}
