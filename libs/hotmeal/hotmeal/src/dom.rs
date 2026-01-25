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
use tendril::StrTendril;

use crate::diff::{DiffError, InsertContent, NodeRef, Patch, PropKey};
use crate::{Stem, debug};

#[cfg(feature = "tracing")]
macro_rules! trace {
    ($($arg:tt)*) => {
        tracing::trace!($($arg)*)
    };
}

#[cfg(not(feature = "tracing"))]
macro_rules! trace {
    ($($arg:tt)*) => {};
}

/// Arena-based DOM document.
#[derive(Debug, Clone)]
pub struct Document<'a> {
    /// The tree - all nodes live here
    pub arena: Arena<NodeData<'a>>,

    /// Root node (usually `<html>` element)
    pub root: NodeId,

    /// Errors encountered while parsing
    pub errors: Vec<Cow<'static, str>>,

    /// DOCTYPE if present (usually "html")
    pub doctype: Option<Stem<'a>>,
}

impl<'a> Document<'a> {
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
            errors: Default::default(),
            doctype: None,
        }
    }

    /// Get immutable reference to node data
    pub fn get(&self, id: NodeId) -> &NodeData<'a> {
        self.arena[id].get()
    }

    /// Get a human-readable label for a node (for debugging)
    #[allow(dead_code)]
    fn node_label(&self, id: NodeId) -> String {
        let data = self.get(id);
        match &data.kind {
            NodeKind::Element(elem) => format!("<{}>", elem.tag),
            NodeKind::Text(t) => {
                let preview: String = t.chars().take(10).collect();
                format!("text({:?})", preview)
            }
            NodeKind::Comment(t) => {
                let preview: String = t.chars().take(10).collect();
                format!("comment({:?})", preview)
            }
            NodeKind::Document => "#document".to_string(),
        }
    }

    /// Get mutable reference to node data
    pub fn get_mut(&mut self, id: NodeId) -> &mut NodeData<'a> {
        self.arena[id].get_mut()
    }

    /// Iterate children of a node
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        id.children(&self.arena)
    }

    /// Pretty-print a subtree for debugging.
    pub fn dump_subtree(&self, node_id: NodeId) -> String {
        let mut out = String::new();
        self.dump_node(node_id, 0, &mut out);
        out
    }

    /// Pretty-print the body subtree, if present.
    pub fn dump_body(&self) -> Option<String> {
        self.body().map(|body_id| self.dump_subtree(body_id))
    }

    fn dump_node(&self, node_id: NodeId, indent: usize, out: &mut String) {
        let prefix = "  ".repeat(indent);
        let node = &self.arena[node_id].get().kind;

        match node {
            NodeKind::Element(elem) => {
                let tag = elem.tag.to_string().to_ascii_lowercase();
                let mut attrs: Vec<(String, String)> = elem
                    .attrs
                    .iter()
                    .map(|(qname, value)| (qname.local.to_string(), value.as_ref().to_string()))
                    .collect();
                attrs.sort_by(|a, b| a.0.cmp(&b.0));

                out.push_str(&format!("{prefix}<{tag}"));
                for (name, value) in attrs {
                    out.push_str(&format!(" {}={:?}", name, value));
                }
                out.push_str(">\n");

                for child in self.children(node_id) {
                    self.dump_node(child, indent + 1, out);
                }

                out.push_str(&format!("{prefix}</{tag}>\n"));
            }
            NodeKind::Text(text) => {
                out.push_str(&format!("{prefix}TEXT: {:?}\n", text.as_ref()));
            }
            NodeKind::Comment(text) => {
                out.push_str(&format!("{prefix}COMMENT: {:?}\n", text.as_ref()));
            }
            NodeKind::Document => {
                out.push_str(&format!("{prefix}#document\n"));
                for child in self.children(node_id) {
                    self.dump_node(child, indent + 1, out);
                }
            }
        }
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

    /// Ensure the document has a body element, creating one if needed.
    /// Returns the body NodeId.
    fn ensure_body(&mut self) -> NodeId {
        // If body exists, return it
        if let Some(body_id) = self.body() {
            return body_id;
        }

        // Create body element and attach to root (html element)
        let body_id = self.arena.new_node(NodeData {
            kind: NodeKind::Element(ElementData {
                tag: LocalName::from("body"),
                attrs: Vec::new(),
            }),
            ns: Namespace::Html,
        });
        self.root.append(body_id, &mut self.arena);
        body_id
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
    pub fn create_text(&mut self, text: impl Into<Stem<'a>>) -> NodeId {
        self.arena.new_node(NodeData {
            kind: NodeKind::Text(text.into()),
            ns: Namespace::Html,
        })
    }

    /// Create a comment node (not yet attached to the tree)
    pub fn create_comment(&mut self, text: impl Into<Stem<'a>>) -> NodeId {
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
    pub fn set_attr(&mut self, element: NodeId, name: QualName, value: impl Into<Stem<'a>>) {
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
    pub fn set_text(&mut self, node: NodeId, text: impl Into<Stem<'a>>) {
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

impl Default for Document<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Document<'a> {
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

    /// Serialize just the body's inner content.
    /// Returns empty string if there's no body.
    pub fn to_body_html(&self) -> String {
        let Some(body_id) = self.body() else {
            return String::new();
        };
        let mut output = String::new();
        for child_id in body_id.children(&self.arena) {
            self.serialize_node(&mut output, child_id);
        }
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
    pub fn apply_patches(&mut self, patches: Vec<Patch<'a>>) -> Result<(), DiffError> {
        // Empty patches is a no-op (even if document has no body)
        if patches.is_empty() {
            return Ok(());
        }

        // Slots hold NodeIds - slot 0 is always the body (main tree)
        let mut slots: HashMap<u32, NodeId> = HashMap::new();
        let body_id = self.body().unwrap_or_else(|| self.ensure_body());
        slots.insert(0, body_id);

        for patch in patches {
            self.apply_patch(patch, &mut slots)?;
        }

        Ok(())
    }

    /// Initialize slot map for patch application.
    pub fn init_patch_slots(&mut self) -> HashMap<u32, NodeId> {
        let mut slots: HashMap<u32, NodeId> = HashMap::new();
        let body_id = self.body().unwrap_or_else(|| self.ensure_body());
        slots.insert(0, body_id);
        slots
    }

    /// Apply a single patch using a caller-provided slot map.
    pub fn apply_patch_with_slots(
        &mut self,
        patch: Patch<'a>,
        slots: &mut HashMap<u32, NodeId>,
    ) -> Result<(), DiffError> {
        self.apply_patch(patch, slots)
    }

    #[allow(clippy::too_many_lines)]
    fn apply_patch(
        &mut self,
        patch: Patch<'a>,
        slots: &mut HashMap<u32, NodeId>,
    ) -> Result<(), DiffError> {
        debug!("Applying patch: {:?}", patch);
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
            "insert_at: path={:?} parent={} pos={}",
            path,
            self.node_label(parent_id),
            position
        );

        if let Some(slot) = detach_to_slot {
            let children: Vec<_> = parent_id.children(&self.arena).collect();
            debug!(
                "insert_at: detaching child {} at pos {} to slot{}",
                children
                    .get(position)
                    .map(|c| self.node_label(*c))
                    .unwrap_or_else(|| "?".to_string()),
                position,
                slot
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
            "insert_at_position: {} under {} at pos {} (has {} children)",
            self.node_label(node_to_insert),
            self.node_label(parent_id),
            position,
            children.len()
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

    fn create_insert_content(&mut self, content: InsertContent<'a>) -> Result<NodeId, DiffError> {
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

        // Children - raw text elements (script, style) should not have their content escaped
        let is_raw_text = is_raw_text_element(tag);
        for child_id in node_id.children(&self.arena) {
            if is_raw_text {
                self.serialize_node_raw(out, child_id);
            } else {
                self.serialize_node(out, child_id);
            }
        }

        // Closing tag
        out.push_str("</");
        out.push_str(tag);
        out.push('>');
    }

    /// Serialize a node without escaping text content (for raw text elements like script/style)
    fn serialize_node_raw(&self, out: &mut String, node_id: NodeId) {
        let node = self.get(node_id);
        match &node.kind {
            NodeKind::Document => {}
            NodeKind::Element(elem) => {
                self.serialize_element(out, node_id, elem);
            }
            NodeKind::Text(text) => {
                // Raw text - no escaping
                out.push_str(text.as_ref());
            }
            NodeKind::Comment(text) => {
                out.push_str("<!--");
                out.push_str(text.as_ref());
                out.push_str("-->");
            }
        }
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

/// HTML5 raw text elements whose content should not be escaped
/// See: https://html.spec.whatwg.org/multipage/syntax.html#raw-text-elements
///
/// Note: `noscript` is included because we always parse with scripting enabled
/// (html5ever's default). When scripting is enabled, noscript is parsed as rawtext,
/// so it must also be serialized as rawtext for innerHTML to be idempotent.
fn is_raw_text_element(tag: &str) -> bool {
    matches!(tag, "script" | "style" | "noscript")
}

/// What goes in each arena slot
#[derive(Debug, Clone)]
pub struct NodeData<'a> {
    pub kind: NodeKind<'a>,
    pub ns: Namespace,
}

/// Node types
#[derive(Debug, Clone)]
pub enum NodeKind<'a> {
    /// Document root (invisible, parent of `<html>`)
    Document,
    /// Element with tag and attributes
    Element(ElementData<'a>),
    /// Text content
    Text(Stem<'a>),
    /// HTML comment
    Comment(Stem<'a>),
}

/// Element data (tag + attributes)
#[derive(Debug, Clone)]
pub struct ElementData<'a> {
    /// Tag name (LocalName is interned via string_cache)
    pub tag: LocalName,

    /// Attributes - Vec preserves insertion order for consistent serialization
    pub attrs: Vec<(QualName, Stem<'a>)>,
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

fn has_doctype_prefix(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let doctype = b"<!doctype";
    i + doctype.len() <= bytes.len()
        && bytes[i..i + doctype.len()]
            .iter()
            .zip(doctype)
            .all(|(b, d)| b.to_ascii_lowercase() == *d)
}

/// Parse HTML from a StrTendril with zero-copy borrowing.
///
/// The returned Document borrows from the tendril's buffer, so substrings
/// that don't need transformation will be borrowed rather than copied.
///
/// ```
/// use hotmeal::{parse, StrTendril};
///
/// let input = StrTendril::from("<html><body>Hello</body></html>");
/// let doc = parse(&input);
/// // doc borrows from input - zero-copy for unchanged content
/// ```
pub fn parse(tendril: &StrTendril) -> Document<'_> {
    use tendril::TendrilSink;

    let input_ref: &str = tendril.as_ref();
    let sink = ArenaSink::new(input_ref);

    // Prepend DOCTYPE if not present to ensure no-quirks mode parsing.
    // This matches browser behavior for innerHTML which always uses no-quirks.
    let has_doctype = has_doctype_prefix(input_ref);

    // When the input doesn't have a doctype, we need to wrap it carefully.
    // Simply prepending "<!DOCTYPE html>" would cause leading whitespace in
    // content like " *" to become inter-element whitespace before <html> and
    // get discarded. Instead, wrap in <html><body>...</body></html> so the
    // content is clearly in the body context.
    let input = if has_doctype {
        tendril.clone()
    } else if has_html_structure(input_ref) {
        // Input has <html> or <body> tags, just prepend DOCTYPE
        let mut with_doctype = StrTendril::from("<!DOCTYPE html>");
        with_doctype.push_tendril(tendril);
        with_doctype
    } else {
        // Input is raw body content - wrap it to preserve leading whitespace
        let mut wrapped = StrTendril::from("<!DOCTYPE html><html><body>");
        wrapped.push_tendril(tendril);
        wrapped.push_slice("</body></html>");
        wrapped
    };

    let mut doc = parse_document(sink, Default::default()).one(input);
    if !has_doctype {
        // Don't store the artificially added DOCTYPE - preserve original input behavior
        doc.doctype = None;
    }
    doc
}

/// Parse HTML as a body fragment (like `body.innerHTML = html`).
///
/// This uses the HTML5 fragment parsing algorithm with `<body>` as the context element.
/// This matches browser innerHTML parsing behavior with scripting enabled.
///
/// Use this when comparing against browser innerHTML parsing for parity testing.
pub fn parse_body_fragment(tendril: &StrTendril) -> Document<'_> {
    use html5ever::parse_fragment;
    use tendril::TendrilSink;

    let input_ref: &str = tendril.as_ref();
    let sink = ArenaSink::new(input_ref);

    // Parse as fragment with <body> as context, scripting enabled
    let context_name = QualName::new(None, ns!(html), local_name!("body"));
    let context_attrs = vec![];
    let scripting_enabled = true;

    parse_fragment(
        sink,
        Default::default(),
        context_name,
        context_attrs,
        scripting_enabled,
    )
    .one(tendril.clone())
}

/// Check if input has HTML structure tags (<html> or <body>).
fn has_html_structure(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    lower.contains("<html") || lower.contains("<body")
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
struct ArenaSink<'a> {
    /// Original input - used to borrow strings when possible
    input: &'a str,

    /// Our arena - wrapped in RefCell for interior mutability
    arena: RefCell<Arena<NodeData<'a>>>,

    /// Document node (parent of `<html>`)
    document: NodeId,

    /// Parse errors
    errors: RefCell<Vec<Cow<'static, str>>>,

    /// DOCTYPE encountered during parse
    doctype: RefCell<Option<Stem<'a>>>,
}

/// Convert a StrTendril to Stem, borrowing from input if possible.
/// This is a free function so it can be used when self is partially borrowed.
fn tendril_to_stem_with_input<'a>(input: &'a str, t: StrTendril) -> Stem<'a> {
    let t_bytes = t.as_bytes();
    let input_bytes = input.as_bytes();

    let t_start = t_bytes.as_ptr() as usize;
    let t_end = t_start + t_bytes.len();
    let input_start = input_bytes.as_ptr() as usize;
    let input_end = input_start + input_bytes.len();

    if t_start >= input_start && t_end <= input_end {
        let offset = t_start - input_start;
        Stem::Borrowed(&input[offset..offset + t.len()])
    } else {
        Stem::from(t)
    }
}

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

fn dump_arena_subtree<'a>(
    arena: &Arena<NodeData<'a>>,
    node_id: NodeId,
    highlights: &[(NodeId, &'static str, &'static str)],
) -> String {
    fn highlight_for(
        node_id: NodeId,
        highlights: &[(NodeId, &'static str, &'static str)],
    ) -> Option<(&'static str, &'static str)> {
        highlights
            .iter()
            .find(|(id, _, _)| *id == node_id)
            .map(|(_, color, label)| (*color, *label))
    }

    fn dump_node<'b>(
        arena: &Arena<NodeData<'b>>,
        node_id: NodeId,
        indent: usize,
        out: &mut String,
        highlights: &[(NodeId, &'static str, &'static str)],
    ) {
        let indent_prefix = "  ".repeat(indent);
        let node_label = node_id_short(node_id);
        let prefix = format!("{indent_prefix}[{node_label}] ");
        let highlight = highlight_for(node_id, highlights);
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
        let node = &arena[node_id].get().kind;

        match node {
            NodeKind::Element(elem) => {
                let tag = elem.tag.to_string().to_ascii_lowercase();
                let tag_display = if hl_start.is_empty() {
                    tag
                } else {
                    format!("{hl_start}{tag}{hl_end}")
                };
                let mut attrs: Vec<(String, String)> = elem
                    .attrs
                    .iter()
                    .map(|(qname, value)| (qname.local.to_string(), value.as_ref().to_string()))
                    .collect();
                attrs.sort_by(|a, b| a.0.cmp(&b.0));

                out.push_str(&format!("{prefix}<{tag_display}"));
                for (name, value) in attrs {
                    out.push_str(&format!(" {}={:?}", name, value));
                }
                out.push_str(&format!(">{badge}\n"));

                for child in node_id.children(arena) {
                    dump_node(arena, child, indent + 1, out, highlights);
                }

                out.push_str(&format!("{prefix}</{tag_display}>\n"));
            }
            NodeKind::Text(text) => {
                out.push_str(&format!("{prefix}TEXT: {:?}{badge}\n", text.as_ref()));
            }
            NodeKind::Comment(text) => {
                out.push_str(&format!("{prefix}COMMENT: {:?}{badge}\n", text.as_ref()));
            }
            NodeKind::Document => {
                out.push_str(&format!("{prefix}#document{badge}\n"));
                for child in node_id.children(arena) {
                    dump_node(arena, child, indent + 1, out, highlights);
                }
            }
        }
    }

    let mut out = String::new();
    dump_node(arena, node_id, 0, &mut out, highlights);
    out
}

#[allow(dead_code)]
struct LazyTreeDump<'a> {
    arena: &'a Arena<NodeData<'a>>,
    node_id: NodeId,
    highlights: &'a [(NodeId, &'static str, &'static str)],
}

impl<'a> LazyTreeDump<'a> {
    #[allow(dead_code)]
    fn new(
        arena: &'a Arena<NodeData<'a>>,
        node_id: NodeId,
        highlights: &'a [(NodeId, &'static str, &'static str)],
    ) -> Self {
        Self {
            arena,
            node_id,
            highlights,
        }
    }
}

impl<'a> std::fmt::Display for LazyTreeDump<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tree = dump_arena_subtree(self.arena, self.node_id, self.highlights);
        write!(f, "\n{tree}")
    }
}

impl<'a> ArenaSink<'a> {
    fn new(input: &'a str) -> Self {
        let mut arena = Arena::new();

        let document = arena.new_node(NodeData {
            kind: NodeKind::Document,
            ns: Namespace::Html,
        });

        ArenaSink {
            input,
            arena: RefCell::new(arena),
            document,
            doctype: RefCell::new(None),
            errors: Default::default(),
        }
    }

    /// Convert a StrTendril to Stem, borrowing from input if possible
    fn tendril_to_stem(&self, t: StrTendril) -> Stem<'a> {
        tendril_to_stem_with_input(self.input, t)
    }
}

impl<'a> TreeSink for ArenaSink<'a> {
    type Handle = NodeId;
    type Output = Document<'a>;
    type ElemName<'b>
        = OwnedElemName
    where
        Self: 'b;

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
            errors: self.errors.into_inner(),
        }
    }

    fn parse_error(&self, msg: Cow<'static, str>) {
        self.errors.borrow_mut().push(msg);
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

    fn elem_name<'b>(&'b self, target: &'b Self::Handle) -> OwnedElemName {
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
            .map(|attr| (attr.name, self.tendril_to_stem(attr.value)))
            .collect();

        // Create node in arena
        self.arena.borrow_mut().new_node(NodeData {
            kind: NodeKind::Element(ElementData { tag, attrs }),
            ns,
        })
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.arena.borrow_mut().new_node(NodeData {
            kind: NodeKind::Comment(self.tendril_to_stem(text)),
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

    #[cfg_attr(not(feature = "tracing"), allow(unused_variables))]
    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        let mut arena = self.arena.borrow_mut();
        match child {
            NodeOrText::AppendNode(node) => {
                trace!(
                    parent_id = %node_id_short(*parent),
                    node_id = %node_id_short(node),
                    "append: node"
                );
                let highlights = [
                    (*parent, "\x1b[32m", "PARENT"),
                    (node, "\x1b[36m", "INSERTED"),
                ];
                if let Some(grandparent) = arena[*parent].parent() {
                    let gp_highlights = [
                        (grandparent, "\x1b[35m", "GRANDPARENT"),
                        (*parent, "\x1b[32m", "PARENT"),
                        (node, "\x1b[36m", "INSERTED"),
                    ];
                    trace!(
                        parent_id = %node_id_short(*parent),
                        node_id = %node_id_short(node),
                        grandparent_id = %node_id_short(grandparent),
                        tree = %LazyTreeDump::new(&arena, grandparent, &gp_highlights),
                        "append: before insert (grandparent)"
                    );
                } else {
                    trace!(
                        parent_id = %node_id_short(*parent),
                        node_id = %node_id_short(node),
                        tree = %LazyTreeDump::new(&arena, *parent, &highlights),
                        "append: before insert"
                    );
                }
                parent.append(node, &mut *arena);
                if let Some(grandparent) = arena[*parent].parent() {
                    let gp_highlights = [
                        (grandparent, "\x1b[35m", "GRANDPARENT"),
                        (*parent, "\x1b[32m", "PARENT"),
                        (node, "\x1b[36m", "INSERTED"),
                    ];
                    trace!(
                        parent_id = %node_id_short(*parent),
                        node_id = %node_id_short(node),
                        grandparent_id = %node_id_short(grandparent),
                        tree = %LazyTreeDump::new(&arena, grandparent, &gp_highlights),
                        "append: after insert (grandparent)"
                    );
                } else {
                    trace!(
                        parent_id = %node_id_short(*parent),
                        node_id = %node_id_short(node),
                        tree = %LazyTreeDump::new(&arena, *parent, &highlights),
                        "append: after insert"
                    );
                }
            }
            NodeOrText::AppendText(text) => {
                let text_len = text.len();
                let text_preview = text.as_ref().to_string();
                trace!(
                    parent_id = %node_id_short(*parent),
                    text_len,
                    text = text_preview.as_str(),
                    "append: text"
                );
                // Try to merge with previous text node (html5ever behavior)
                let last_child_id = parent.children(&arena).next_back();

                if let Some(last_child) = last_child_id
                    && let NodeKind::Text(existing) = &mut arena[last_child].get_mut().kind
                {
                    let existing_preview = existing.as_ref();
                    trace!(
                        parent_id = %node_id_short(*parent),
                        last_child_id = %node_id_short(last_child),
                        text_len,
                        text = text_preview,
                        existing = existing_preview,
                        "append: merged text"
                    );
                    existing.push_tendril(&text);
                    return;
                }

                // Can't use self.tendril_to_stem here because we have arena borrowed
                // Need to do the check manually
                let stem = tendril_to_stem_with_input(self.input, text);
                let text_node = arena.new_node(NodeData {
                    kind: NodeKind::Text(stem),
                    ns: Namespace::Html,
                });
                trace!(
                    parent_id = %node_id_short(*parent),
                    text_node_id = %node_id_short(text_node),
                    text_len,
                    text = text_preview.as_str(),
                    "append: new text node"
                );
                parent.append(text_node, &mut arena);
            }
        }
    }

    #[cfg_attr(not(feature = "tracing"), allow(unused_variables))]
    fn append_before_sibling(&self, sibling: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        let mut arena = self.arena.borrow_mut();

        match new_node {
            NodeOrText::AppendNode(node) => {
                let parent = arena[*sibling].parent();
                trace!(
                    sibling_id = %node_id_short(*sibling),
                    node_id = %node_id_short(node),
                    "append_before_sibling: node"
                );
                if let Some(parent) = parent {
                    if let Some(grandparent) = arena[parent].parent() {
                        let gp_highlights = [
                            (grandparent, "\x1b[35m", "GRANDPARENT"),
                            (parent, "\x1b[32m", "PARENT"),
                            (*sibling, "\x1b[33m", "SIBLING"),
                        ];
                        trace!(
                            sibling_id = %node_id_short(*sibling),
                            parent_id = %node_id_short(parent),
                            grandparent_id = %node_id_short(grandparent),
                            tree = %LazyTreeDump::new(&arena, grandparent, &gp_highlights),
                            "append_before_sibling: before insert (grandparent)"
                        );
                    } else {
                        let highlights = [
                            (parent, "\x1b[32m", "PARENT"),
                            (*sibling, "\x1b[33m", "SIBLING"),
                        ];
                        trace!(
                            sibling_id = %node_id_short(*sibling),
                            parent_id = %node_id_short(parent),
                            tree = %LazyTreeDump::new(&arena, parent, &highlights),
                            "append_before_sibling: before insert"
                        );
                    }
                }
                sibling.insert_before(node, &mut *arena);
                if let Some(parent) = parent {
                    if let Some(grandparent) = arena[parent].parent() {
                        let gp_highlights = [
                            (grandparent, "\x1b[35m", "GRANDPARENT"),
                            (parent, "\x1b[32m", "PARENT"),
                            (*sibling, "\x1b[33m", "SIBLING"),
                            (node, "\x1b[36m", "INSERTED"),
                        ];
                        trace!(
                            sibling_id = %node_id_short(*sibling),
                            parent_id = %node_id_short(parent),
                            inserted_id = %node_id_short(node),
                            grandparent_id = %node_id_short(grandparent),
                            tree = %LazyTreeDump::new(&arena, grandparent, &gp_highlights),
                            "append_before_sibling: after insert (grandparent)"
                        );
                    } else {
                        let highlights = [
                            (parent, "\x1b[32m", "PARENT"),
                            (*sibling, "\x1b[33m", "SIBLING"),
                            (node, "\x1b[36m", "INSERTED"),
                        ];
                        trace!(
                            sibling_id = %node_id_short(*sibling),
                            parent_id = %node_id_short(parent),
                            inserted_id = %node_id_short(node),
                            tree = %LazyTreeDump::new(&arena, parent, &highlights),
                            "append_before_sibling: after insert"
                        );
                    }
                }
            }
            NodeOrText::AppendText(text) => {
                let text_len = text.len();
                let text_preview = text.as_ref().to_string();
                let parent = arena[*sibling].parent();
                trace!(
                    sibling_id = %node_id_short(*sibling),
                    text_len,
                    text = text_preview.as_str(),
                    "append_before_sibling: text"
                );
                if let Some(parent) = parent {
                    let highlights = [
                        (parent, "\x1b[32m", "PARENT"),
                        (*sibling, "\x1b[33m", "SIBLING"),
                    ];
                    let tree_dump = dump_arena_subtree(&arena, parent, &highlights);
                    trace!(
                        sibling_id = %node_id_short(*sibling),
                        parent_id = %node_id_short(parent),
                        tree = %tree_dump,
                        "append_before_sibling: before text insert"
                    );
                }
                // Try to merge with the previous sibling if it's a text node
                let prev_sibling = arena[*sibling].previous_sibling();
                if let Some(prev_sibling) = prev_sibling {
                    let prev_kind = &arena[prev_sibling].get().kind;
                    trace!(
                        sibling_id = %node_id_short(*sibling),
                        prev_sibling_id = %node_id_short(prev_sibling),
                        prev_kind = ?prev_kind,
                        "append_before_sibling: prev sibling kind"
                    );

                    if let NodeKind::Text(existing) = &mut arena[prev_sibling].get_mut().kind {
                        let existing_preview = existing.as_ref();
                        trace!(
                            sibling_id = %node_id_short(*sibling),
                            prev_sibling_id = %node_id_short(prev_sibling),
                            text_len,
                            text = text_preview.as_str(),
                            existing = existing_preview,
                            "append_before_sibling: merged text"
                        );
                        existing.push_tendril(&text);
                        if let Some(parent) = parent {
                            let highlights = [
                                (parent, "\x1b[32m", "PARENT"),
                                (prev_sibling, "\x1b[35m", "MERGED"),
                                (*sibling, "\x1b[33m", "SIBLING"),
                            ];
                            let tree_dump = dump_arena_subtree(&arena, parent, &highlights);
                            trace!(
                                sibling_id = %node_id_short(*sibling),
                                parent_id = %node_id_short(parent),
                                merged_into_id = %node_id_short(prev_sibling),
                                tree = %tree_dump,
                                "append_before_sibling: after merge"
                            );
                        }
                        return;
                    }
                }

                let stem = tendril_to_stem_with_input(self.input, text);
                let text_node = arena.new_node(NodeData {
                    kind: NodeKind::Text(stem),
                    ns: Namespace::Html,
                });
                trace!(
                    sibling_id = %node_id_short(*sibling),
                    text_node_id = %node_id_short(text_node),
                    text_len,
                    text = text_preview.as_str(),
                    "append_before_sibling: new text node"
                );
                sibling.insert_before(text_node, &mut *arena);
                if let Some(parent) = parent {
                    let highlights = [
                        (parent, "\x1b[32m", "PARENT"),
                        (*sibling, "\x1b[33m", "SIBLING"),
                        (text_node, "\x1b[36m", "INSERTED"),
                    ];
                    let tree_dump = dump_arena_subtree(&arena, parent, &highlights);
                    trace!(
                        sibling_id = %node_id_short(*sibling),
                        parent_id = %node_id_short(parent),
                        inserted_id = %node_id_short(text_node),
                        tree = %tree_dump,
                        "append_before_sibling: after text insert"
                    );
                }
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
        *self.doctype.borrow_mut() = Some(self.tendril_to_stem(name));
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        // For <template>, return the element itself
        // (proper template support would need a template contents fragment)
        *target
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        // Convert tendrils to stems before borrowing arena
        let converted_attrs: Vec<_> = attrs
            .into_iter()
            .map(|attr| (attr.name, self.tendril_to_stem(attr.value)))
            .collect();

        let mut arena = self.arena.borrow_mut();
        let node = &mut arena[*target].get_mut();
        if let NodeKind::Element(elem) = &mut node.kind {
            for (name, value) in converted_attrs {
                // Only add if not already present
                if !elem.attrs.iter().any(|(k, _)| k == &name) {
                    trace!(
                        target_id = %node_id_short(*target),
                        tag = %elem.tag.as_ref(),
                        attr_ns = %name.ns,
                        attr_name = %name.local.as_ref(),
                        attr_value = %value.as_ref(),
                        "add_attrs_if_missing: adding attribute"
                    );
                    elem.attrs.push((name, value));
                } else {
                    trace!(
                        target_id = %node_id_short(*target),
                        tag = %elem.tag.as_ref(),
                        attr_ns = %name.ns,
                        attr_name = %name.local.as_ref(),
                        "add_attrs_if_missing: attribute already present"
                    );
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
    use facet_testhelpers::test;

    /// Helper to create a StrTendril from a string
    fn t(s: &str) -> StrTendril {
        StrTendril::from(s)
    }

    #[test]
    fn test_parse_simple_html() {
        let html = t("<html><body><p>Hello</p></body></html>");
        let doc = parse(&html);

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
        let full_html = t(&format!("<html><body>{}</body></html>", html));
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
        let html = t("<!DOCTYPE html><html><body></body></html>");
        let doc = parse(&html);

        assert!(doc.doctype.is_some());
        assert_eq!(doc.doctype.as_ref().map(|d| d.as_ref()), Some("html"));
    }

    #[test]
    fn test_has_doctype_prefix() {
        assert!(has_doctype_prefix("<!DOCTYPE html>"));
        assert!(has_doctype_prefix("   <!DoCtYpE html>"));
        assert!(!has_doctype_prefix("<html><body></body></html>"));
        assert!(!has_doctype_prefix("<!-- <!doctype html> -->"));
    }

    #[test]
    fn test_zero_copy_parsing() {
        // Verify that parsed strings borrow from the original input when possible
        // Note: DOCTYPE is required for zero-copy since parse() prepends it otherwise
        let html = t("<!DOCTYPE html><html><body><p>Hello World</p></body></html>");
        let html_start = html.as_ref().as_ptr() as usize;
        let html_end = html_start + html.len();
        trace!("Input range: {:#x}..{:#x}", html_start, html_end);

        let doc = parse(&html);

        // Check that text nodes borrow from source
        let body = doc.body().expect("should have body");
        let p = body
            .children(&doc.arena)
            .next()
            .expect("body should have p");
        let text_node = p.children(&doc.arena).next().expect("p should have text");

        if let NodeKind::Text(stem) = &doc.get(text_node).kind {
            let stem_str = stem.as_str();
            let stem_start = stem_str.as_ptr() as usize;
            let stem_end = stem_start + stem_str.len();
            trace!("Stem range: {:#x}..{:#x}", stem_start, stem_end);
            trace!("Stem variant: {:?}", matches!(stem, Stem::Borrowed(_)));

            // The text content should be the Borrowed variant
            assert!(
                matches!(stem, Stem::Borrowed(_)),
                "Text should be borrowed from input (zero-copy), but got owned"
            );
            assert_eq!(stem.as_ref(), "Hello World");
        } else {
            panic!("Expected text node");
        }
    }

    #[test]
    fn test_parse_nested_elements() {
        let html = t("<html><body><div><span>Text</span></div></body></html>");
        let doc = parse(&html);

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
        let html = t("<html><body><!-- This is a comment --></body></html>");
        let doc = parse(&html);

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
        let html = t("<html><body><div>Hello</div></body></html>");
        let doc = parse(&html);
        assert_eq!(
            doc.to_html(),
            "<html><head></head><body><div>Hello</div></body></html>"
        );
    }

    #[test]
    fn test_to_html_with_attributes() {
        let html = t(r#"<html><body><div class="container" id="main">Content</div></body></html>"#);
        let doc = parse(&html);
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
        let html = t("<html><body><div>&lt;script&gt; &amp; \"quotes\"</div></body></html>");
        let doc = parse(&html);
        assert_eq!(
            doc.to_html(),
            "<html><head></head><body><div>&lt;script&gt; &amp; \"quotes\"</div></body></html>"
        );
    }

    #[test]
    fn test_to_html_void_elements() {
        let html = t("<html><body><br><img src=\"test.png\"></body></html>");
        let doc = parse(&html);
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
        let old_html = t("<html><body><div>Old content</div></body></html>");
        let new_html = t("<html><body><div>New content</div></body></html>");

        let old_doc = parse(&old_html);
        let new_doc = parse(&new_html);

        // Generate patches
        let patches = crate::diff::diff(&old_doc, &new_doc).expect("diff should succeed");

        // Apply patches to a fresh copy of old
        let mut mut_old_doc = parse(&old_html);
        mut_old_doc
            .apply_patches(patches)
            .expect("patches should apply");

        // Check result matches new
        assert_eq!(mut_old_doc.to_html(), new_doc.to_html());
    }

    #[test]
    fn test_apply_patches_insert_element() {
        let old_html = t("<html><body><div>First</div></body></html>");
        let new_html = t("<html><body><div>First</div><p>Second</p></body></html>");

        let old_doc = parse(&old_html);
        let new_doc = parse(&new_html);

        let patches = crate::diff::diff(&old_doc, &new_doc).expect("diff failed");

        let mut mut_old_doc = parse(&old_html);
        mut_old_doc.apply_patches(patches).expect("apply failed");

        assert_eq!(mut_old_doc.to_html(), new_doc.to_html());
    }

    #[test]
    fn test_script_content_not_escaped() {
        // Script content should NOT be HTML-escaped (it's a raw text element)
        // Build DOM directly to test serialization, not parsing
        let html = t("<html><head></head><body></body></html>");
        let mut doc = parse(&html);

        let head = doc.head().expect("should have head");
        let script = doc.create_element("script");
        let script_text = doc.create_text("const x = () => { return 1 < 2; };");
        doc.append_child(script, script_text);
        doc.append_child(head, script);

        let output = doc.to_html();

        // The arrow function => should NOT become =&gt;
        assert!(
            output.contains("() => {"),
            "Arrow function should not be escaped. Got: {}",
            output
        );
        // The < comparison should NOT become &lt;
        assert!(
            output.contains("1 < 2"),
            "Less-than in script should not be escaped. Got: {}",
            output
        );
    }

    #[test]
    fn test_style_content_not_escaped() {
        // Style content should NOT be HTML-escaped (it's a raw text element)
        // Build DOM directly to test serialization, not parsing
        let html = t("<html><head></head><body></body></html>");
        let mut doc = parse(&html);

        let head = doc.head().expect("should have head");
        let style = doc.create_element("style");
        let style_text = doc.create_text(".foo > .bar { color: red; }");
        doc.append_child(style, style_text);
        doc.append_child(head, style);

        let output = doc.to_html();

        // The > selector should NOT become &gt;
        assert!(
            output.contains(".foo > .bar"),
            "CSS selector should not be escaped. Got: {}",
            output
        );
    }

    #[test]
    fn test_normal_text_still_escaped() {
        // Regular text content SHOULD still be escaped
        // Build DOM directly to test serialization, not parsing
        let html = t("<html><head></head><body></body></html>");
        let mut doc = parse(&html);

        let body = doc.body().expect("should have body");
        let p = doc.create_element("p");
        let p_text = doc.create_text("1 < 2 & 3 > 1");
        doc.append_child(p, p_text);
        doc.append_child(body, p);

        let output = doc.to_html();

        assert!(
            output.contains("1 &lt; 2 &amp; 3 &gt; 1"),
            "Normal text should be escaped. Got: {}",
            output
        );
    }

    #[test]
    fn test_incomplete_tag_at_eof() {
        // Test how html5ever handles "incomplete" tags like `<b</body>`
        //
        // Per the HTML spec, in TagName state, characters like '<' fall through to
        // "anything else" and are appended to the tag name. So `<b</body>` parses as:
        // - '<' -> TagOpen
        // - 'b' -> create tag 'b', TagName state
        // - '<' -> append '<' to tag name -> "b<"
        // - '/' -> SelfClosingStartTag state
        // - 'b' -> not '>', reconsume in BeforeAttributeName
        // - 'o','d','y' -> attribute name "body"
        // - '>' -> emit tag
        //
        // Both html5ever and Chrome produce: <b< body="">
        // This is correct spec-compliant behavior.
        let html = t("<!DOCTYPE html><html><body>hr<b</body></html>");
        let doc = parse(&html);

        let body = doc.body().expect("should have body");
        let children: Vec<_> = body.children(&doc.arena).collect();

        // We get 2 children: Text("hr") and Element("b<" with attr body="")
        assert_eq!(children.len(), 2);

        // First child is text "hr"
        match &doc.get(children[0]).kind {
            NodeKind::Text(t) => assert_eq!(t.as_ref(), "hr"),
            other => panic!("Expected Text node, got {:?}", other),
        }

        // Second child is element "b<" with attribute "body"
        match &doc.get(children[1]).kind {
            NodeKind::Element(elem) => {
                assert_eq!(elem.tag.as_ref(), "b<");
                assert_eq!(elem.attrs.len(), 1);
                assert_eq!(elem.attrs[0].0.local.as_ref(), "body");
                assert_eq!(elem.attrs[0].1.as_ref(), "");
            }
            other => panic!("Expected Element node, got {:?}", other),
        }
    }

    #[test]
    fn test_foster_parented_text_merging() {
        // Test that foster-parented text nodes get merged
        // Input: <table>+<tr>more</table>
        // The "+" and "more" should be foster-parented before the table
        // and merged into a single text node
        let html = t("<!DOCTYPE html><html><body><table>+<tr>more</table></body></html>");
        let doc = parse(&html);

        let body = doc.body().expect("should have body");
        let children: Vec<_> = body.children(&doc.arena).collect();

        trace!("Body has {} children", children.len());
        for (i, child_id) in children.iter().enumerate() {
            let node = doc.get(*child_id);
            trace!("  Child {}: {:?}", i, node.kind);
        }

        // Should have 2 children: merged text node + table
        // Browser produces: TEXT "+more" then TABLE
        assert_eq!(children.len(), 2, "Should have text + table");

        match &doc.get(children[0]).kind {
            NodeKind::Text(t) => assert_eq!(t.as_ref(), "+more", "Text should be merged"),
            other => panic!("Expected Text node, got {:?}", other),
        }

        match &doc.get(children[1]).kind {
            NodeKind::Element(elem) => assert_eq!(elem.tag.as_ref(), "table"),
            other => panic!("Expected table element, got {:?}", other),
        }
    }

    #[test]
    fn test_parser_stray_body_tags_merge_attrs() {
        // Per HTML5 spec, stray <body> tags should merge their attributes into
        // the existing body element. This matches browser behavior.
        // Input observed from browser fuzzer:
        // "t-Eh<body>g>selectedt-Eh<body hrselectedt"
        let html =
            t("<!DOCTYPE html><html><body>t-Eh<body>g>selectedt-Eh<body hrselectedt</body></html>");
        trace!(%html, "Input");
        let doc = parse(&html);

        let body = doc.body().expect("should have body");
        let body_data = doc.get(body);
        match &body_data.kind {
            NodeKind::Element(elem) => {
                trace!(attrs = ?elem.attrs, "Body attrs");
                // Per spec, attrs from stray body tags should be merged
                // The input "<body hrselectedt" gets parsed as body with attrs "hrselectedt<" and "body"
                assert_eq!(
                    elem.attrs.len(),
                    2,
                    "Body should receive attributes from stray <body> tokens per HTML5 spec: {:?}",
                    elem.attrs
                );
            }
            other => panic!("Expected body element, got {:?}", other),
        }
    }

    #[test]
    fn test_parser_mismatch_li_u_svg() {
        // Regression test for <li><u><li><svg> mismatch
        // Browser: second <li> contains an implied <u> wrapping <svg>
        // html5ever should match browser output here
        let html = t("<!DOCTYPE html><html><body><li><u><li><svg></body></html>");
        let doc = parse(&html);

        let body = doc.body().expect("should have body");
        let children: Vec<_> = body.children(&doc.arena).collect();

        // Expect two <li> elements
        assert_eq!(children.len(), 2, "Should have two <li> siblings");

        let first_li = children[0];
        let second_li = children[1];

        match &doc.get(first_li).kind {
            NodeKind::Element(elem) => assert_eq!(elem.tag.as_ref(), "li"),
            other => panic!("Expected first child to be <li>, got {:?}", other),
        }

        match &doc.get(second_li).kind {
            NodeKind::Element(elem) => assert_eq!(elem.tag.as_ref(), "li"),
            other => panic!("Expected second child to be <li>, got {:?}", other),
        }

        // First <li> should contain a <u>
        let first_li_children: Vec<_> = first_li.children(&doc.arena).collect();
        assert_eq!(
            first_li_children.len(),
            1,
            "First <li> should have one child"
        );

        match &doc.get(first_li_children[0]).kind {
            NodeKind::Element(elem) => assert_eq!(elem.tag.as_ref(), "u"),
            other => panic!("Expected first <li> child to be <u>, got {:?}", other),
        }

        // Second <li> should contain a <u>, which contains <svg>
        let second_li_children: Vec<_> = second_li.children(&doc.arena).collect();
        assert_eq!(
            second_li_children.len(),
            1,
            "Second <li> should have one child"
        );

        let second_u = second_li_children[0];
        match &doc.get(second_u).kind {
            NodeKind::Element(elem) => assert_eq!(elem.tag.as_ref(), "u"),
            other => panic!("Expected second <li> child to be <u>, got {:?}", other),
        }

        let second_u_children: Vec<_> = second_u.children(&doc.arena).collect();
        assert_eq!(
            second_u_children.len(),
            1,
            "Second <u> should have one child"
        );

        match &doc.get(second_u_children[0]).kind {
            NodeKind::Element(elem) => assert_eq!(elem.tag.as_ref(), "svg"),
            other => panic!("Expected <u> to wrap <svg>, got {:?}", other),
        }
    }

    #[test]
    fn test_parser_lf_cr_attribute_handling() {
        // Test for LF-CR (\n\r) in tag attribute parsing
        // Input: "p<H\nz\n\rH\nt"
        //
        // Per INFRA spec "normalize newlines":
        // 1. Replace CR-LF pairs with LF
        // 2. Replace remaining CR with LF
        //
        // So \n\r becomes \n\n (LF stays, CR becomes LF) = two whitespace chars
        // This means z and h are separate attributes.
        //
        // Browser behavior:
        // - Firefox: z, h, t<, body (4 attrs) ✓ spec-compliant
        // - Safari:  z, h, t<, body (4 attrs) ✓ spec-compliant
        // - Chrome:  z, ht<, body (3 attrs) ✗ Chrome bug
        let html = t("<!DOCTYPE html><html><body>p<H\nz\n\rH\nt</body></html>");
        trace!(%html, "Input");
        let doc = parse(&html);

        let body = doc.body().expect("should have body");
        let children: Vec<_> = body.children(&doc.arena).collect();

        // Find the <h> element
        let h_elem = children.iter().find(
            |&&id| matches!(&doc.get(id).kind, NodeKind::Element(e) if e.tag.as_ref() == "h"),
        );

        let h_id = h_elem.expect("should have <h> element");
        let h_data = doc.get(*h_id);

        match &h_data.kind {
            NodeKind::Element(elem) => {
                trace!(attrs = ?elem.attrs, "H element attrs");
                // Per INFRA spec, \n\r → \n\n, creating 4 separate attributes
                assert_eq!(
                    elem.attrs.len(),
                    4,
                    "Should have 4 attributes (z, h, t<, body) per INFRA spec. Got: {:?}",
                    elem.attrs
                );

                // Verify the separate h and t< attributes exist (not combined as ht<)
                let has_h = elem.attrs.iter().any(|(k, _)| k.local.as_ref() == "h");
                let has_t = elem.attrs.iter().any(|(k, _)| k.local.as_ref() == "t<");
                assert!(has_h, "Should have 'h' attribute, got: {:?}", elem.attrs);
                assert!(has_t, "Should have 't<' attribute, got: {:?}", elem.attrs);
            }
            other => panic!("Expected Element, got {:?}", other),
        }
    }

    #[test]
    fn test_dt_dd_outside_dl() {
        // Test <dt> and <dd> parsing outside of <dl> context
        // These have special rules about auto-closing and tree construction
        //
        // Chrome produces: <s></s><dt><s></s></dt><dd></dd>
        // Need to verify html5ever matches
        let html = t("<!DOCTYPE html><html><body><s><dt></s><dd></body></html>");
        trace!(%html, "Input");
        let doc = parse(&html);
        let body_html = doc.to_body_html();
        trace!(%body_html, "Body HTML");

        // Chrome output: <s></s><dt><s></s></dt><dd></dd>
        // The <dt> causes <s> to close, then </s> reopens and closes inside <dt>
        assert_eq!(
            body_html, "<s></s><dt><s></s></dt><dd></dd>",
            "Should match browser output for dt/dd outside dl"
        );
    }

    #[test]
    fn test_dt_dd_nested_s() {
        // Test nested <s> with dt/dd
        // Chrome: <s><s></s><dt><s></s></dt><dd></dd></s>
        let html = t("<!DOCTYPE html><html><body><s><s><dt/></s><dd></body></html>");
        let doc = parse(&html);
        let body_html = doc.to_body_html();
        trace!(%body_html, "nested s dt dd");
        assert_eq!(
            body_html, "<s><s></s><dt><s></s></dt><dd></dd></s>",
            "nested s with dt/dd"
        );
    }

    #[test]
    fn test_dt_dd_with_custom_tag() {
        // Test with custom tag name <s?>
        // Chrome: <s><s?></s?></s><dt><s></s></dt><dd></dd>
        let html = t("<!DOCTYPE html><html><body><s><s?><dt/></s><dd></body></html>");
        let doc = parse(&html);
        let body_html = doc.to_body_html();
        trace!(%body_html, "s s? dt dd");
        assert_eq!(
            body_html, "<s><s?></s?></s><dt><s></s></dt><dd></dd>",
            "s with custom s? and dt/dd"
        );
    }

    #[test]
    fn test_malformed_close_tag_with_slash() {
        // Test </S/<DD> - malformed close tag with slash
        // Chrome: <s></s><dt><s></s></dt>  (no dd!)
        let html = t("<!DOCTYPE html><html><body><s><DT/></S/<DD></body></html>");
        let doc = parse(&html);
        let body_html = doc.to_body_html();
        trace!(%body_html, "malformed close tag");
        assert_eq!(
            body_html, "<s></s><dt><s></s></dt>",
            "malformed </S/ should eat the <DD>"
        );
    }

    #[test]
    fn test_leading_whitespace_in_body_content() {
        // Regression test for fuzzer finding: leading whitespace before text
        // was being lost when parsing body content without DOCTYPE.
        //
        // The scenario: browser's innerHTML returns " *" and we parse it.
        // Previously this would produce "*" (space lost) because the
        // prepended DOCTYPE caused the space to become inter-element whitespace.

        // Test 1: Full document preserves whitespace
        let html = t("<!DOCTYPE html><html><body> *</body></html>");
        let doc = parse(&html);
        assert_eq!(
            doc.to_body_html(),
            " *",
            "Full document should preserve leading space"
        );

        // Test 2: Parsing body innerHTML (no wrapper) should also preserve whitespace
        // This is the exact scenario from the apply_parity fuzzer
        let body_inner = t(" *");
        let doc2 = parse(&body_inner);
        assert_eq!(
            doc2.to_body_html(),
            " *",
            "Parsing body innerHTML ' *' should preserve leading space"
        );
    }

    #[test]
    fn test_noscript_with_malformed_tag() {
        // Test noscript parsing with malformed input like `<noscript n</body></html>`
        //
        // When scripting is enabled (default), noscript is a raw text element.
        // The tokenizer should only recognize `</noscript>` as an end tag.
        // Other "end tags" like `</body>` and `</html>` should be emitted as text.
        //
        // The input `<noscript n</body></html>` should parse as:
        // - `<noscript n` -> element with attribute `n<` and attribute `body` (due to </ being part of attr)
        //   OR as noscript with attr `n` depending on exact tokenization
        // - Content depends on whether we enter rawtext mode
        //
        // This test documents html5ever's behavior for this edge case.
        let html = t("<!DOCTYPE html><html><body><noscript n</body></html>");
        trace!(%html, "Input");
        let doc = parse(&html);

        let body = doc.body().expect("should have body");
        let body_html = doc.to_body_html();
        trace!(%body_html, "Body HTML");

        // Find the noscript element
        let noscript = body.children(&doc.arena).find(
            |&id| matches!(&doc.get(id).kind, NodeKind::Element(e) if e.tag.as_ref() == "noscript"),
        );

        if let Some(noscript_id) = noscript {
            let noscript_data = doc.get(noscript_id);
            if let NodeKind::Element(elem) = &noscript_data.kind {
                trace!(attrs = ?elem.attrs, "noscript attrs");
            }

            // Check children of noscript
            let noscript_children: Vec<_> = noscript_id.children(&doc.arena).collect();
            trace!(num_children = noscript_children.len(), "noscript children");

            for (i, child_id) in noscript_children.iter().enumerate() {
                let child = doc.get(*child_id);
                trace!(i, kind = ?child.kind, "noscript child");
            }

            // Document the actual behavior
            eprintln!("html5ever noscript parsing result:");
            eprintln!("  body innerHTML: {}", body_html);
            eprintln!("  noscript children: {}", noscript_children.len());
        } else {
            eprintln!("No noscript element found in body");
            eprintln!("  body innerHTML: {}", body_html);
        }
    }

    #[test]
    fn test_noscript_placement_in_head() {
        // Test where noscript ends up when it appears before body
        // Input: <!DOCTYPE html><noscript>nn
        //
        // html5ever (scripting enabled): noscript goes in <head>, body is empty
        // Browser behavior may differ
        let html = t("<!DOCTYPE html><noscript>nn");
        let doc = parse(&html);

        // Check what's in head
        let head = doc.head().expect("should have head");
        let head_children: Vec<_> = head.children(&doc.arena).collect();
        eprintln!("Head children: {}", head_children.len());
        for child_id in &head_children {
            let child = doc.get(*child_id);
            eprintln!("  Head child: {:?}", child.kind);
        }

        // Check what's in body
        let body = doc.body().expect("should have body");
        let body_children: Vec<_> = body.children(&doc.arena).collect();
        eprintln!("Body children: {}", body_children.len());
        for child_id in &body_children {
            let child = doc.get(*child_id);
            eprintln!("  Body child: {:?}", child.kind);
        }

        // Full HTML output
        eprintln!("Full HTML: {}", doc.to_html());
    }

    #[test]
    fn test_noscript_rawtext_vs_browser() {
        // Test: parse with html5ever, then re-parse the body innerHTML
        // This simulates what the browser does: get innerHTML, set innerHTML
        //
        // html5ever (scripting enabled):
        //   Input: <noscript n</body></html>
        //   Output: <noscript n<="" body="">&lt;/html&gt;</noscript>
        //
        // Browser (scripting enabled):
        //   When we get innerHTML of a noscript, the content is HTML-escaped
        //   When we set innerHTML, that escaped content gets parsed as TEXT
        //   But the browser also parses noscript as rawtext when scripting is enabled
        //
        // The issue: if the browser's innerHTML getter escapes the content,
        // then re-parsing that innerHTML produces different content.
        let html = t("<!DOCTYPE html><html><body><noscript n</body></html>");
        let doc = parse(&html);
        let body_html = doc.to_body_html();

        // Now parse that body_html again (simulating innerHTML round-trip)
        let body_html_tendril = t(&body_html);
        let reparsed = parse(&body_html_tendril);
        let reparsed_body_html = reparsed.to_body_html();

        eprintln!("Original body HTML: {}", body_html);
        eprintln!("Re-parsed body HTML: {}", reparsed_body_html);

        // They should be the same (idempotent)
        assert_eq!(
            body_html, reparsed_body_html,
            "innerHTML should be idempotent for noscript"
        );
    }

    #[test]
    fn test_parse_body_fragment_structure() {
        // Understand what parse_body_fragment actually produces
        let input = t("[");
        let doc = parse_body_fragment(&input);

        eprintln!("=== Fragment parse output ===");
        eprintln!("Root dump:\n{}", doc.dump_subtree(doc.root));

        // Check root element
        let root_data = doc.get(doc.root);
        eprintln!("Root kind: {:?}", root_data.kind);

        // Check for body
        eprintln!("Has body(): {:?}", doc.body());

        // List root's children
        let children: Vec<_> = doc.children(doc.root).collect();
        eprintln!("Root has {} children", children.len());
        for (i, child) in children.iter().enumerate() {
            eprintln!("  Child {}: {:?}", i, doc.get(*child).kind);
        }
    }

    #[test]
    fn test_math_fragment_parsing() {
        // Test how <math> is parsed in fragment context
        let input = t("<math>6<mn>x</mn></math>");
        let doc = parse_body_fragment(&input);
        eprintln!("Fragment parse of <math>:");
        eprintln!("{}", doc.dump_subtree(doc.root));
    }

    #[test]
    fn test_math_with_li_fragment_parsing() {
        // Test <li> inside <math> - this breaks out of foreign content
        let input = t("<li>a<math>6<mn>x<li>b</mn></math>");
        let doc = parse_body_fragment(&input);
        eprintln!("Fragment parse of <li><math>...<li>:");
        eprintln!("{}", doc.dump_subtree(doc.root));

        // Check innerHTML roundtrip
        let html = doc.to_html_without_doctype();
        eprintln!("Serialized HTML: {}", html);

        // Re-parse
        let html_tendril = t(&html);
        let reparsed = parse_body_fragment(&html_tendril);
        eprintln!("Re-parsed:");
        eprintln!("{}", reparsed.dump_subtree(reparsed.root));
    }

    #[test]
    fn test_bogus_comment_parsing() {
        let input = t("<o><!D");
        let doc = parse_body_fragment(&input);
        println!("Input: {:?}", input);
        println!("Tree:\n{}", doc.dump_subtree(doc.root));
    }

    #[test]
    fn test_bogus_comment_variations() {
        let cases = ["<o><!D", "<o><!D>", "<o><!--D-->", "<o><!>", "<!D>text"];
        for input in cases {
            eprintln!("\n=== Input: {:?} ===", input);
            let tendril = t(input);
            let doc = parse_body_fragment(&tendril);
            eprintln!("{}", doc.dump_subtree(doc.root));
        }
    }
}
