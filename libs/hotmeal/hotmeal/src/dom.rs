//! Arena-based DOM with borrowed strings for zero-copy parsing.
//!
//! This module provides the core Document representation used throughout hotmeal.
//! Key features:
//! - **indextree Arena**: All nodes in contiguous memory (cache-friendly)
//! - **Borrowed strings**: Tags, attributes, text borrowed from source HTML
//! - **Zero conversions**: Same representation used by parser, differ, and patch applier

use cinereus::indextree::{Arena, NodeId};
use html5ever::tree_builder::{ElemName, ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{Attribute, LocalName, QualName, parse_document};
use html5ever::{local_name, ns};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use tendril::{StrTendril, TendrilSink};

use crate::diff::{DiffError, InsertContent, NodeRef, Patch, PropKey};
use crate::{Stem, debug};

/// Document = Arena (strings are StrTendrils with refcounted sharing)
#[derive(Debug, Clone)]
pub struct Document {
    /// THE tree - all nodes live here
    pub arena: Arena<NodeData>,

    /// Root node (usually `<html>` element)
    pub root: NodeId,

    /// DOCTYPE if present (usually "html")
    pub doctype: Option<Stem>,
}

impl Document {
    /// Create an empty document with just `<html><head></head><body></body></html>`
    pub fn new() -> Self {
        let mut arena = Arena::new();

        // Create html element
        let html = arena.new_node(NodeData {
            kind: NodeKind::Element(ElementData {
                tag: LocalName::from("html"),
                attrs: Vec::new(),
            }),
            ns: Namespace::Html,
        });

        // Create head element
        let head = arena.new_node(NodeData {
            kind: NodeKind::Element(ElementData {
                tag: LocalName::from("head"),
                attrs: Vec::new(),
            }),
            ns: Namespace::Html,
        });
        html.append(head, &mut arena);

        // Create body element
        let body = arena.new_node(NodeData {
            kind: NodeKind::Element(ElementData {
                tag: LocalName::from("body"),
                attrs: Vec::new(),
            }),
            ns: Namespace::Html,
        });
        html.append(body, &mut arena);

        Document {
            arena,
            root: html,
            doctype: None,
        }
    }

    /// Get immutable reference to node data
    pub fn get(&self, id: NodeId) -> &NodeData {
        self.arena[id].get()
    }

    /// Get mutable reference to node data
    pub fn get_mut(&mut self, id: NodeId) -> &mut NodeData {
        self.arena[id].get_mut()
    }

    /// Iterate children of a node
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        id.children(&self.arena)
    }

    /// Get the `<body>` element if present
    pub fn body(&self) -> Option<NodeId> {
        self.root.children(&self.arena).find(|&id| {
            if let NodeKind::Element(elem) = &self.arena[id].get().kind {
                elem.tag.as_ref() == "body"
            } else {
                false
            }
        })
    }

    /// Get the `<head>` element if present
    pub fn head(&self) -> Option<NodeId> {
        self.root.children(&self.arena).find(|&id| {
            if let NodeKind::Element(elem) = &self.arena[id].get().kind {
                elem.tag.as_ref() == "head"
            } else {
                false
            }
        })
    }

    // ==================== DOM Manipulation API ====================

    /// Create an element node (not yet attached to the tree)
    pub fn create_element(&mut self, tag: impl Into<LocalName>) -> NodeId {
        self.arena.new_node(NodeData {
            kind: NodeKind::Element(ElementData {
                tag: tag.into(),
                attrs: Vec::new(),
            }),
            ns: Namespace::Html,
        })
    }

    /// Create a text node (not yet attached to the tree)
    pub fn create_text(&mut self, text: impl Into<Stem>) -> NodeId {
        self.arena.new_node(NodeData {
            kind: NodeKind::Text(text.into()),
            ns: Namespace::Html,
        })
    }

    /// Create a comment node (not yet attached to the tree)
    pub fn create_comment(&mut self, text: impl Into<Stem>) -> NodeId {
        self.arena.new_node(NodeData {
            kind: NodeKind::Comment(text.into()),
            ns: Namespace::Html,
        })
    }

    /// Append a child node to a parent
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        parent.append(child, &mut self.arena);
    }

    /// Insert a node before a sibling
    pub fn insert_before(&mut self, sibling: NodeId, new_node: NodeId) {
        sibling.insert_before(new_node, &mut self.arena);
    }

    /// Insert a node after a sibling
    pub fn insert_after(&mut self, sibling: NodeId, new_node: NodeId) {
        sibling.insert_after(new_node, &mut self.arena);
    }

    /// Remove a node from its parent (node remains in arena but detached)
    pub fn remove(&mut self, node: NodeId) {
        node.detach(&mut self.arena);
    }

    /// Set an attribute on an element
    pub fn set_attr(&mut self, element: NodeId, name: QualName, value: impl Into<Stem>) {
        if let NodeKind::Element(elem) = &mut self.arena[element].get_mut().kind {
            let value = value.into();
            // Find existing attribute and update, or append new one
            if let Some((_, existing_value)) = elem.attrs.iter_mut().find(|(k, _)| k == &name) {
                *existing_value = value;
            } else {
                elem.attrs.push((name, value));
            }
        }
    }

    /// Remove an attribute from an element
    pub fn remove_attr(&mut self, element: NodeId, name: &QualName) {
        if let NodeKind::Element(elem) = &mut self.arena[element].get_mut().kind {
            elem.attrs.retain(|(k, _)| k != name);
        }
    }

    /// Set the text content of a text node
    pub fn set_text(&mut self, node: NodeId, text: impl Into<Stem>) {
        if let NodeKind::Text(t) = &mut self.arena[node].get_mut().kind {
            *t = text.into();
        }
    }

    /// Get parent of a node
    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.arena[node].parent()
    }

    /// Get first child of a node
    pub fn first_child(&self, node: NodeId) -> Option<NodeId> {
        node.children(&self.arena).next()
    }

    /// Get last child of a node
    pub fn last_child(&self, node: NodeId) -> Option<NodeId> {
        node.children(&self.arena).next_back()
    }

    /// Get next sibling of a node
    pub fn next_sibling(&self, node: NodeId) -> Option<NodeId> {
        self.arena[node].next_sibling()
    }

    /// Get previous sibling of a node
    pub fn prev_sibling(&self, node: NodeId) -> Option<NodeId> {
        self.arena[node].previous_sibling()
    }

    /// Count children of a node
    pub fn child_count(&self, node: NodeId) -> usize {
        node.children(&self.arena).count()
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// Serialize to full HTML string including doctype
    pub fn to_html(&self) -> String {
        let mut output = String::new();
        if let Some(ref doctype) = self.doctype {
            output.push_str("<!DOCTYPE ");
            output.push_str(doctype.as_ref());
            output.push('>');
        }
        self.serialize_node(&mut output, self.root);
        output
    }

    /// Serialize to HTML string without the doctype declaration.
    /// Useful for comparing DOM structure when doctype differences should be ignored.
    pub fn to_html_without_doctype(&self) -> String {
        let mut output = String::new();
        self.serialize_node(&mut output, self.root);
        output
    }

    /// Navigate to a node using the new unified path format.
    /// Path `[slot, a, b, c]` means: get slot root, then navigate a → b → c.
    fn navigate_slot_path(
        &self,
        path: &[u32],
        slots: &HashMap<u32, NodeId>,
    ) -> Result<NodeId, DiffError> {
        if path.is_empty() {
            return Err(DiffError::EmptyPath);
        }

        let slot = path[0];
        let slot_root = *slots.get(&slot).ok_or(DiffError::SlotNotFound { slot })?;

        let mut current = slot_root;
        for &idx in &path[1..] {
            let mut children = current.children(&self.arena);
            current = children
                .nth(idx as usize)
                .ok_or(DiffError::PathOutOfBounds {
                    index: idx as usize,
                })?;
        }

        Ok(current)
    }

    /// Get parent and position from a unified slot path.
    /// Path `[slot, a, b, c]` returns (node at [slot, a, b], position c).
    fn get_slot_parent(
        &self,
        path: &[u32],
        slots: &HashMap<u32, NodeId>,
    ) -> Result<(NodeId, usize), DiffError> {
        if path.len() < 2 {
            return Err(DiffError::EmptyPath);
        }

        let position = path[path.len() - 1] as usize;
        let parent_path = &path[..path.len() - 1];
        let parent_id = self.navigate_slot_path(parent_path, slots)?;

        Ok((parent_id, position))
    }

    /// Apply patches to this document (modifying it in place).
    pub fn apply_patches(&mut self, patches: Vec<Patch>) -> Result<(), DiffError> {
        // Slots hold NodeIds - slot 0 is always the body (main tree)
        let mut slots: HashMap<u32, NodeId> = HashMap::new();
        let body_id = self.body().ok_or(DiffError::NoBody)?;
        slots.insert(0, body_id);

        for patch in patches {
            self.apply_patch(patch, &mut slots)?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn apply_patch(
        &mut self,
        patch: Patch,
        slots: &mut HashMap<u32, NodeId>,
    ) -> Result<(), DiffError> {
        debug!("Applying patch: {:#?}", patch);
        match patch {
            Patch::InsertElement {
                at,
                tag,
                attrs,
                children,
                detach_to_slot,
            } => {
                // Create new element node
                let elem_data = ElementData {
                    tag: tag.clone(),
                    attrs: attrs
                        .iter()
                        .map(|a| (a.name.clone(), a.value.clone()))
                        .collect(),
                };
                let new_node = self.arena.new_node(NodeData {
                    kind: NodeKind::Element(elem_data),
                    ns: Namespace::Html,
                });

                // Add children to the new element
                for child in children {
                    let child_node = self.create_insert_content(child)?;
                    new_node.append(child_node, &mut self.arena);
                }

                self.insert_at(&at, new_node, detach_to_slot, slots)?;
            }
            Patch::InsertText {
                at,
                text,
                detach_to_slot,
            } => {
                let new_node = self.arena.new_node(NodeData {
                    kind: NodeKind::Text(text),
                    ns: Namespace::Html,
                });
                self.insert_at(&at, new_node, detach_to_slot, slots)?;
            }
            Patch::InsertComment {
                at,
                text,
                detach_to_slot,
            } => {
                let new_node = self.arena.new_node(NodeData {
                    kind: NodeKind::Comment(text),
                    ns: Namespace::Html,
                });
                self.insert_at(&at, new_node, detach_to_slot, slots)?;
            }
            Patch::Remove { node } => {
                let path = &node.0.0;
                // Navigate to the node and replace with placeholder
                let node_id = self.navigate_slot_path(path, slots)?;
                let empty_text = self.arena.new_node(NodeData {
                    kind: NodeKind::Text(Stem::new()),
                    ns: Namespace::Html,
                });
                node_id.insert_before(empty_text, &mut self.arena);
                node_id.detach(&mut self.arena);
            }
            Patch::SetText { path, text } => {
                let node_id = self.navigate_slot_path(&path.0, slots)?;
                let node_data = self.arena[node_id].get_mut();
                match &mut node_data.kind {
                    NodeKind::Text(t) => *t = text,
                    NodeKind::Comment(t) => *t = text,
                    _ => return Err(DiffError::NotATextNode),
                }
            }
            Patch::SetAttribute { path, name, value } => {
                let node_id = self.navigate_slot_path(&path.0, slots)?;
                let node_data = self.arena[node_id].get_mut();
                if let NodeKind::Element(elem) = &mut node_data.kind {
                    // Find existing attribute and update, or append new one
                    if let Some((_, existing_value)) =
                        elem.attrs.iter_mut().find(|(k, _)| k == &name)
                    {
                        *existing_value = value;
                    } else {
                        elem.attrs.push((name, value));
                    }
                } else {
                    return Err(DiffError::NotAnElement);
                }
            }
            Patch::RemoveAttribute { path, name } => {
                let node_id = self.navigate_slot_path(&path.0, slots)?;
                let node_data = self.arena[node_id].get_mut();
                if let NodeKind::Element(elem) = &mut node_data.kind {
                    elem.attrs.retain(|(k, _)| k != &name);
                } else {
                    return Err(DiffError::NotAnElement);
                }
            }
            Patch::Move {
                from,
                to,
                detach_to_slot,
            } => {
                let from_path = &from.0.0;
                let node_to_move = self.navigate_slot_path(from_path, slots)?;

                // Replace source position with empty text (no shifting!)
                // Exception: path of length 1 (just [slot]) means the slot root itself
                let needs_replacement = from_path.len() > 1;

                if needs_replacement {
                    let empty_text = self.arena.new_node(NodeData {
                        kind: NodeKind::Text(Stem::new()),
                        ns: Namespace::Html,
                    });
                    node_to_move.insert_before(empty_text, &mut self.arena);
                    node_to_move.detach(&mut self.arena);
                } else {
                    node_to_move.detach(&mut self.arena);
                }

                self.insert_at(&to, node_to_move, detach_to_slot, slots)?;
            }
            Patch::UpdateProps { path, changes } => {
                let node_id = self.navigate_slot_path(&path.0, slots)?;
                let node_data = self.arena[node_id].get_mut();

                // Handle text node updates
                if let Some(text_change) = changes.iter().find(|c| matches!(c.name, PropKey::Text))
                {
                    if let NodeKind::Text(t) = &mut node_data.kind {
                        if let Some(new_text) = &text_change.value {
                            *t = new_text.clone();
                        }
                    } else if let NodeKind::Comment(c) = &mut node_data.kind
                        && let Some(new_text) = &text_change.value
                    {
                        *c = new_text.clone();
                    }
                }

                // Handle element attribute updates
                // The changes vec represents the ENTIRE final attribute state in order
                if let NodeKind::Element(elem) = &mut node_data.kind {
                    // Always rebuild attrs from changes, even if empty (to handle removals)
                    let old_attrs = std::mem::take(&mut elem.attrs);
                    debug!(
                        "UpdateProps: rebuilding attrs old_count={} changes_count={}",
                        old_attrs.len(),
                        changes.len()
                    );

                    for change in changes {
                        if let PropKey::Attr(ref qual_name) = change.name {
                            debug!(
                                "UpdateProps: processing attr {} value={:?}",
                                change.name, change.value
                            );
                            let value = if let Some(new_value) = &change.value {
                                // Different value - use the new one
                                new_value.clone()
                            } else {
                                // Same value - copy from old attrs
                                old_attrs
                                    .iter()
                                    .find(|(k, _)| k == qual_name)
                                    .map(|(_, v)| v.clone())
                                    .unwrap_or_default()
                            };
                            elem.attrs.push((qual_name.clone(), value));
                        }
                    }
                    debug!("UpdateProps: final attrs_count={}", elem.attrs.len());
                    // Attributes not in changes are implicitly removed (we cleared attrs and only
                    // added back what's in changes)
                }
            }
        }

        Ok(())
    }

    fn insert_at(
        &mut self,
        at: &NodeRef,
        node_to_insert: NodeId,
        detach_to_slot: Option<u32>,
        slots: &mut HashMap<u32, NodeId>,
    ) -> Result<(), DiffError> {
        let path = &at.0.0;
        let (parent_id, position) = self.get_slot_parent(path, slots)?;

        debug!(
            "insert_at: path={:?}, parent={:?}, position={}",
            path, parent_id, position
        );

        if let Some(slot) = detach_to_slot {
            let children: Vec<_> = parent_id.children(&self.arena).collect();
            debug!(
                "insert_at: detaching at position {}, children.len()={}",
                position,
                children.len()
            );
            if position < children.len() {
                let displaced = children[position];
                displaced.detach(&mut self.arena);
                slots.insert(slot, displaced);
            }
        }

        self.insert_at_position(parent_id, position, node_to_insert)?;

        Ok(())
    }

    fn insert_at_position(
        &mut self,
        parent_id: NodeId,
        position: usize,
        node_to_insert: NodeId,
    ) -> Result<(), DiffError> {
        let children: Vec<_> = parent_id.children(&self.arena).collect();

        debug!(
            "insert_at_position: parent={:?}, position={}, children.len()={}, node_to_insert={:?}",
            parent_id,
            position,
            children.len(),
            node_to_insert
        );

        // Chawathe semantics: fill gaps with empty text nodes
        // If inserting at position 3 with 0 children, first insert empty text at 0, 1, 2
        #[allow(unused_variables)]
        for i in children.len()..position {
            let empty_text = self.arena.new_node(NodeData {
                kind: NodeKind::Text(Stem::new()),
                ns: Namespace::Html,
            });
            parent_id.append(empty_text, &mut self.arena);
            debug!("Filled gap at position {} with empty text node", i);
        }

        // Now insert at the exact position
        let children: Vec<_> = parent_id.children(&self.arena).collect();
        if position >= children.len() {
            parent_id.append(node_to_insert, &mut self.arena);
        } else {
            let next_sibling = children[position];
            next_sibling.insert_before(node_to_insert, &mut self.arena);
        }

        Ok(())
    }

    fn create_insert_content(&mut self, content: InsertContent) -> Result<NodeId, DiffError> {
        match content {
            InsertContent::Element {
                tag,
                attrs,
                children,
            } => {
                let elem_data = ElementData {
                    tag,
                    attrs: attrs.into_iter().map(|a| (a.name, a.value)).collect(),
                };
                let node = self.arena.new_node(NodeData {
                    kind: NodeKind::Element(elem_data),
                    ns: Namespace::Html,
                });

                for child in children {
                    let child_node = self.create_insert_content(child)?;
                    node.append(child_node, &mut self.arena);
                }

                Ok(node)
            }
            InsertContent::Text(text) => {
                let node = self.arena.new_node(NodeData {
                    kind: NodeKind::Text(text),
                    ns: Namespace::Html,
                });
                Ok(node)
            }
            InsertContent::Comment(text) => {
                let node = self.arena.new_node(NodeData {
                    kind: NodeKind::Comment(text),
                    ns: Namespace::Html,
                });
                Ok(node)
            }
        }
    }

    fn serialize_node(&self, out: &mut String, node_id: NodeId) {
        let node = self.get(node_id);
        match &node.kind {
            NodeKind::Document => {
                // Document nodes are invisible
            }
            NodeKind::Element(elem) => {
                self.serialize_element(out, node_id, elem);
            }
            NodeKind::Text(text) => {
                // Escape text content
                for c in text.as_ref().chars() {
                    match c {
                        '&' => out.push_str("&amp;"),
                        '<' => out.push_str("&lt;"),
                        '>' => out.push_str("&gt;"),
                        _ => out.push(c),
                    }
                }
            }
            NodeKind::Comment(text) => {
                out.push_str("<!--");
                out.push_str(text.as_ref());
                out.push_str("-->");
            }
        }
    }

    fn serialize_element(&self, out: &mut String, node_id: NodeId, elem: &ElementData) {
        let tag = elem.tag.as_ref();

        // Opening tag
        out.push('<');
        out.push_str(tag);

        // Attributes
        for (name, value) in &elem.attrs {
            out.push(' ');
            // Serialize QualName with prefix if present
            if let Some(ref prefix) = name.prefix {
                out.push_str(prefix.as_ref());
                out.push(':');
            }
            out.push_str(name.local.as_ref());
            out.push_str("=\"");
            // Escape attribute value
            for c in value.as_ref().chars() {
                match c {
                    '&' => out.push_str("&amp;"),
                    '"' => out.push_str("&quot;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    _ => out.push(c),
                }
            }
            out.push('"');
        }

        // Check if void element
        if is_void_element(tag) {
            out.push('>');
            return;
        }

        out.push('>');

        // Children
        for child_id in node_id.children(&self.arena) {
            self.serialize_node(out, child_id);
        }

        // Closing tag
        out.push_str("</");
        out.push_str(tag);
        out.push('>');
    }
}

/// HTML5 void elements that never have closing tags
fn is_void_element(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

/// What goes in each arena slot
#[derive(Debug, Clone)]
pub struct NodeData {
    pub kind: NodeKind,
    pub ns: Namespace,
}

/// Node types
#[derive(Debug, Clone)]
pub enum NodeKind {
    /// Document root (invisible, parent of `<html>`)
    Document,
    /// Element with tag and attributes
    Element(ElementData),
    /// Text content (Stem is refcounted - cheap to clone)
    Text(Stem),
    /// HTML comment
    Comment(Stem),
}

/// Element data (tag + attributes)
#[derive(Debug, Clone)]
pub struct ElementData {
    /// Tag name (LocalName is interned via string_cache)
    pub tag: LocalName,

    /// Attributes - Vec preserves insertion order for consistent serialization
    /// Keys are QualName (preserves namespace for xlink:href, xml:lang, etc.), values are Tendril
    pub attrs: Vec<(QualName, Stem)>,
}

/// XML namespace
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
}

impl Namespace {
    pub fn from_url(url: &str) -> Self {
        match url {
            "http://www.w3.org/1999/xhtml" => Namespace::Html,
            "http://www.w3.org/2000/svg" => Namespace::Svg,
            "http://www.w3.org/1998/Math/MathML" => Namespace::MathMl,
            _ => Namespace::Html, // default
        }
    }

    pub fn url(&self) -> &'static str {
        match self {
            Namespace::Html => "http://www.w3.org/1999/xhtml",
            Namespace::Svg => "http://www.w3.org/2000/svg",
            Namespace::MathMl => "http://www.w3.org/1998/Math/MathML",
        }
    }
}

/// Parse HTML into arena-based Document
pub fn parse(html: &str) -> Document {
    let sink = ArenaSink::new();
    // Create a Tendril from our source string
    // html5ever will create subtendrils that share this buffer via refcounting
    let tendril = Stem::from(html);
    parse_document(sink, Default::default()).one(tendril)
}

/// Owned element name wrapper
#[derive(Debug, Clone)]
struct OwnedElemName(QualName);

impl ElemName for OwnedElemName {
    fn ns(&self) -> &html5ever::Namespace {
        &self.0.ns
    }

    fn local_name(&self) -> &LocalName {
        &self.0.local
    }
}

/// TreeSink implementation for building arena-based DOM
struct ArenaSink {
    /// Our arena (same type as everywhere else!) - wrapped in RefCell for interior mutability
    arena: RefCell<Arena<NodeData>>,

    /// Document node (parent of `<html>`)
    document: NodeId,

    /// DOCTYPE encountered during parse - wrapped in RefCell
    doctype: RefCell<Option<Stem>>,
}

impl ArenaSink {
    fn new() -> Self {
        let mut arena = Arena::new();

        // Create document root node
        let document = arena.new_node(NodeData {
            kind: NodeKind::Document,
            ns: Namespace::Html,
        });

        ArenaSink {
            arena: RefCell::new(arena),
            document,
            doctype: RefCell::new(None),
        }
    }
}

impl TreeSink for ArenaSink {
    type Handle = NodeId;
    type Output = Document;
    type ElemName<'a>
        = OwnedElemName
    where
        Self: 'a;

    fn finish(self) -> Self::Output {
        let arena = self.arena.into_inner();

        // Find the root element (usually <html>)
        let root = self
            .document
            .children(&arena)
            .next()
            .unwrap_or(self.document);

        Document {
            arena,
            root,
            doctype: self.doctype.into_inner(),
        }
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {
        // Ignore parse errors (html5ever recovers automatically)
    }

    fn get_document(&self) -> Self::Handle {
        self.document
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {
        // We don't care about quirks mode for diffing
    }

    fn same_node(&self, a: &Self::Handle, b: &Self::Handle) -> bool {
        a == b
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> OwnedElemName {
        let arena = self.arena.borrow();
        let node = &arena[*target].get();

        if let NodeKind::Element(elem) = &node.kind {
            // Clone is just an atomic refcount bump since LocalName is interned
            let local_name = elem.tag.clone();
            let ns = match node.ns {
                Namespace::Html => ns!(html),
                Namespace::Svg => ns!(svg),
                Namespace::MathMl => ns!(mathml),
            };

            OwnedElemName(QualName {
                prefix: None,
                ns,
                local: local_name,
            })
        } else {
            // Not an element - return placeholder
            OwnedElemName(QualName {
                prefix: None,
                ns: ns!(html),
                local: local_name!(""),
            })
        }
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        _flags: ElementFlags,
    ) -> Self::Handle {
        let tag = name.local;
        let ns = Namespace::from_url(name.ns.as_ref());

        let attrs: Vec<_> = attrs
            .into_iter()
            .map(|attr| (attr.name, attr.value))
            .collect();

        // Create node in arena
        self.arena.borrow_mut().new_node(NodeData {
            kind: NodeKind::Element(ElementData { tag, attrs }),
            ns,
        })
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.arena.borrow_mut().new_node(NodeData {
            kind: NodeKind::Comment(text),
            ns: Namespace::Html,
        })
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Self::Handle {
        // Processing instructions - create empty comment
        self.arena.borrow_mut().new_node(NodeData {
            kind: NodeKind::Comment(Stem::new()),
            ns: Namespace::Html,
        })
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        let mut arena = self.arena.borrow_mut();
        match child {
            NodeOrText::AppendNode(node) => {
                parent.append(node, &mut *arena);
            }
            NodeOrText::AppendText(text) => {
                // Try to merge with previous text node (html5ever behavior)
                // First get the last child ID (if any) without holding a borrow
                let last_child_id = parent.children(&arena).next_back();

                if let Some(last_child) = last_child_id {
                    // Now we can safely get mutable access
                    if let NodeKind::Text(existing) = &mut arena[last_child].get_mut().kind {
                        // Merge text - push_tendril shares buffers when possible
                        existing.push_tendril(&text);
                        return;
                    }
                }

                let text_node = arena.new_node(NodeData {
                    kind: NodeKind::Text(text),
                    ns: Namespace::Html,
                });
                parent.append(text_node, &mut arena);
            }
        }
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        let mut arena = self.arena.borrow_mut();
        match new_node {
            NodeOrText::AppendNode(node) => {
                sibling.insert_before(node, &mut *arena);
            }
            NodeOrText::AppendText(text) => {
                // Try to merge with the previous sibling if it's a text node
                // (matching RcDom behavior for proper adoption agency algorithm support)
                if let Some(prev_sibling) = sibling.preceding_siblings(&*arena).next()
                    && let NodeKind::Text(existing) = &mut arena[prev_sibling].get_mut().kind
                {
                    existing.push_tendril(&text);
                    return;
                }

                let text_node = arena.new_node(NodeData {
                    kind: NodeKind::Text(text),
                    ns: Namespace::Html,
                });
                sibling.insert_before(text_node, &mut *arena);
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        // Foster parenting: if the element (table) has a parent, insert before it.
        // Otherwise, append to the previous element in the stack.
        let has_parent = {
            let arena = self.arena.borrow();
            arena[*element].parent().is_some()
        };

        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        *self.doctype.borrow_mut() = Some(name);
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        // For <template>, return the element itself
        // (proper template support would need a template contents fragment)
        *target
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        let mut arena = self.arena.borrow_mut();
        let node = &mut arena[*target].get_mut();
        if let NodeKind::Element(elem) = &mut node.kind {
            for attr in attrs {
                // Only add if not already present
                if !elem.attrs.iter().any(|(k, _)| k == &attr.name) {
                    elem.attrs.push((attr.name, attr.value));
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        target.detach(&mut self.arena.borrow_mut());
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        let mut arena = self.arena.borrow_mut();
        let children: Vec<NodeId> = node.children(&*arena).collect();
        for child in children {
            child.detach(&mut *arena);
            new_parent.append(child, &mut *arena);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_html() {
        let html = "<html><body><p>Hello</p></body></html>";
        let doc = parse(html);

        // Check root is <html>
        let root_data = doc.get(doc.root);
        assert!(matches!(root_data.kind, NodeKind::Element(_)));
        if let NodeKind::Element(elem) = &root_data.kind {
            assert_eq!(elem.tag.as_ref(), "html");
        }

        // Check we have a body
        let body = doc.body().expect("should have body");
        let body_data = doc.get(body);
        if let NodeKind::Element(elem) = &body_data.kind {
            assert_eq!(elem.tag.as_ref(), "body");
        }

        // Check body has a <p> child
        let p = body
            .children(&doc.arena)
            .next()
            .expect("body should have child");
        let p_data = doc.get(p);
        if let NodeKind::Element(elem) = &p_data.kind {
            assert_eq!(elem.tag.as_ref(), "p");
        }

        // Check <p> has text child
        let text = p.children(&doc.arena).next().expect("p should have text");
        let text_data = doc.get(text);
        if let NodeKind::Text(t) = &text_data.kind {
            assert_eq!(t.as_ref(), "Hello");
        }
    }

    #[test]
    fn test_parse_with_attributes() {
        let html = r#"<div class="container" id="main">Content</div>"#;
        let full_html = format!("<html><body>{}</body></html>", html);
        let doc = parse(&full_html);

        let body = doc.body().expect("should have body");
        let div = body
            .children(&doc.arena)
            .next()
            .expect("body should have div");
        let div_data = doc.get(div);

        if let NodeKind::Element(elem) = &div_data.kind {
            assert_eq!(elem.tag.as_ref(), "div");

            // Check attributes (keys are QualName with empty namespace for regular HTML attrs)
            let class_name = QualName::new(None, ns!(), local_name!("class"));
            assert_eq!(
                elem.attrs
                    .iter()
                    .find(|(k, _)| k == &class_name)
                    .map(|(_, v)| v.as_ref()),
                Some("container")
            );
            let id_name = QualName::new(None, ns!(), local_name!("id"));
            assert_eq!(
                elem.attrs
                    .iter()
                    .find(|(k, _)| k == &id_name)
                    .map(|(_, v)| v.as_ref()),
                Some("main")
            );
        }
    }

    #[test]
    fn test_parse_doctype() {
        let html = "<!DOCTYPE html><html><body></body></html>";
        let doc = parse(html);

        assert!(doc.doctype.is_some());
        assert_eq!(doc.doctype.as_ref().map(|d| d.as_ref()), Some("html"));
    }

    #[test]
    fn test_tendril_buffer_sharing() {
        // Verify that parsed strings actually share buffers via Tendril refcounting
        let html = "<html><body><p>Hello World</p></body></html>";

        // Create the source tendril explicitly so we can check sharing
        let source_tendril = StrTendril::from(html);
        let sink = ArenaSink::new();
        let doc = parse_document(sink, Default::default()).one(source_tendril.clone());

        // Check that text nodes share the buffer with source
        let body = doc.body().expect("should have body");
        let p = body
            .children(&doc.arena)
            .next()
            .expect("body should have p");
        let text_node = p.children(&doc.arena).next().expect("p should have text");

        if let NodeKind::Text(text_tendril) = &doc.get(text_node).kind {
            // Verify buffer sharing with is_shared_with()
            assert!(
                text_tendril.is_shared_with(&source_tendril),
                "Text tendril should share buffer with source tendril (zero-copy)"
            );

            // Also check is_shared() - should be true since there are multiple refs
            assert!(
                text_tendril.is_shared(),
                "Text tendril should be marked as shared"
            );
        } else {
            panic!("Expected text node");
        }

        // Check that doctype shares buffer
        if let Some(doctype) = &doc.doctype {
            // DOCTYPE is "html" which should be a subtendril of source
            assert!(
                doctype.is_shared_with(&source_tendril),
                "DOCTYPE should share buffer with source tendril (zero-copy)"
            );
        }
    }

    #[test]
    fn test_parse_nested_elements() {
        let html = "<html><body><div><span>Text</span></div></body></html>";
        let doc = parse(html);

        let body = doc.body().expect("should have body");
        let div = body
            .children(&doc.arena)
            .next()
            .expect("body should have div");

        let div_data = doc.get(div);
        if let NodeKind::Element(elem) = &div_data.kind {
            assert_eq!(elem.tag.as_ref(), "div");
        }

        let span = div
            .children(&doc.arena)
            .next()
            .expect("div should have span");
        let span_data = doc.get(span);
        if let NodeKind::Element(elem) = &span_data.kind {
            assert_eq!(elem.tag.as_ref(), "span");
        }
    }

    #[test]
    fn test_parse_comment() {
        let html = "<html><body><!-- This is a comment --></body></html>";
        let doc = parse(html);

        let body = doc.body().expect("should have body");
        let comment = body
            .children(&doc.arena)
            .next()
            .expect("body should have comment");

        let comment_data = doc.get(comment);
        if let NodeKind::Comment(text) = &comment_data.kind {
            assert_eq!(text.as_ref(), " This is a comment ");
        }
    }

    #[test]
    fn test_to_html() {
        let html = "<html><body><div>Hello</div></body></html>";
        let doc = parse(html);
        assert_eq!(
            doc.to_html(),
            "<html><head></head><body><div>Hello</div></body></html>"
        );
    }

    #[test]
    fn test_to_html_with_attributes() {
        let html = r#"<html><body><div class="container" id="main">Content</div></body></html>"#;
        let doc = parse(html);
        let output = doc.to_html();
        assert!(output.contains("<html>"));
        assert!(output.contains("<body>"));
        assert!(output.contains("<div"));
        assert!(output.contains("class=\"container\""));
        assert!(output.contains("id=\"main\""));
        assert!(output.contains(">Content</div>"));
    }

    #[test]
    fn test_to_html_escaping() {
        let html = "<html><body><div>&lt;script&gt; &amp; \"quotes\"</div></body></html>";
        let doc = parse(html);
        assert_eq!(
            doc.to_html(),
            "<html><head></head><body><div>&lt;script&gt; &amp; \"quotes\"</div></body></html>"
        );
    }

    #[test]
    fn test_to_html_void_elements() {
        let html = "<html><body><br><img src=\"test.png\"></body></html>";
        let doc = parse(html);
        let output = doc.to_html();
        assert!(output.contains("<html>"));
        assert!(output.contains("<br>"));
        assert!(output.contains("<img"));
        assert!(output.contains("src=\"test.png\">"));
        assert!(!output.contains("</br>"));
        assert!(!output.contains("</img>"));
    }

    #[test]
    fn test_apply_patches_roundtrip() {
        // Test that we can diff two arena_dom documents and apply patches
        let old_html = "<html><body><div>Old content</div></body></html>";
        let new_html = "<html><body><div>New content</div></body></html>";

        let old_doc = parse(old_html);
        let new_doc = parse(new_html);

        // Generate patches
        let patches = crate::diff::diff(&old_doc, &new_doc).expect("diff should succeed");

        // Apply patches to a fresh copy of old
        let mut mut_old_doc = parse(old_html);
        mut_old_doc
            .apply_patches(patches)
            .expect("patches should apply");

        // Check result matches new
        assert_eq!(mut_old_doc.to_html(), new_doc.to_html());
    }

    #[test]
    fn test_apply_patches_insert_element() {
        let old_html = "<html><body><div>First</div></body></html>";
        let new_html = "<html><body><div>First</div><p>Second</p></body></html>";

        let old_doc = parse(old_html);
        let new_doc = parse(new_html);

        let patches = crate::diff::diff(&old_doc, &new_doc).expect("diff failed");

        let mut mut_old_doc = parse(old_html);
        mut_old_doc.apply_patches(patches).expect("apply failed");

        assert_eq!(mut_old_doc.to_html(), new_doc.to_html());
    }
}
