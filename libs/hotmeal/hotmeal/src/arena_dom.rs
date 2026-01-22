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
use html5ever::{local_name, namespace_url, ns};
use indexmap::IndexMap;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use tendril::{StrTendril, TendrilSink};

use crate::debug;
use crate::diff::{InsertContent, NodeRef, Patch};

/// Document = Arena (strings are StrTendrils with refcounted sharing)
#[derive(Debug, Clone)]
pub struct Document {
    /// THE tree - all nodes live here
    pub arena: Arena<NodeData>,

    /// Root node (usually `<html>` element)
    pub root: NodeId,

    /// DOCTYPE if present (usually "html")
    pub doctype: Option<StrTendril>,
}

impl Document {
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

    /// Serialize to HTML string (body content only, no doctype)
    pub fn to_html(&self) -> String {
        let mut output = String::new();
        // Find body element and serialize its children
        if let Some(body_id) = self.body() {
            for child_id in body_id.children(&self.arena) {
                self.serialize_node(&mut output, child_id);
            }
        }
        output
    }

    /// Navigate to a node by path starting from body.
    /// Returns the NodeId at the given path.
    fn navigate_path(&self, path: &[usize]) -> Result<NodeId, String> {
        let mut current = self.body().ok_or("no body element")?;

        for &idx in path {
            let mut children = current.children(&self.arena);
            current = children
                .nth(idx)
                .ok_or_else(|| format!("path index {idx} out of bounds"))?;
        }

        Ok(current)
    }

    /// Get parent of a node by path. Returns (parent_id, child_index).
    fn get_parent(&self, path: &[usize]) -> Result<(NodeId, usize), String> {
        if path.is_empty() {
            return Err("cannot get parent of empty path".to_string());
        }

        let parent_path = &path[..path.len() - 1];
        let child_idx = path[path.len() - 1];
        let parent_id = if parent_path.is_empty() {
            self.body().ok_or("no body element")?
        } else {
            self.navigate_path(parent_path)?
        };

        Ok((parent_id, child_idx))
    }

    /// Apply patches to this document (modifying it in place).
    pub fn apply_patches(&mut self, patches: &[Patch]) -> Result<(), String> {
        // Slots hold NodeIds that were displaced during edits
        let mut slots: HashMap<u32, NodeId> = HashMap::new();

        for patch in patches {
            self.apply_patch(patch, &mut slots)?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn apply_patch(
        &mut self,
        patch: &Patch,
        slots: &mut HashMap<u32, NodeId>,
    ) -> Result<(), String> {
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
                    tag: StrTendril::from(tag.as_str()),
                    attrs: attrs
                        .iter()
                        .map(|(k, v)| (k.clone(), StrTendril::from(v.as_str())))
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

                self.insert_at(at, new_node, *detach_to_slot, slots)?;
            }
            Patch::InsertText {
                at,
                text,
                detach_to_slot,
            } => {
                let new_node = self.arena.new_node(NodeData {
                    kind: NodeKind::Text(StrTendril::from(text.as_str())),
                    ns: Namespace::Html,
                });
                self.insert_at(at, new_node, *detach_to_slot, slots)?;
            }
            Patch::InsertComment {
                at,
                text,
                detach_to_slot,
            } => {
                let new_node = self.arena.new_node(NodeData {
                    kind: NodeKind::Comment(StrTendril::from(text.as_str())),
                    ns: Namespace::Html,
                });
                self.insert_at(at, new_node, *detach_to_slot, slots)?;
            }
            Patch::Remove { node } => match node {
                NodeRef::Path(path) => {
                    // Replace with empty text node to preserve sibling positions
                    let node_id = self.navigate_path(&path.0)?;
                    let empty_text = self.arena.new_node(NodeData {
                        kind: NodeKind::Text(StrTendril::new()),
                        ns: Namespace::Html,
                    });
                    node_id.insert_before(empty_text, &mut self.arena);
                    node_id.detach(&mut self.arena);
                }
                NodeRef::Slot(_slot, _rel_path) => {
                    // Slots are already detached - just remove from map
                    slots.remove(_slot);
                }
            },
            Patch::SetText { path, text } => {
                let node_id = self.navigate_path(&path.0)?;
                let node_data = self.arena[node_id].get_mut();
                if let NodeKind::Text(t) = &mut node_data.kind {
                    *t = StrTendril::from(text.as_str());
                } else {
                    return Err("SetText: node is not a text node".to_string());
                }
            }
            Patch::SetAttribute { path, name, value } => {
                let node_id = self.navigate_path(&path.0)?;
                let node_data = self.arena[node_id].get_mut();
                if let NodeKind::Element(elem) = &mut node_data.kind {
                    elem.attrs
                        .insert(name.clone(), StrTendril::from(value.as_str()));
                } else {
                    return Err("SetAttribute: node is not an element".to_string());
                }
            }
            Patch::RemoveAttribute { path, name } => {
                let node_id = self.navigate_path(&path.0)?;
                let node_data = self.arena[node_id].get_mut();
                if let NodeKind::Element(elem) = &mut node_data.kind {
                    elem.attrs.shift_remove(name);
                } else {
                    return Err("RemoveAttribute: node is not an element".to_string());
                }
            }
            Patch::Move {
                from,
                to,
                detach_to_slot,
            } => {
                let node_to_move = self.resolve_node_ref(from, slots)?;

                // Replace source position with empty text (no shifting!)
                // Exception: Slot(_, None) means moving the entire slot root
                let needs_replacement = match from {
                    NodeRef::Path(_) => true,
                    NodeRef::Slot(_, rel_path) => rel_path.is_some(),
                };

                if needs_replacement {
                    let empty_text = self.arena.new_node(NodeData {
                        kind: NodeKind::Text(StrTendril::new()),
                        ns: Namespace::Html,
                    });
                    node_to_move.insert_before(empty_text, &mut self.arena);
                    node_to_move.detach(&mut self.arena);
                }

                self.insert_at(to, node_to_move, *detach_to_slot, slots)?;
            }
            Patch::UpdateProps { path, changes } => {
                let node_id = self.navigate_path(&path.0)?;
                let node_data = self.arena[node_id].get_mut();

                // Handle text node updates
                if let Some(text_change) = changes.iter().find(|c| c.name == "_text") {
                    if let NodeKind::Text(t) = &mut node_data.kind {
                        if let Some(new_text) = &text_change.value {
                            *t = StrTendril::from(new_text.as_str());
                        }
                    } else if let NodeKind::Comment(c) = &mut node_data.kind
                        && let Some(new_text) = &text_change.value
                    {
                        *c = StrTendril::from(new_text.as_str());
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
                        if change.name != "_text" {
                            debug!(
                                "UpdateProps: processing attr {} value={:?}",
                                change.name, change.value
                            );
                            let value = if let Some(new_value) = &change.value {
                                // Different value - use the new one
                                StrTendril::from(new_value.as_str())
                            } else {
                                // Same value - copy from old attrs
                                old_attrs.get(&change.name).cloned().unwrap_or_default()
                            };
                            elem.attrs.insert(change.name.clone(), value);
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

    fn resolve_node_ref(
        &self,
        node_ref: &NodeRef,
        slots: &HashMap<u32, NodeId>,
    ) -> Result<NodeId, String> {
        match node_ref {
            NodeRef::Path(path) => self.navigate_path(&path.0),
            NodeRef::Slot(slot, rel_path) => {
                let slot_node = slots
                    .get(slot)
                    .ok_or_else(|| format!("slot {slot} not found"))?;
                if let Some(path) = rel_path {
                    let mut current = *slot_node;
                    for &idx in &path.0 {
                        let mut children = current.children(&self.arena);
                        current = children
                            .nth(idx)
                            .ok_or_else(|| format!("relative path index {idx} out of bounds"))?;
                    }
                    Ok(current)
                } else {
                    Ok(*slot_node)
                }
            }
        }
    }

    fn insert_at(
        &mut self,
        at: &NodeRef,
        node_to_insert: NodeId,
        detach_to_slot: Option<u32>,
        slots: &mut HashMap<u32, NodeId>,
    ) -> Result<(), String> {
        match at {
            NodeRef::Path(path) => {
                let (parent_id, position) = self.get_parent(&path.0)?;

                if let Some(slot) = detach_to_slot {
                    let children: Vec<_> = parent_id.children(&self.arena).collect();
                    if position < children.len() {
                        let displaced = children[position];
                        displaced.detach(&mut self.arena);
                        slots.insert(slot, displaced);
                    }
                }

                self.insert_at_position(parent_id, position, node_to_insert)?;
            }
            NodeRef::Slot(slot, rel_path) => {
                let slot_node = *slots
                    .get(slot)
                    .ok_or_else(|| format!("slot {slot} not found"))?;

                if let Some(path) = rel_path {
                    let parent_path = &path.0[..path.0.len() - 1];
                    let position = path.0[path.0.len() - 1];

                    debug!(
                        "insert_at Slot: slot={}, slot_node={:?}, rel_path={:?}, position={}",
                        slot, slot_node, path.0, position
                    );

                    let mut parent_id = slot_node;
                    for &idx in parent_path {
                        let mut children = parent_id.children(&self.arena);
                        parent_id = children
                            .nth(idx)
                            .ok_or_else(|| format!("relative path index {idx} out of bounds"))?;
                    }

                    debug!(
                        "insert_at Slot: after navigation, parent_id={:?}",
                        parent_id
                    );

                    if let Some(new_slot) = detach_to_slot {
                        let children: Vec<_> = parent_id.children(&self.arena).collect();
                        debug!(
                            "insert_at Slot: detaching at position {}, children.len()={}",
                            position,
                            children.len()
                        );
                        if position < children.len() {
                            let displaced = children[position];
                            displaced.detach(&mut self.arena);
                            slots.insert(new_slot, displaced);
                        }
                    }

                    self.insert_at_position(parent_id, position, node_to_insert)?;
                } else {
                    return Err("cannot insert at slot without relative path".to_string());
                }
            }
        }

        Ok(())
    }

    fn insert_at_position(
        &mut self,
        parent_id: NodeId,
        position: usize,
        node_to_insert: NodeId,
    ) -> Result<(), String> {
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
                kind: NodeKind::Text(StrTendril::new()),
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

    fn create_insert_content(&mut self, content: &InsertContent) -> Result<NodeId, String> {
        match content {
            InsertContent::Element {
                tag,
                attrs,
                children,
            } => {
                let elem_data = ElementData {
                    tag: StrTendril::from(tag.as_str()),
                    attrs: attrs
                        .iter()
                        .map(|(k, v)| (k.clone(), StrTendril::from(v.as_str())))
                        .collect(),
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
                    kind: NodeKind::Text(StrTendril::from(text.as_str())),
                    ns: Namespace::Html,
                });
                Ok(node)
            }
            InsertContent::Comment(text) => {
                let node = self.arena.new_node(NodeData {
                    kind: NodeKind::Comment(StrTendril::from(text.as_str())),
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
            out.push_str(name);
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
    /// Text content (StrTendril is refcounted - cheap to clone)
    Text(StrTendril),
    /// HTML comment
    Comment(StrTendril),
}

/// Element data (tag + attributes)
#[derive(Debug, Clone)]
pub struct ElementData {
    /// Tag name (StrTendril shares buffer with source via refcounting)
    pub tag: StrTendril,

    /// Attributes - keys are String (to avoid clippy mutable_key_type), values are StrTendril
    /// IndexMap preserves insertion order for consistent serialization
    pub attrs: IndexMap<String, StrTendril>,
}

/// XML namespace
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    let tendril = StrTendril::from(html);
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
    doctype: RefCell<Option<StrTendril>>,
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
            let tag = elem.tag.as_ref();
            let local_name = LocalName::from(tag);
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
        // Convert tag name to StrTendril
        let tag = StrTendril::from(name.local.as_ref());
        let ns = Namespace::from_url(name.ns.as_ref());

        // Convert attributes - keys are String, values are StrTendril
        // IndexMap preserves insertion order from HTML
        let attr_map: IndexMap<_, _> = attrs
            .into_iter()
            .map(|attr| {
                let key = attr.name.local.to_string();
                let value = attr.value.clone(); // StrTendril clone is cheap (refcounted)
                (key, value)
            })
            .collect();

        // Create node in arena
        self.arena.borrow_mut().new_node(NodeData {
            kind: NodeKind::Element(ElementData {
                tag,
                attrs: attr_map,
            }),
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
            kind: NodeKind::Comment(StrTendril::new()),
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

                // Create new text node (StrTendril clone is cheap - refcounted)
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
        _prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        // Just append to element
        self.append(element, child);
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
                let key = attr.name.local.to_string();
                elem.attrs.entry(key).or_insert_with(|| {
                    attr.value.clone() // StrTendril clone is cheap (refcounted)
                });
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
            // StrTendril shares buffers via refcounting (cheap clone)
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

            // Check attributes (keys are Strings)
            assert_eq!(
                elem.attrs.get("class").map(|v| v.as_ref()),
                Some("container")
            );
            assert_eq!(elem.attrs.get("id").map(|v| v.as_ref()), Some("main"));
            // StrTendril values use refcounted buffer sharing (cheap clone)
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
        // Use div instead of p to avoid auto-closing weirdness
        let html = "<html><body><div>Hello</div></body></html>";
        let doc = parse(html);
        assert_eq!(doc.to_html(), "<div>Hello</div>");
    }

    #[test]
    fn test_to_html_with_attributes() {
        let html = r#"<html><body><div class="container" id="main">Content</div></body></html>"#;
        let doc = parse(html);
        let output = doc.to_html();
        // Note: attribute order is nondeterministic due to HashMap
        assert!(output.contains("<div"));
        assert!(output.contains("class=\"container\""));
        assert!(output.contains("id=\"main\""));
        assert!(output.contains(">Content</div>"));
    }

    #[test]
    fn test_to_html_escaping() {
        let html = "<html><body><div>&lt;script&gt; &amp; \"quotes\"</div></body></html>";
        let doc = parse(html);
        assert_eq!(doc.to_html(), "<div>&lt;script&gt; &amp; \"quotes\"</div>");
    }

    #[test]
    fn test_to_html_void_elements() {
        let html = "<html><body><br><img src=\"test.png\"></body></html>";
        let doc = parse(html);
        let output = doc.to_html();
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
        let patches =
            crate::diff::diff_arena_documents(&old_doc, &new_doc).expect("diff should succeed");

        // Apply patches to a fresh copy of old
        let mut mut_old_doc = parse(old_html);
        mut_old_doc
            .apply_patches(&patches)
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

        let patches = crate::diff::diff_arena_documents(&old_doc, &new_doc).expect("diff failed");

        let mut mut_old_doc = parse(old_html);
        mut_old_doc.apply_patches(&patches).expect("apply failed");

        assert_eq!(mut_old_doc.to_html(), new_doc.to_html());
    }
}
