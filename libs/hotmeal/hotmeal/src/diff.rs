//! HTML diffing with DOM patch generation.
//!
//! This module uses cinereus (GumTree/Chawathe) to compute tree diffs and translates
//! them into DOM patches that can be applied to update an HTML document incrementally.

use crate::{
    Stem, StrTendril, debug,
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
use smallvec::{SmallVec, smallvec};
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
            // Filter out empty prefix strings - they should be None
            prefix: proxy.prefix.filter(|s| !s.is_empty()).map(Prefix::from),
            ns: Namespace::from(proxy.ns),
            local: LocalName::from(proxy.local),
        }
    }
}

impl From<&QualName> for QualNameProxy {
    fn from(qual: &QualName) -> Self {
        QualNameProxy {
            // Filter out empty prefix - serialize as None instead of Some("")
            prefix: qual
                .prefix
                .as_ref()
                .filter(|p| !p.is_empty())
                .map(|p| p.to_string()),
            ns: qual.ns.to_string(),
            local: qual.local.to_string(),
        }
    }
}

/// An attribute name-value pair
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct AttrPair<'a> {
    #[facet(opaque, proxy = QualNameProxy)]
    pub name: QualName,
    pub value: Stem<'a>,
}

impl<'a> From<(QualName, Stem<'a>)> for AttrPair<'a> {
    fn from((name, value): (QualName, Stem<'a>)) -> Self {
        AttrPair { name, value }
    }
}

impl<'a> From<AttrPair<'a>> for (QualName, Stem<'a>) {
    fn from(pair: AttrPair<'a>) -> Self {
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
pub enum InsertContent<'a> {
    /// An element with its tag, attributes, and nested children
    Element {
        #[facet(opaque, proxy = LocalNameProxy)]
        tag: LocalName,
        attrs: Vec<AttrPair<'a>>,
        children: Vec<InsertContent<'a>>,
    },
    /// A text node
    Text(Stem<'a>),
    /// A comment node
    Comment(Stem<'a>),
}

/// A property in the final state within an UpdateProps operation.
/// The vec position determines the final ordering.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct PropChange<'a> {
    /// The property key (attribute name)
    pub name: PropKey,
    /// The value: None means "keep existing value", Some means "update to this value".
    /// Properties not in the list are implicitly removed.
    pub value: Option<Stem<'a>>,
}

/// Operations to transform the DOM.
#[derive(Clone, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum Patch<'a> {
    /// Insert an element at a position.
    /// The `at` NodeRef path includes the position as the last segment.
    /// Path `[slot, a, b, c]` means: in slot, navigate to a, then b, then insert at position c.
    InsertElement {
        at: NodeRef,
        #[facet(opaque, proxy = LocalNameProxy)]
        tag: LocalName,
        attrs: Vec<AttrPair<'a>>,
        children: Vec<InsertContent<'a>>,
        detach_to_slot: Option<u32>,
    },

    /// Insert a text node at a position.
    InsertText {
        at: NodeRef,
        text: Stem<'a>,
        detach_to_slot: Option<u32>,
    },

    /// Insert a comment node at a position.
    InsertComment {
        at: NodeRef,
        text: Stem<'a>,
        detach_to_slot: Option<u32>,
    },

    /// Remove a node
    Remove { node: NodeRef },

    /// Update text content of a text node at path.
    SetText { path: NodePath, text: Stem<'a> },

    /// Set attribute on element at path
    SetAttribute {
        path: NodePath,
        #[facet(opaque, proxy = QualNameProxy)]
        name: QualName,
        value: Stem<'a>,
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
        changes: Vec<PropChange<'a>>,
    },
}

impl<'a> std::fmt::Debug for Patch<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Patch::InsertElement {
                at,
                tag,
                attrs,
                children,
                detach_to_slot,
            } => {
                write!(f, "Insert <{}> @{:?}", tag, at.0.0.as_slice())?;
                if !attrs.is_empty() {
                    write!(f, " ({} attrs)", attrs.len())?;
                }
                if !children.is_empty() {
                    write!(f, " ({} children)", children.len())?;
                }
                if let Some(slot) = detach_to_slot {
                    write!(f, " →slot{}", slot)?;
                }
                Ok(())
            }
            Patch::InsertText {
                at,
                text,
                detach_to_slot,
            } => {
                let preview: String = text.chars().take(20).collect();
                write!(f, "Insert text {:?} @{:?}", preview, at.0.0.as_slice())?;
                if let Some(slot) = detach_to_slot {
                    write!(f, " →slot{}", slot)?;
                }
                Ok(())
            }
            Patch::InsertComment {
                at,
                text,
                detach_to_slot,
            } => {
                let preview: String = text.chars().take(20).collect();
                write!(f, "Insert comment {:?} @{:?}", preview, at.0.0.as_slice())?;
                if let Some(slot) = detach_to_slot {
                    write!(f, " →slot{}", slot)?;
                }
                Ok(())
            }
            Patch::Remove { node } => {
                write!(f, "Remove @{:?}", node.0.0.as_slice())
            }
            Patch::SetText { path, text } => {
                let preview: String = text.chars().take(20).collect();
                write!(f, "SetText {:?} @{:?}", preview, path.0.as_slice())
            }
            Patch::SetAttribute { path, name, value } => {
                write!(
                    f,
                    "SetAttr {}={:?} @{:?}",
                    name.local,
                    value.as_ref(),
                    path.0.as_slice()
                )
            }
            Patch::RemoveAttribute { path, name } => {
                write!(f, "RemoveAttr {} @{:?}", name.local, path.0.as_slice())
            }
            Patch::Move {
                from,
                to,
                detach_to_slot,
            } => {
                write!(
                    f,
                    "Move {:?} → {:?}",
                    from.0.0.as_slice(),
                    to.0.0.as_slice()
                )?;
                if let Some(slot) = detach_to_slot {
                    write!(f, " →slot{}", slot)?;
                }
                Ok(())
            }
            Patch::UpdateProps { path, changes } => {
                write!(
                    f,
                    "UpdateProps @{:?} ({} changes)",
                    path.0.as_slice(),
                    changes.len()
                )
            }
        }
    }
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

/// HTML element properties (attributes only).
/// Text content is stored separately in NodeData::text.
#[derive(Debug, Clone, Default)]
pub struct HtmlProps<'a> {
    /// Attributes - Vec preserves insertion order for consistent serialization
    /// Keys are QualName (preserves namespace for xlink:href, xml:lang, etc.)
    pub attrs: Vec<(QualName, Stem<'a>)>,
}

impl<'a> Properties for HtmlProps<'a> {
    type Key = PropKey;
    type Value = Stem<'a>;

    #[allow(clippy::mutable_key_type)]
    fn similarity(&self, other: &Self) -> f64 {
        // Compare attributes using Dice coefficient
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

        // Check if attribute ORDER differs (even if values are the same).
        // Build a list of common keys in the order they appear in self.
        let self_keys: Vec<_> = self.attrs.iter().map(|(k, _)| k).collect();
        let other_keys: Vec<_> = other.attrs.iter().map(|(k, _)| k).collect();

        // Find common keys in the order they appear in self
        let self_common: Vec<_> = self_keys
            .iter()
            .filter(|k| other_keys.contains(k))
            .copied()
            .collect();
        // Find common keys in the order they appear in other
        let other_common: Vec<_> = other_keys
            .iter()
            .filter(|k| self_keys.contains(k))
            .copied()
            .collect();

        // If common keys are in different order, we need to force an update
        let order_differs = self_common != other_common;

        // Include all attributes from the final state in order
        let mut forced_one = false;
        for (key, new_val) in &other.attrs {
            let old_val = self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v);
            let value_same = old_val == Some(new_val);

            // If order differs and values are the same, force at least one to be Different
            // to trigger the UpdateProperties operation
            let force_different = order_differs && value_same && !forced_one;
            if force_different {
                forced_one = true;
            }

            result.push(PropertyInFinalState {
                key: PropKey::Attr(key.clone()),
                value: if value_same && !force_different {
                    PropValue::Same
                } else {
                    PropValue::Different(new_val.clone())
                },
            });
        }

        result
    }

    fn is_empty(&self) -> bool {
        self.attrs.is_empty()
    }

    fn len(&self) -> usize {
        self.attrs.len()
    }
}

/// Tree types marker for HTML DOM.
pub struct HtmlTreeTypes<'a>(std::marker::PhantomData<&'a ()>);

impl<'a> TreeTypes for HtmlTreeTypes<'a> {
    type Kind = HtmlNodeKind;
    type Props = HtmlProps<'a>;
    type Text = Stem<'a>;
}

/// Pre-computed diff data for a node.
struct DiffNodeData<'a> {
    hash: NodeHash,
    kind: HtmlNodeKind,
    props: HtmlProps<'a>,
    /// Text content for text/comment nodes
    text: Option<Stem<'a>>,
    height: usize,
    /// Cached position among siblings (0-indexed), computed on-demand
    position: Cell<Option<u32>>,
}

/// A wrapper around Document that implements DiffTree.
///
/// This allows diffing Documents directly without building a separate cinereus Tree.
/// The wrapper pre-computes hashes and caches kind/props for each node.
///
/// Two lifetime parameters:
/// - `'b` - how long we borrow the Document
/// - `'a` - the lifetime of data inside the Document (Stem<'a> values from original input)
pub struct DiffableDocument<'b, 'a> {
    doc: &'b Document<'a>,
    /// The root for diffing (body element)
    body_id: NodeId,
    /// Pre-computed diff data indexed by NodeId
    nodes: HashMap<NodeId, DiffNodeData<'a>>,
}

impl<'b, 'a> DiffableDocument<'b, 'a> {
    /// Create a new DiffableDocument from a Document.
    ///
    /// Pre-computes hashes and caches kind/props for all body descendants.
    pub fn new(doc: &'b Document<'a>) -> Result<Self, DiffError> {
        let body_id = doc.body().ok_or(DiffError::NoBody)?;
        // Pre-allocate based on arena size (upper bound for descendants)
        let mut nodes = HashMap::with_capacity(doc.arena.count());

        // First pass: compute kind, props, and text for all nodes
        for node_id in body_id.descendants(&doc.arena) {
            let node = doc.get(node_id);
            let (kind, props, text) = match &node.kind {
                NodeKind::Element(elem) => {
                    let kind = HtmlNodeKind::Element(elem.tag.clone(), node.ns);
                    let props = HtmlProps {
                        attrs: elem
                            .attrs
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    };
                    (kind, props, None)
                }
                NodeKind::Text(text) => {
                    let kind = HtmlNodeKind::Text;
                    let props = HtmlProps { attrs: Vec::new() };
                    (kind, props, Some(text.clone()))
                }
                NodeKind::Comment(text) => {
                    let kind = HtmlNodeKind::Comment;
                    let props = HtmlProps { attrs: Vec::new() };
                    (kind, props, Some(text.clone()))
                }
                NodeKind::Document => continue, // Skip document nodes
            };

            nodes.insert(
                node_id,
                DiffNodeData {
                    hash: NodeHash(0), // Will be computed in second pass
                    kind,
                    props,
                    text,
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

            // Compute hash (Merkle-tree style): kind + text + children
            let hash = if let Some(data) = nodes.get(&node_id) {
                let mut hasher = RapidHasher::default();
                data.kind.hash(&mut hasher);
                // Include text content - this is the identity of text/comment nodes
                if let Some(text) = &data.text {
                    text.hash(&mut hasher);
                }
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

        Ok(Self {
            doc,
            body_id,
            nodes,
        })
    }
}

impl<'b, 'a> DiffTree for DiffableDocument<'b, 'a> {
    type Types = HtmlTreeTypes<'a>;

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

    fn properties(&self, id: NodeId) -> &HtmlProps<'a> {
        self.nodes
            .get(&id)
            .map(|d| &d.props)
            .expect("node must exist")
    }

    fn text(&self, id: NodeId) -> Option<&Stem<'a>> {
        self.nodes.get(&id).and_then(|d| d.text.as_ref())
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
struct PostOrderIterator<'a, 'b> {
    arena: &'a indextree::Arena<dom::NodeData<'b>>,
    stack: Vec<(NodeId, bool)>,
}

impl<'a, 'b> PostOrderIterator<'a, 'b> {
    fn new(root: NodeId, arena: &'a indextree::Arena<dom::NodeData<'b>>) -> Self {
        Self {
            arena,
            stack: vec![(root, false)],
        }
    }
}

impl Iterator for PostOrderIterator<'_, '_> {
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
/// If the document has no body, returns an empty body tree.
pub fn build_tree_from_arena<'a>(doc: &Document<'a>) -> Tree<HtmlTreeTypes<'a>> {
    // Find body element, or create an empty body tree if none exists
    let (body_tag, body_ns, body_id) = if let Some(body_id) = doc.body() {
        let body_node = doc.get(body_id);
        if let NodeKind::Element(elem) = &body_node.kind {
            (elem.tag.clone(), body_node.ns, Some(body_id))
        } else {
            (LocalName::from("body"), Namespace::Html, None)
        }
    } else {
        (LocalName::from("body"), Namespace::Html, None)
    };

    let root_data = NodeData {
        hash: NodeHash(0),
        kind: HtmlNodeKind::Element(body_tag, body_ns),
        properties: HtmlProps { attrs: Vec::new() },
        text: None,
    };

    let mut tree = Tree::new(root_data);
    let tree_root = tree.root;

    // Add children from body (only if we have a body)
    if let Some(body_id) = body_id {
        add_arena_children(&mut tree, tree_root, doc, body_id);
    }

    // Recompute hashes bottom-up
    recompute_hashes(&mut tree);

    tree
}

fn add_arena_children<'a>(
    tree: &mut Tree<HtmlTreeTypes<'a>>,
    parent: indextree::NodeId,
    doc: &Document<'a>,
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
                };
                let data = NodeData {
                    hash: NodeHash(0),
                    kind,
                    properties: props,
                    text: None,
                };
                let node_id = tree.add_child(parent, data);
                add_arena_children(tree, node_id, doc, child_id);
            }
            NodeKind::Text(text) => {
                let kind = HtmlNodeKind::Text;
                let props = HtmlProps { attrs: Vec::new() };
                let data = NodeData {
                    hash: NodeHash(0),
                    kind,
                    properties: props,
                    text: Some(text.clone()),
                };
                tree.add_child(parent, data);
            }
            NodeKind::Comment(text) => {
                let kind = HtmlNodeKind::Comment;
                let props = HtmlProps { attrs: Vec::new() };
                let data = NodeData {
                    hash: NodeHash(0),
                    kind,
                    properties: props,
                    text: Some(text.clone()),
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
/// The hash captures: node kind + text content + children structure (Merkle-tree style).
/// Attributes are NOT included - they're compared via the Properties trait after matching,
/// using similarity scores to decide whether nodes should match.
fn recompute_hashes(tree: &mut Tree<HtmlTreeTypes<'_>>) {
    // Process in post-order (children before parents)
    let nodes: Vec<NodeId> = tree.post_order().collect();

    for node_id in nodes {
        let mut hasher = RapidHasher::default();

        let data = tree.get(node_id);
        data.kind.hash(&mut hasher);

        // Include text content in hash - this is the identity of text/comment nodes
        if let Some(text) = &data.text {
            text.hash(&mut hasher);
        }

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

/// Create an insert patch for a node and all its descendants.
/// Used when we need to insert entire subtrees (e.g., when old doc has no body).
fn create_insert_patch<'a>(
    doc: &Document<'a>,
    node_id: NodeId,
    position: usize,
) -> Result<Patch<'a>, DiffError> {
    let node = doc.get(node_id);

    match &node.kind {
        NodeKind::Element(elem) => {
            let attrs: Vec<AttrPair<'a>> = elem
                .attrs
                .iter()
                .map(|(name, value)| AttrPair {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect();

            let children: Vec<InsertContent<'a>> = node_id
                .children(&doc.arena)
                .filter_map(|child_id| create_insert_content(doc, child_id))
                .collect();

            Ok(Patch::InsertElement {
                at: NodeRef(NodePath(smallvec![0, position as u32])),
                tag: elem.tag.clone(),
                attrs,
                children,
                detach_to_slot: None,
            })
        }
        NodeKind::Text(text) => Ok(Patch::InsertText {
            at: NodeRef(NodePath(smallvec![0, position as u32])),
            text: text.clone(),
            detach_to_slot: None,
        }),
        NodeKind::Comment(text) => Ok(Patch::InsertComment {
            at: NodeRef(NodePath(smallvec![0, position as u32])),
            text: text.clone(),
            detach_to_slot: None,
        }),
        NodeKind::Document => Err(DiffError::NoBody), // Shouldn't happen
    }
}

/// Create InsertContent for a node and its descendants (recursive).
fn create_insert_content<'a>(doc: &Document<'a>, node_id: NodeId) -> Option<InsertContent<'a>> {
    let node = doc.get(node_id);

    match &node.kind {
        NodeKind::Element(elem) => {
            let attrs: Vec<AttrPair<'a>> = elem
                .attrs
                .iter()
                .map(|(name, value)| AttrPair {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect();

            let children: Vec<InsertContent<'a>> = node_id
                .children(&doc.arena)
                .filter_map(|child_id| create_insert_content(doc, child_id))
                .collect();

            Some(InsertContent::Element {
                tag: elem.tag.clone(),
                attrs,
                children,
            })
        }
        NodeKind::Text(text) => Some(InsertContent::Text(text.clone())),
        NodeKind::Comment(text) => Some(InsertContent::Comment(text.clone())),
        NodeKind::Document => None,
    }
}

/// Diff two HTML tendrils and return DOM patches.
///
/// Parses both HTML inputs and diffs them.
/// Patches borrow from the tendrils' buffers (zero-copy).
pub fn diff_html<'a>(
    old_html: &'a StrTendril,
    new_html: &'a StrTendril,
) -> Result<Vec<Patch<'a>>, DiffError> {
    let old_doc = dom::parse(old_html);
    let new_doc = dom::parse(new_html);
    diff(&old_doc, &new_doc)
}

/// Diff two arena documents and return DOM patches.
///
/// This is the primary diffing API for arena_dom documents.
pub fn diff<'a>(old: &Document<'a>, new: &Document<'a>) -> Result<Vec<Patch<'a>>, DiffError> {
    let old_has_body = old.body().is_some();
    let new_has_body = new.body().is_some();

    // Handle cases where one or both documents have no body
    match (old_has_body, new_has_body) {
        (false, false) => {
            // Neither has body - nothing to diff
            return Ok(vec![]);
        }
        (false, true) => {
            // Old has no body, new has body - insert all new body children
            let new_body_id = new.body().unwrap();
            let mut patches = Vec::new();
            for (pos, child_id) in new_body_id.children(&new.arena).enumerate() {
                patches.push(create_insert_patch(new, child_id, pos)?);
            }
            return Ok(patches);
        }
        (true, false) => {
            // Old has body, new has no body - remove all old body children
            let old_body_id = old.body().unwrap();
            let mut patches = Vec::new();
            // Remove children in reverse order to keep indices stable
            let children: Vec<_> = old_body_id.children(&old.arena).collect();
            for (pos, _child_id) in children.iter().enumerate().rev() {
                patches.push(Patch::Remove {
                    node: NodeRef(NodePath(smallvec![0, pos as u32])),
                });
            }
            return Ok(patches);
        }
        (true, true) => {
            // Both have bodies - proceed with normal diffing
        }
    }

    // Build cinereus Tree for old (needed for shadow tree mutation)
    let tree_a = build_tree_from_arena(old);
    // Use DiffableDocument for new (avoids second tree allocation)
    let diff_b = DiffableDocument::new(new)?;

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
struct ShadowTree<'a> {
    arena: indextree::Arena<NodeData<HtmlTreeTypes<'a>>>,
    /// The super root - its children are slot nodes
    super_root: NodeId,
    /// Number of slots (slot 0 always exists, created in new())
    next_slot: u32,
}

/// Extract short node label like "n1" from NodeId debug output
fn node_id_short(node_id: NodeId) -> String {
    let debug = format!("{:?}", node_id);
    let Some(start) = debug.find("index1: ") else {
        return debug;
    };
    let digits = &debug[start + "index1: ".len()..];
    let value: String = digits.chars().take_while(|c| c.is_ascii_digit()).collect();
    if value.is_empty() {
        debug
    } else {
        format!("n{}", value)
    }
}

/// Helper for pretty-printing a shadow tree
struct ShadowTreeDump<'a, 'b> {
    shadow: &'b ShadowTree<'a>,
    highlights: &'b [(NodeId, &'static str, &'static str)],
}

impl<'a, 'b> std::fmt::Display for ShadowTreeDump<'a, 'b> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (slot_num, slot_node) in self
            .shadow
            .super_root
            .children(&self.shadow.arena)
            .enumerate()
        {
            writeln!(f, "Slot {}:", slot_num)?;
            for content in slot_node.children(&self.shadow.arena) {
                self.fmt_node(f, content, 1)?;
            }
        }
        Ok(())
    }
}

impl<'a, 'b> ShadowTreeDump<'a, 'b> {
    fn highlight_for(&self, node_id: NodeId) -> Option<(&'static str, &'static str)> {
        self.highlights
            .iter()
            .find(|(id, _, _)| *id == node_id)
            .map(|(_, color, label)| (*color, *label))
    }

    fn fmt_node(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        node: NodeId,
        depth: usize,
    ) -> std::fmt::Result {
        let indent = "  ".repeat(depth);
        let node_label = node_id_short(node);
        let prefix = format!("{indent}[{node_label}] ");
        let data = &self.shadow.arena[node].get();

        let highlight = self.highlight_for(node);
        let (hl_start, hl_end, hl_label) = if let Some((color, label)) = highlight {
            (color, "\x1b[0m", label)
        } else {
            ("", "", "")
        };
        let badge = if hl_label.is_empty() {
            String::new()
        } else {
            format!(" {hl_start}<{hl_label}>{hl_end}")
        };

        match &data.kind {
            HtmlNodeKind::Element(tag, _ns) => {
                let tag_display = if hl_start.is_empty() {
                    tag.to_string()
                } else {
                    format!("{hl_start}{tag}{hl_end}")
                };
                writeln!(f, "{prefix}<{tag_display}>{badge}")?;
                for child in node.children(&self.shadow.arena) {
                    self.fmt_node(f, child, depth + 1)?;
                }
                writeln!(f, "{prefix}</{tag_display}>")?;
            }
            HtmlNodeKind::Text => {
                let text = data.text.as_deref().unwrap_or("");
                writeln!(f, "{prefix}TEXT: {text:?}{badge}")?;
                // Placeholders are TEXT nodes but may have children
                for child in node.children(&self.shadow.arena) {
                    self.fmt_node(f, child, depth + 1)?;
                }
            }
            HtmlNodeKind::Comment => {
                let text = data.text.as_deref().unwrap_or("");
                writeln!(f, "{prefix}COMMENT: {text:?}{badge}")?;
                // Slots are COMMENT nodes with children
                for child in node.children(&self.shadow.arena) {
                    self.fmt_node(f, child, depth + 1)?;
                }
            }
        }
        Ok(())
    }
}

impl<'a> ShadowTree<'a> {
    fn new(
        mut arena: indextree::Arena<NodeData<HtmlTreeTypes<'a>>>,
        original_root: NodeId,
    ) -> Self {
        // Create the super root (a meta node, not a real DOM node)
        let super_root = arena.new_node(NodeData {
            hash: NodeHash(0),
            kind: HtmlNodeKind::Comment, // Just a marker, not used
            properties: HtmlProps::default(),
            text: None,
        });

        // Create slot 0 and reparent the original tree under it
        let slot0 = arena.new_node(NodeData {
            hash: NodeHash(0),
            kind: HtmlNodeKind::Comment, // Slot marker
            properties: HtmlProps::default(),
            text: None,
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
            text: None,
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
            text: None,
        });
        node.insert_before(placeholder, &mut self.arena);
        node.detach(&mut self.arena);
    }

    /// Check if `ancestor` is an ancestor of `node`.
    fn is_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut current = node;
        while let Some(parent) = self.arena.get(current).and_then(|n| n.parent()) {
            if parent == ancestor {
                return true;
            }
            current = parent;
        }
        false
    }

    /// Replace a node with a placeholder, returning the placeholder's NodeId.
    /// The node's children are reparented under the placeholder.
    fn replace_with_placeholder(&mut self, node: NodeId) -> NodeId {
        debug!(
            ?node,
            node_kind = %self.arena[node].get().kind,
            "replace_with_placeholder: replacing node"
        );
        let placeholder = self.arena.new_node(NodeData {
            hash: NodeHash(0),
            kind: HtmlNodeKind::Text,
            properties: HtmlProps::default(),
            text: None,
        });
        // Insert placeholder as sibling of node
        node.insert_before(placeholder, &mut self.arena);
        // Move all children from node to placeholder
        let children: Vec<_> = node.children(&self.arena).collect();
        debug!(
            children_count = children.len(),
            "replace_with_placeholder: moving children to placeholder"
        );
        for child in children {
            child.detach(&mut self.arena);
            placeholder.append(child, &mut self.arena);
        }
        // Now detach the empty node
        node.detach(&mut self.arena);
        debug!(?placeholder, "replace_with_placeholder: done");
        placeholder
    }

    /// Pretty-print the shadow tree for debugging.
    #[allow(dead_code)]
    fn debug_print_tree(&self, title: &str) {
        self.debug_print_tree_with_highlights(title, &[]);
    }

    /// Pretty-print the shadow tree with highlighted nodes.
    #[allow(dead_code)]
    fn debug_print_tree_with_highlights(
        &self,
        title: &str,
        highlights: &[(NodeId, &'static str, &'static str)],
    ) {
        debug!(
            "=== {} ===\n{}",
            title,
            ShadowTreeDump {
                shadow: self,
                highlights
            }
        );
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
                    properties: HtmlProps { attrs: Vec::new() },
                    text: Some(Stem::new()),
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
        // CRITICAL: If node is an ancestor of new_parent, we must replace node with
        // a placeholder FIRST. Otherwise, insert_before would create a cycle in the
        // tree (indextree's insert_before doesn't check for cycles like append does).
        //
        // Example: moving A under C when the tree is A -> B -> C
        //   1. Replace A with placeholder: (P) -> B -> C, A is detached
        //   2. Now we can safely insert A under C: (P) -> B -> C -> A
        let is_ancestor = self.is_ancestor(node, new_parent);
        debug!(
            ?node,
            ?new_parent,
            is_ancestor,
            "move_to_position: checking ancestry"
        );
        if is_ancestor {
            self.replace_with_placeholder(node);
        } else {
            // Check if node is a direct child of a slot (i.e., a slot root).
            // In that case, we just detach it without a placeholder.
            let parent = self.arena.get(node).and_then(|n| n.parent());
            let is_slot_root = parent.is_some_and(|p| {
                self.arena.get(p).and_then(|n| n.parent()) == Some(self.super_root)
            });

            if !is_slot_root {
                // Node is deeper in the tree - detach with placeholder to prevent shifts
                self.detach_with_placeholder(node);
            } else {
                // Node is a slot root - just detach it
                node.detach(&mut self.arena);
            }
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
                    properties: HtmlProps { attrs: Vec::new() },
                    text: Some(Stem::new()),
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
fn convert_ops_with_shadow<'a, T: DiffTree<Types = HtmlTreeTypes<'a>>>(
    ops: Vec<EditOp<HtmlTreeTypes<'a>>>,
    tree_a: &Tree<HtmlTreeTypes<'a>>,
    tree_b: &T,
    matching: &Matching,
) -> Result<Vec<Patch<'a>>, DiffError> {
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
                    text: tree_b.text(node_b).cloned(),
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
                        let text = tree_b.text(node_b).cloned().unwrap_or_default();
                        result.push(Patch::InsertText {
                            at,
                            text,
                            detach_to_slot,
                        });
                    }
                    HtmlNodeKind::Comment => {
                        let text = tree_b.text(node_b).cloned().unwrap_or_default();
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

                // CRITICAL: If node_a is an ancestor of shadow_new_parent, we need special handling.
                // In the DOM, moving a node moves its entire subtree. So we can't directly move
                // an ancestor under its descendant - we'd be moving the descendant too!
                //
                // Solution: First move all children of node_a to node_a's parent position,
                // then move the (now childless) node_a under the descendant.
                let is_ancestor = shadow.is_ancestor(node_a, shadow_new_parent);
                if is_ancestor {
                    debug!(
                        ?node_a,
                        ?shadow_new_parent,
                        "Move: ancestor case - reparenting children first"
                    );

                    // Get the parent of node_a and the position of node_a within that parent
                    let node_a_parent = shadow
                        .arena
                        .get(node_a)
                        .and_then(|n| n.parent())
                        .expect("node_a should have a parent");
                    let node_a_position = node_a_parent
                        .children(&shadow.arena)
                        .position(|c| c == node_a)
                        .unwrap_or(0);

                    // Collect children of node_a (they need to be moved to node_a's position)
                    let children: Vec<_> = node_a.children(&shadow.arena).collect();

                    // Move each child to node_a's parent, right after node_a's position
                    // We process in reverse order so positions stay stable
                    for (i, child) in children.iter().enumerate().rev() {
                        let child_from = shadow.get_node_ref(*child);
                        let child_position = node_a_position + 1 + i;

                        // Move child in shadow tree (simple detach since we're moving to sibling position)
                        shadow.detach_with_placeholder(*child);

                        // Insert after node_a
                        let siblings: Vec<_> = node_a_parent.children(&shadow.arena).collect();
                        if child_position < siblings.len() {
                            siblings[child_position].insert_before(*child, &mut shadow.arena);
                        } else {
                            node_a_parent.append(*child, &mut shadow.arena);
                        }

                        let child_to =
                            shadow.get_node_ref_with_position(node_a_parent, child_position);

                        debug!(
                            ?child,
                            ?child_from,
                            ?child_to,
                            "Move: reparenting child of ancestor"
                        );

                        result.push(Patch::Move {
                            from: child_from,
                            to: child_to,
                            detach_to_slot: None,
                        });
                    }

                    #[cfg(test)]
                    shadow.debug_print_tree("After reparenting children");
                }

                // Get source reference (after any child reparenting)
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

            EditOp::SetText {
                node_a,
                node_b: _,
                text,
            } => {
                // Path to the text/comment node
                let path = shadow.compute_path(node_a);

                result.push(Patch::SetText {
                    path: NodePath(path),
                    text,
                });
                // No structural change for SetText
            }
        }
    }

    Ok(result)
}

/// Extract attributes and children from a node in tree_b.
fn extract_content_from_tree_b<'a, T: DiffTree<Types = HtmlTreeTypes<'a>>>(
    node_b: NodeId,
    tree_b: &T,
    b_to_shadow: &HashMap<NodeId, NodeId>,
    nodes_with_insert_ops: &HashSet<NodeId>,
) -> (Vec<AttrPair<'a>>, Vec<InsertContent<'a>>) {
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
                let text = tree_b.text(child_id).cloned().unwrap_or_default();
                children.push(InsertContent::Text(text));
            }
            HtmlNodeKind::Comment => {
                let text = tree_b.text(child_id).cloned().unwrap_or_default();
                children.push(InsertContent::Comment(text));
            }
        }
    }

    (attrs, children)
}

// ============================================================================
// into_owned implementations for serialization across lifetime boundaries
// ============================================================================

impl<'a> AttrPair<'a> {
    /// Convert to an owned version with 'static lifetime.
    pub fn into_owned(self) -> AttrPair<'static> {
        AttrPair {
            name: self.name,
            value: self.value.into_owned(),
        }
    }
}

impl<'a> InsertContent<'a> {
    /// Convert to an owned version with 'static lifetime.
    pub fn into_owned(self) -> InsertContent<'static> {
        match self {
            InsertContent::Element {
                tag,
                attrs,
                children,
            } => InsertContent::Element {
                tag,
                attrs: attrs.into_iter().map(|a| a.into_owned()).collect(),
                children: children.into_iter().map(|c| c.into_owned()).collect(),
            },
            InsertContent::Text(s) => InsertContent::Text(s.into_owned()),
            InsertContent::Comment(s) => InsertContent::Comment(s.into_owned()),
        }
    }
}

impl<'a> PropChange<'a> {
    /// Convert to an owned version with 'static lifetime.
    pub fn into_owned(self) -> PropChange<'static> {
        PropChange {
            name: self.name,
            value: self.value.map(|s| s.into_owned()),
        }
    }
}

impl<'a> Patch<'a> {
    /// Convert to an owned version with 'static lifetime.
    pub fn into_owned(self) -> Patch<'static> {
        match self {
            Patch::InsertElement {
                at,
                tag,
                attrs,
                children,
                detach_to_slot,
            } => Patch::InsertElement {
                at,
                tag,
                attrs: attrs.into_iter().map(|a| a.into_owned()).collect(),
                children: children.into_iter().map(|c| c.into_owned()).collect(),
                detach_to_slot,
            },
            Patch::InsertText {
                at,
                text,
                detach_to_slot,
            } => Patch::InsertText {
                at,
                text: text.into_owned(),
                detach_to_slot,
            },
            Patch::InsertComment {
                at,
                text,
                detach_to_slot,
            } => Patch::InsertComment {
                at,
                text: text.into_owned(),
                detach_to_slot,
            },
            Patch::Remove { node } => Patch::Remove { node },
            Patch::SetText { path, text } => Patch::SetText {
                path,
                text: text.into_owned(),
            },
            Patch::SetAttribute { path, name, value } => Patch::SetAttribute {
                path,
                name,
                value: value.into_owned(),
            },
            Patch::RemoveAttribute { path, name } => Patch::RemoveAttribute { path, name },
            Patch::Move {
                from,
                to,
                detach_to_slot,
            } => Patch::Move {
                from,
                to,
                detach_to_slot,
            },
            Patch::UpdateProps { path, changes } => Patch::UpdateProps {
                path,
                changes: changes.into_iter().map(|c| c.into_owned()).collect(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom;
    use facet_testhelpers::test;

    /// Helper to create a StrTendril from a string
    fn t(s: &str) -> StrTendril {
        StrTendril::from(s)
    }

    #[test]
    fn test_build_tree_simple() {
        let html = t("<html><body><div>hello</div></body></html>");
        let doc = dom::parse(&html);
        let tree = build_tree_from_arena(&doc);

        // Root is body element (build_tree_from_arena uses body as root)
        let root_data = tree.get(tree.root);
        assert!(matches!(&root_data.kind, HtmlNodeKind::Element(t, _) if t.as_ref() == "body"));

        // One child (div)
        assert_eq!(tree.child_count(tree.root), 1);
    }

    #[test]
    fn test_diff_text_change() {
        let old_html = t("<html><body><div>old</div></body></html>");
        let new_html = t("<html><body><div>new</div></body></html>");

        let old = dom::parse(&old_html);
        let new = dom::parse(&new_html);
        let patches = diff(&old, &new).unwrap();

        // Should have a SetText patch for the text change
        let has_text_update = patches.iter().any(|p| matches!(p, Patch::SetText { .. }));
        assert!(
            has_text_update,
            "Expected SetText patch, got: {:?}",
            patches
        );
    }

    #[test]
    fn test_diff_attr_change() {
        let old_html = t(r#"<html><body><div class="foo"></div></body></html>"#);
        let new_html = t(r#"<html><body><div class="bar"></div></body></html>"#);

        let old = dom::parse(&old_html);
        let new = dom::parse(&new_html);
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
        let old_html = t("<html><body><span></span></body></html>");
        let new_html = t("<html><body></body></html>");

        let old = dom::parse(&old_html);
        let new = dom::parse(&new_html);
        let patches = diff(&old, &new).unwrap();
        debug!("Patches: {:#?}", patches);

        // Should be able to apply the patches
        let mut doc = dom::parse(&old_html);
        doc.apply_patches(patches).expect("apply should succeed");

        let result = doc.to_html();
        let expected = dom::parse(&new_html).to_html();
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_diff_complex_fuzzer_case() {
        // Fuzzer found: <body><strong>old_text</strong></body> -> <body>new_text<strong>updated</strong></body>
        let old_html = t("<html><body><strong>old</strong></body></html>");
        let new_html = t("<html><body>new_text<strong>updated</strong></body></html>");

        let old = dom::parse(&old_html);
        let new = dom::parse(&new_html);
        let patches = diff(&old, &new).unwrap();
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(&old_html);
        doc.apply_patches(patches).expect("apply should succeed");

        let result = doc.to_html();
        let expected = dom::parse(&new_html).to_html();
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
            t("<html><body><strong>text1</strong><strong>text2</strong><img></body></html>");
        let new_html = t("<html><body>text3<strong>text4</strong></body></html>");

        let old = dom::parse(&old_html);
        let new = dom::parse(&new_html);
        let patches = diff(&old, &new).unwrap();
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(&old_html);
        doc.apply_patches(patches).expect("apply should succeed");

        let result = doc.to_html();
        let expected = dom::parse(&new_html).to_html();
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
        let old_html = t(
            r#"<html><body><strong>n<&nhnnz"""" v</strong><strong>< bit<jva       xx a ></strong><img src="n" alt="v"></body></html>"#,
        );
        let new_html = t(r#"<html><body>n<strong>aaa</strong></body></html>"#);

        let patches = super::diff_html(&old_html, &new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(&old_html);
        trace!("Old tree: {:#?}", doc);

        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected = dom::parse(&new_html).to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_img_li_roundtrip() {
        // Fuzzer found roundtrip failure with img and li elements
        let old_html = t(
            r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://www.w3.org/TR/html4/strict.dtd"><html><body><p>Unclosed paragraph<span class=""></span><div>Inside P which browsers will auto-close</div><span>Unclosed span<div>Block in span</div></body></html>"#,
        );
        let new_html = t(
            r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://www.w3.org/TR/html4/strict.dtd"><html><body><img src="vvv" alt="ttt">d <li>d<<<&<<a"d <<<</li></body></html>"#,
        );

        let patches = super::diff_html(&old_html, &new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(&old_html);
        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected_doc = dom::parse(&new_html);
        let expected = expected_doc.to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_nested_ul_remove() {
        // Fuzzer found issue with nested ul elements - img not being removed correctly
        let old_html = t(
            r#"<!DOCTYPE html><html><body><ul class="h"><ul class="z"><img src="vvv" alt="wvv"><ul class="h"><img src="vvv"></ul></ul></ul></body></html>"#,
        );
        let new_html = t(
            r#"<!DOCTYPE html><html><body><ul class="h"><ul class="h"></ul><ul class="q"><img src="aaa"></ul></ul></body></html>"#,
        );

        let patches = super::diff_html(&old_html, &new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(&old_html);
        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected_doc = dom::parse(&new_html);
        let expected = expected_doc.to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_em_li_navigate_text() {
        // Fuzzer found "Insert: cannot navigate through text node" error
        // The fuzzer generates unescaped HTML which html5ever parses as actual elements
        let old_html = t(r#"<!DOCTYPE html><html><body><em> <v<      << v</em></body></html>"#);
        let new_html =
            t(r#"<!DOCTYPE html><html><body><li>a< <v<      <<</li><img src=""></body></html>"#);

        let old_doc = dom::parse(&old_html);
        let new_doc = dom::parse(&new_html);
        debug!("Old HTML parsed: {:#?}", old_doc);
        debug!("New HTML parsed: {:#?}", new_doc);

        let patches = super::diff_html(&old_html, &new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(&old_html);
        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected = dom::parse(&new_html).to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_nested_ol_patch_order() {
        // Fuzzer found "patch at index 4 is out of order" error
        let old_html = t(r#"<!DOCTYPE html><html><body><ol start="0"></ol></body></html>"#);
        let new_html = t(
            r#"<!DOCTYPE html><html><body><ol start="255"></ol><ol start="93"></ol><ol start="91"><ol start="1"><a href="vaaaaaaaaaaaaa"></a></ol></ol></body></html>"#,
        );

        let patches = super::diff_html(&old_html, &new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(&old_html);
        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected_doc = dom::parse(&new_html);
        let expected = expected_doc.to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_slot_contains_text() {
        // Fuzzer found "Move: slot contains text, cannot navigate to child" error
        let old_html = t(
            r#"<!DOCTYPE html><html><body><article><code><</code><code><</code><code><</code><code><</code></article></body></html>"#,
        );
        let new_html = t(
            r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd"><html><body><code><</code><code><</code><code><</code><code><</code><article><code><</code><code><</code><h2><<<<<<<<<<<<<</h2></article></body></html>"#,
        );

        let patches = super::diff_html(&old_html, &new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(&old_html);
        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected = dom::parse(&new_html).to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_fuzzer_article_code_move() {
        // Fuzzer found "Move: slot contains text, cannot navigate to child"
        // Fuzzer generates unescaped HTML
        let old_html = t(
            r#"<!DOCTYPE html><html><body><article><code><</code><code><</code><code><</code><code><</code><article><article><code><</code></article></article></article></body></html>"#,
        );
        let new_html = t(
            r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd"><html><body><code><</code><code><</code><code><</code><code><</code><article><code><</code><code><</code><h2><<<<<<<<<<<<<</h2></article></body></html>"#,
        );

        let patches = super::diff_html(&old_html, &new_html).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        let mut doc = dom::parse(&old_html);
        doc.apply_patches(patches).expect("apply failed");

        let result = doc.to_html();
        let expected = dom::parse(&new_html).to_html();

        debug!("Result: {}", result);
        debug!("Expected: {}", expected);
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_arena_dom_diff() {
        // Test diffing with arena_dom documents
        let old_html = t("<html><body><div>Old content</div></body></html>");
        let new_html = t("<html><body><div>New content</div></body></html>");

        let old_doc = dom::parse(&old_html);
        let new_doc = dom::parse(&new_html);

        let patches = diff(&old_doc, &new_doc).expect("diff failed");
        debug!("Patches: {:#?}", patches);

        // Verify correctness by applying patches and comparing result
        let mut doc = dom::parse(&old_html);
        doc.apply_patches(patches).expect("apply failed");
        let result = doc.to_html();
        let expected = dom::parse(&new_html).to_html();
        assert_eq!(result, expected, "HTML output should match");
    }

    #[test]
    fn test_arena_dom_diff_add_element() {
        let old_html = t("<html><body><div>Content</div></body></html>");
        let new_html = t("<html><body><div>Content</div><p>New paragraph</p></body></html>");

        let old_doc = dom::parse(&old_html);
        let new_doc = dom::parse(&new_html);

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

        let old_html = t(r#"<html><body><div>Content</div></body></html>"#);
        let new_html = t(r#"<html><body><div class="highlight">Content</div></body></html>"#);

        let patches = diff_html(&old_html, &new_html).expect("diff should work");

        let json = facet_json::to_string(&patches).expect("serialization should work");
        let roundtrip: Vec<Patch> =
            facet_json::from_str(&json).expect("deserialization should work");
        assert_eq!(patches, roundtrip);
    }

    #[test]
    fn test_fuzz_seed_0_template_0() {
        use crate::diff_html;

        let old_html = t(r##"<article>
    <h1>Article Title</h1>
    <p>First paragraph with <strong>bold</strong> and <em>italic</em> text.</p>
    <p>Second paragraph with a <a href="#">link</a>.</p>
  </article>"##);

        let new_html = t(r##"<article>

    <p>First paragraph with <strong>bold</strong> and content</p>
    <p data-test="hidden">Second paragraph with a .</p>
  </article>"##);

        let patches = diff_html(&old_html, &new_html).expect("diff should work");
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

        let old_tendril = t(old_html);
        let new_tendril = t(new_html);
        let patches = diff_html(&old_tendril, &new_tendril).expect("diff should work");
        debug!("Patches: {:#?}", patches);

        for (i, patch) in patches.iter().enumerate() {
            debug!("Patch {}: {:?}", i, patch);
        }

        // Apply all patches at once
        let full_html = t(&format!("<html><body>{}</body></html>", old_html));
        let mut doc = dom::parse(&full_html);
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
        let old_tendril = t(xxl_html);
        let new_tendril = t(&modified);
        let old = dom::parse(&old_tendril);
        let new = dom::parse(&new_tendril);
        let _patches = diff(&old, &new).expect("diff failed");

        // Get stats
        let (calls, scanned) = get_position_stats();

        trace!("\n=== XXL document diff position() stats ===");
        trace!("  position() calls: {}", calls);
        trace!("  siblings scanned: {}", scanned);
        if calls > 0 {
            trace!(
                "  avg siblings per call: {:.2}",
                scanned as f64 / calls as f64
            );
        }
        trace!("===========================================\n");
    }

    /// Regression test for facet-json bug: escape sequences after multi-byte UTF-8
    /// were incorrectly deserialized. Fixed in https://github.com/facet-rs/facet/pull/1892
    #[test]
    fn test_facet_json_unicode_escape_bug() {
        // When Unicode characters precede an escape sequence like \n,
        // facet-json incorrectly deserializes it as literal backslash-n
        let original = "中文\n".to_string();
        debug!("Original bytes: {:?}", original.as_bytes());
        // Original bytes: [228, 184, 173, 230, 150, 135, 10]

        let json = facet_json::to_string(&original).expect("serialize");
        debug!("JSON: {}", json);

        let roundtrip: String = facet_json::from_str(&json).expect("deserialize");
        debug!("Roundtrip bytes: {:?}", roundtrip.as_bytes());
        // BUG: Roundtrip bytes: [228, 184, 173, 230, 150, 135, 92, 110]
        //                                                    ^^ literal '\n'

        assert_eq!(original, roundtrip, "String should roundtrip through JSON");
    }

    #[test]
    fn test_string_ascii_only_multiple_newlines() {
        // ASCII-only strings with escapes work correctly
        let original = "\n  hello\n  world\n".to_string();

        let json = facet_json::to_string(&original).expect("serialize");
        let roundtrip: String = facet_json::from_str(&json).expect("deserialize");

        assert_eq!(
            original, roundtrip,
            "ASCII string should roundtrip through JSON"
        );
    }

    // =========================================================================
    // HTML5 Spec: Foster Parenting Tests
    // https://html.spec.whatwg.org/multipage/parsing.html#foster-parent
    //
    // When content appears in invalid positions inside tables (e.g., text or
    // inline elements directly inside <table>, <tbody>, <tr>, etc.), the HTML5
    // spec requires "foster parenting" - moving that content before the table.
    // =========================================================================

    /// Basic foster parenting: element inside table
    #[test]
    fn test_foster_parent_element_in_table() {
        use crate::dom;

        // Invalid HTML: <span> directly inside <table> triggers foster parenting
        let html = "<table><span>hello</span><tr><td>cell</td></tr></table>";
        let full_html = t(&format!("<html><body>{}</body></html>", html));
        let doc = dom::parse(&full_html);
        let result = doc.to_html();

        // The span should be foster-parented BEFORE the table
        assert!(
            result.contains("<span>hello</span><table>"),
            "Span should appear before table, got: {}",
            result
        );
    }

    /// Foster parenting: text node inside table
    #[test]
    fn test_foster_parent_text_in_table() {
        use crate::dom;

        // Text directly in table gets foster parented
        let html = "<table>orphan text<tr><td>cell</td></tr></table>";
        let full_html = t(&format!("<html><body>{}</body></html>", html));
        let doc = dom::parse(&full_html);
        let result = doc.to_html();

        // The text should appear before the table
        assert!(
            result.contains("orphan text<table>"),
            "Text should be foster-parented before table, got: {}",
            result
        );
    }

    /// Foster parenting: multiple items
    #[test]
    fn test_foster_parent_multiple_items() {
        use crate::dom;

        // Multiple items that need foster parenting
        let html = "<table><b>bold</b><i>italic</i><tr><td>cell</td></tr></table>";
        let full_html = t(&format!("<html><body>{}</body></html>", html));
        let doc = dom::parse(&full_html);
        let result = doc.to_html();

        // Both elements should be before the table, in order
        assert!(
            result.contains("<b>bold</b><i>italic</i><table>"),
            "Both elements should be foster-parented before table, got: {}",
            result
        );
    }

    /// HTML5 spec example: `<table><b><tr><td>aaa</td></tr>bbb</table>ccc`
    /// From: https://html.spec.whatwg.org/multipage/parsing.html#foster-parent
    #[test]
    fn test_foster_parent_spec_example() {
        use crate::dom;

        // This is the exact example from the HTML5 spec
        let html = "<table><b><tr><td>aaa</td></tr>bbb</table>ccc";
        let full_html = t(&format!("<html><body>{}</body></html>", html));
        let doc = dom::parse(&full_html);
        let result = doc.to_html();

        // Per the spec, this should produce:
        // <b></b><b>bbb</b><table><tbody><tr><td>aaa</td></tr></tbody></table><b>ccc</b>
        //
        // The <b> is opened before table (foster parented), empty because it's
        // immediately followed by <tr>. The "bbb" text after </tr> is also foster
        // parented (with the <b> formatting inherited). The "ccc" after </table>
        // keeps the <b> formatting.

        // Check key structural elements
        assert!(
            result.contains("<tbody><tr><td>aaa</td></tr></tbody>"),
            "Table content should be properly structured, got: {}",
            result
        );
        assert!(
            result.contains("bbb"),
            "Foster parented text 'bbb' should exist, got: {}",
            result
        );
        assert!(
            result.contains("ccc"),
            "Text 'ccc' after table should exist, got: {}",
            result
        );
    }

    /// Foster parenting with nested tables
    #[test]
    fn test_foster_parent_nested_tables() {
        use crate::dom;

        // Inner table content should only affect the innermost table
        let html =
            "<table><tr><td><table><span>inner</span><tr><td>x</td></tr></table></td></tr></table>";
        let full_html = t(&format!("<html><body>{}</body></html>", html));
        let doc = dom::parse(&full_html);
        let result = doc.to_html();

        // The span should be foster parented before the inner table, not the outer
        assert!(
            result.contains("<span>inner</span><table>"),
            "Span should be foster-parented before inner table, got: {}",
            result
        );
        // The outer table structure should remain intact
        assert!(
            result.contains("<table><tbody><tr><td>"),
            "Outer table structure should be preserved, got: {}",
            result
        );
    }

    /// Foster parenting: content in tbody
    #[test]
    fn test_foster_parent_content_in_tbody() {
        use crate::dom;

        // Content directly in tbody (outside tr) also triggers foster parenting
        let html = "<table><tbody><span>orphan</span><tr><td>cell</td></tr></tbody></table>";
        let full_html = t(&format!("<html><body>{}</body></html>", html));
        let doc = dom::parse(&full_html);
        let result = doc.to_html();

        // The span should be foster parented before the table
        assert!(
            result.contains("<span>orphan</span><table>"),
            "Span in tbody should be foster-parented before table, got: {}",
            result
        );
    }

    /// Foster parenting: content in tr (outside td)
    #[test]
    fn test_foster_parent_content_in_tr() {
        use crate::dom;

        // Content directly in tr (outside td/th) triggers foster parenting
        let html = "<table><tr><span>orphan</span><td>cell</td></tr></table>";
        let full_html = t(&format!("<html><body>{}</body></html>", html));
        let doc = dom::parse(&full_html);
        let result = doc.to_html();

        // The span should be foster parented before the table
        assert!(
            result.contains("<span>orphan</span><table>"),
            "Span in tr should be foster-parented before table, got: {}",
            result
        );
    }

    /// Foster parenting preserves element order
    #[test]
    fn test_foster_parent_preserves_order() {
        use crate::dom;

        let html = "<table><a>1</a><b>2</b><c>3</c><tr><td>x</td></tr></table>";
        let full_html = t(&format!("<html><body>{}</body></html>", html));
        let doc = dom::parse(&full_html);
        let result = doc.to_html();

        // Elements should appear in the same order they were in the source
        let a_pos = result.find("<a>1</a>").expect("should find a");
        let b_pos = result.find("<b>2</b>").expect("should find b");
        let c_pos = result.find("<c>3</c>").expect("should find c");
        let table_pos = result.find("<table>").expect("should find table");

        assert!(
            a_pos < b_pos && b_pos < c_pos && c_pos < table_pos,
            "Elements should be in order before table, got: {}",
            result
        );
    }

    /// Foster parenting: whitespace handling
    #[test]
    fn test_foster_parent_whitespace() {
        use crate::dom;

        // Whitespace-only text nodes in certain positions are handled specially
        // but significant text gets foster parented
        let html = "<table>  significant  <tr><td>cell</td></tr></table>";
        let full_html = t(&format!("<html><body>{}</body></html>", html));
        let doc = dom::parse(&full_html);
        let result = doc.to_html();

        // The text should be before the table
        assert!(
            result.contains("significant") && result.find("significant") < result.find("<table>"),
            "Significant text should be foster-parented before table, got: {}",
            result
        );
    }

    /// Regression test for patch JSON roundtrip with Unicode.
    /// Fixed in https://github.com/facet-rs/facet/pull/1892
    #[test]
    fn test_patch_json_roundtrip_with_unicode() {
        use crate::diff_html;
        use crate::dom;

        let old_html = t("<my-header>\n      <h1>Title</h1>\n    </my-header>");
        let new_html = t("<my-header>\n      中文App Title\n    </my-header>");

        let patches = diff_html(&old_html, &new_html).expect("diff should work");
        let json = facet_json::to_string(&patches).expect("serialization should work");
        let roundtrip: Vec<Patch> =
            facet_json::from_str(&json).expect("deserialization should work");

        assert_eq!(patches, roundtrip, "Patches should roundtrip through JSON");

        let old_full = t(&format!(
            "<html><body>{}</body></html>",
            old_html.as_ref() as &str
        ));
        let mut doc = dom::parse(&old_full);
        doc.apply_patches(roundtrip).expect("apply should succeed");

        let result = doc.to_html();
        let new_full = t(&format!(
            "<html><body>{}</body></html>",
            new_html.as_ref() as &str
        ));
        let expected = dom::parse(&new_full).to_html();
        assert_eq!(result, expected, "HTML output should match");
    }

    /// Test SVG inside strong - reproduction of fuzz seed 1 failure
    #[test]
    fn test_svg_inside_strong() {
        use crate::diff_html;
        use crate::dom;

        // Simplified version of the failing case
        let old_html = r#"<p>Text with <strong>bold</strong> word.</p>"#;
        let new_html = r#"<p>Text with <strong><svg width="29" height="21"><circle></circle></svg></strong> word.</p>"#;

        trace!("=== Old HTML ===");
        trace!("{}", old_html);
        trace!("\n=== New HTML ===");
        trace!("{}", new_html);

        let old_full = t(&format!("<html><body>{}</body></html>", old_html));
        let new_full = t(&format!("<html><body>{}</body></html>", new_html));
        let old_parsed = dom::parse(&old_full);
        let new_parsed = dom::parse(&new_full);

        trace!("\n=== Old parsed ===");
        trace!("{}", old_parsed.to_html());
        trace!("\n=== New parsed ===");
        trace!("{}", new_parsed.to_html());

        let patches = diff_html(&old_full, &new_full).expect("diff should work");

        trace!("\n=== Patches ===");
        for (i, patch) in patches.iter().enumerate() {
            trace!("{}: {:?}", i, patch);
        }

        let mut doc = dom::parse(&old_full);
        doc.apply_patches(patches).expect("apply should succeed");

        let result = doc.to_html();
        let expected = new_parsed.to_html();

        trace!("\n=== After patches ===");
        trace!("{}", result);
        trace!("\n=== Expected ===");
        trace!("{}", expected);

        assert_eq!(result, expected, "HTML output should match");
    }

    /// Test: adoption agency algorithm difference with block elements
    ///
    /// When you put a block element (like `<section>`) inside a formatting element
    /// (like `<strong>`) inside `<p>`, HTML5 requires complex "adoption agency"
    /// handling. We should preserve the formatting element across the block boundary
    /// so that subsequent SVG is wrapped correctly.
    ///
    /// Browser: maintains `<strong>` context after `</section>`, wrapping subsequent SVG
    /// html5ever: now matches browser behavior here.
    #[test]
    fn test_adoption_agency_block_in_formatting() {
        use crate::dom;

        // When you put <section> inside <p>, the browser closes the <p> first
        // This tests HTML5 adoption agency algorithm behavior
        let html = r#"<p>First with <strong>text<section>break</section><svg width="29"><circle></circle></svg></strong> end.</p>"#;

        let full_html = t(&format!("<html><body>{}</body></html>", html));
        let doc = dom::parse(&full_html);
        let result = doc.to_html();

        // The <section> should have been moved outside <p> due to HTML5 parsing rules
        assert!(
            !result.contains("<p><strong>text<section>"),
            "Section should not remain inside p>strong (invalid nesting), got: {}",
            result
        );

        // Verify our output matches the corrected html5ever behavior.
        // The SVG should be wrapped by the active formatting element.
        assert_eq!(
            result,
            "<html><head></head><body><p>First with <strong>text</strong></p>\
             <section><strong>break</strong></section>\
             <strong><svg width=\"29\"><circle></circle></svg></strong> end.<p></p></body></html>",
            "Output should match html5ever behavior"
        );
    }

    /// Regression test for OOM caused by cycle in parent chain.
    /// When moving a node under its own descendant, we must not create a cycle.
    #[test]
    fn test_move_parent_under_child_no_cycle() {
        // Build a tree: A -> B -> C (A is grandparent of C)
        // Then try to move A to position 0 under B, displacing C.
        // This triggers insert_before which doesn't check for cycles.
        let mut arena: indextree::Arena<NodeData<HtmlTreeTypes>> = indextree::Arena::new();

        // Create nodes: body -> div_a -> div_b -> div_c
        let body = arena.new_node(NodeData {
            hash: NodeHash(0),
            kind: HtmlNodeKind::Element(LocalName::from("body"), Namespace::Html),
            properties: HtmlProps::default(),
            text: None,
        });
        let div_a = arena.new_node(NodeData {
            hash: NodeHash(0),
            kind: HtmlNodeKind::Element(LocalName::from("div"), Namespace::Html),
            properties: HtmlProps::default(),
            text: None,
        });
        let div_b = arena.new_node(NodeData {
            hash: NodeHash(0),
            kind: HtmlNodeKind::Element(LocalName::from("div"), Namespace::Html),
            properties: HtmlProps::default(),
            text: None,
        });
        let div_c = arena.new_node(NodeData {
            hash: NodeHash(0),
            kind: HtmlNodeKind::Element(LocalName::from("div"), Namespace::Html),
            properties: HtmlProps::default(),
            text: None,
        });

        body.append(div_a, &mut arena);
        div_a.append(div_b, &mut arena);
        div_b.append(div_c, &mut arena);

        // Structure: body -> div_a -> div_b -> div_c
        let mut shadow = ShadowTree::new(arena, body);

        shadow.debug_print_tree("Initial");

        // Now try to move div_a to position 0 under div_b (moving parent under child)
        // div_c is at position 0, so this triggers insert_before
        // This should NOT create a cycle
        shadow.move_to_position(div_a, div_b, 0);

        shadow.debug_print_tree("After move");

        // Verify no cycle by computing path - this would hang/OOM if there's a cycle
        let path = shadow.compute_path(div_a);
        debug!(?path, "Path to div_a after move");

        // div_a should now be a child of div_b
        assert!(
            div_b.children(&shadow.arena).any(|c| c == div_a),
            "div_a should be a child of div_b after move"
        );
    }
}
