//! Apply patches to HTML documents.
//!
//! For property testing: apply(A, diff(A, B)) == B

use super::translate::{InsertContent, NodePath, NodeRef, Patch, PropChange};
use std::collections::HashMap;
use std::fmt::Write;

/// A simple DOM element for patch application.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct Element {
    /// Tag name
    #[facet(tag)]
    pub tag: String,
    /// Attributes as key-value pairs
    #[facet(flatten)]
    pub attrs: HashMap<String, String>,
    /// Child nodes
    #[facet(flatten)]
    pub children: Vec<Content>,
}

/// DOM content - either an element or text.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum Content {
    /// An element node
    Element(Element),
    /// A text node
    #[facet(text)]
    Text(String),
}

impl Element {
    /// Get mutable reference to children at a path.
    pub fn children_mut(&mut self, path: &[usize]) -> Result<&mut Vec<Content>, String> {
        let mut current = self;
        for &idx in path {
            let child = current
                .children
                .get_mut(idx)
                .ok_or_else(|| format!("path index {idx} out of bounds"))?;
            current = match child {
                Content::Element(e) => e,
                Content::Text(_) => return Err("cannot navigate through text node".to_string()),
            };
        }
        Ok(&mut current.children)
    }

    /// Get mutable reference to attrs at a path.
    pub fn attrs_mut(&mut self, path: &[usize]) -> Result<&mut HashMap<String, String>, String> {
        let mut current = self;
        for &idx in path {
            let child = current
                .children
                .get_mut(idx)
                .ok_or_else(|| format!("path index {idx} out of bounds"))?;
            current = match child {
                Content::Element(e) => e,
                Content::Text(_) => return Err("cannot navigate through text node".to_string()),
            };
        }
        Ok(&mut current.attrs)
    }

    /// Get mutable reference to content at a path.
    pub fn get_content_mut(&mut self, path: &[usize]) -> Result<&mut Content, String> {
        if path.is_empty() {
            return Err("cannot get content at empty path".to_string());
        }
        let parent_path = &path[..path.len() - 1];
        let idx = path[path.len() - 1];
        let children = self.children_mut(parent_path)?;
        children
            .get_mut(idx)
            .ok_or_else(|| format!("index {idx} out of bounds"))
    }

    /// Serialize this element to an HTML string (body content only).
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        self.write_html(&mut out);
        out
    }

    /// Write HTML to a string buffer.
    fn write_html(&self, out: &mut String) {
        write!(out, "<{}", self.tag).unwrap();

        // Sort attributes for deterministic output
        let mut attrs: Vec<_> = self.attrs.iter().collect();
        attrs.sort_by_key(|(k, _)| *k);
        for (name, value) in attrs {
            write!(out, " {}=\"{}\"", name, escape_attr(value)).unwrap();
        }
        out.push('>');

        for child in &self.children {
            child.write_html(out);
        }

        out.push_str("</");
        out.push_str(&self.tag);
        out.push('>');
    }

    /// Get concatenated text content of this element and all descendants.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        for child in &self.children {
            match child {
                Content::Text(t) => out.push_str(t),
                Content::Element(e) => e.collect_text(out),
            }
        }
    }
}

impl Content {
    fn write_html(&self, out: &mut String) {
        match self {
            Content::Text(t) => out.push_str(&escape_text(t)),
            Content::Element(e) => e.write_html(out),
        }
    }
}

/// Escape text content for HTML.
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escape attribute value for HTML.
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Parse an HTML string into an Element tree, returning the body.
pub fn parse_html(html: &str) -> Result<Element, String> {
    Ok(crate::parse_untyped(html))
}

// =============================================================================
// Conversion from untyped_dom types
// =============================================================================

impl From<&crate::untyped_dom::Element> for Element {
    fn from(e: &crate::untyped_dom::Element) -> Self {
        Element {
            tag: e.tag.clone(),
            attrs: e.attrs.clone(),
            children: e
                .children
                .iter()
                .filter_map(|n| Content::try_from(n).ok())
                .collect(),
        }
    }
}

impl From<crate::untyped_dom::Element> for Element {
    fn from(e: crate::untyped_dom::Element) -> Self {
        Element {
            tag: e.tag,
            attrs: e.attrs,
            children: e
                .children
                .into_iter()
                .filter_map(|n| Content::try_from(n).ok())
                .collect(),
        }
    }
}

impl TryFrom<&crate::untyped_dom::Node> for Content {
    type Error = ();

    fn try_from(n: &crate::untyped_dom::Node) -> Result<Self, Self::Error> {
        match n {
            crate::untyped_dom::Node::Element(e) => Ok(Content::Element(Element::from(e))),
            crate::untyped_dom::Node::Text(t) => Ok(Content::Text(t.clone())),
            crate::untyped_dom::Node::Comment(_) => Err(()), // Skip comments
        }
    }
}

impl TryFrom<crate::untyped_dom::Node> for Content {
    type Error = ();

    fn try_from(n: crate::untyped_dom::Node) -> Result<Self, Self::Error> {
        match n {
            crate::untyped_dom::Node::Element(e) => Ok(Content::Element(Element::from(e))),
            crate::untyped_dom::Node::Text(t) => Ok(Content::Text(t)),
            crate::untyped_dom::Node::Comment(_) => Err(()), // Skip comments
        }
    }
}

// =============================================================================
// Conversion to untyped_dom types
// =============================================================================

impl From<&Element> for crate::untyped_dom::Element {
    fn from(e: &Element) -> Self {
        crate::untyped_dom::Element {
            tag: e.tag.clone(),
            ns: crate::untyped_dom::Namespace::Html,
            attrs: e.attrs.clone(),
            children: e.children.iter().map(|c| c.into()).collect(),
        }
    }
}

impl From<&Content> for crate::untyped_dom::Node {
    fn from(c: &Content) -> Self {
        match c {
            Content::Element(e) => {
                crate::untyped_dom::Node::Element(crate::untyped_dom::Element::from(e))
            }
            Content::Text(t) => crate::untyped_dom::Node::Text(t.clone()),
        }
    }
}
/// Navigate within an element using a relative path (all but the last index) and return the children vec.
/// The last element of the path is the target index within the returned children.
/// Used for operations on nodes within detached slots.
fn navigate_to_children_in_slot<'a>(
    slot_node: &'a mut Element,
    rel_path: Option<&NodePath>,
) -> Result<&'a mut Vec<Content>, String> {
    let mut current = slot_node;
    if let Some(path) = rel_path {
        // Navigate all but the last segment (the last is the target index)
        let nav_path = if path.0.len() > 1 {
            &path.0[..path.0.len() - 1]
        } else {
            &[]
        };
        for &idx in nav_path {
            let child = current
                .children
                .get_mut(idx)
                .ok_or_else(|| format!("path index {idx} out of bounds in slot"))?;
            current = match child {
                Content::Element(e) => e,
                Content::Text(_) => {
                    return Err("cannot navigate through text node".to_string());
                }
            };
        }
    }
    Ok(&mut current.children)
}

/// Apply a list of patches to an Element tree in order.
pub fn apply_patches(root: &mut Element, patches: &[Patch]) -> Result<(), String> {
    // Slots hold Content (either Element or Text) that was displaced during edits
    let mut slots: HashMap<u32, Content> = HashMap::new();
    for patch in patches {
        apply_patch(root, patch, &mut slots)?;
    }
    Ok(())
}

/// Apply a single patch.
fn apply_patch(
    root: &mut Element,
    patch: &Patch,
    slots: &mut HashMap<u32, Content>,
) -> Result<(), String> {
    match patch {
        Patch::InsertElement {
            parent,
            position,
            tag,
            attrs,
            children,
            detach_to_slot,
        } => {
            // Create element with its attrs and children
            let new_element = Element {
                tag: tag.clone(),
                attrs: attrs.iter().cloned().collect(),
                children: children.iter().map(insert_content_to_content).collect(),
            };
            let new_content = Content::Element(new_element);

            insert_at_position(root, slots, parent, *position, new_content, *detach_to_slot)?;
        }
        Patch::InsertText {
            parent,
            position,
            text,
            detach_to_slot,
        } => {
            let new_content = Content::Text(text.clone());
            insert_at_position(root, slots, parent, *position, new_content, *detach_to_slot)?;
        }
        Patch::Remove { node } => {
            match node {
                NodeRef::Path(path) => {
                    if path.0.is_empty() {
                        return Err("Remove: cannot remove root".to_string());
                    }
                    let parent_path = &path.0[..path.0.len() - 1];
                    let idx = path.0[path.0.len() - 1];
                    let children = root
                        .children_mut(parent_path)
                        .map_err(|e| format!("Remove: {e}"))?;
                    if idx < children.len() {
                        // Swap with placeholder instead of remove (no shifting!)
                        children[idx] = Content::Text(String::new());
                    } else {
                        return Err(format!("Remove: index {idx} out of bounds"));
                    }
                }
                NodeRef::Slot(slot, _relative_path) => {
                    // Just remove from slots - the node was already detached
                    slots.remove(slot);
                }
            }
        }
        Patch::SetText { path, text } => {
            // Path points to a specific text node (e.g., [0, 1] = element at 0, text child at 1).
            // Navigate to the parent and replace just that child.
            if path.0.is_empty() {
                return Err("SetText: cannot set text on root".to_string());
            }
            let parent_path = &path.0[..path.0.len() - 1];
            let text_idx = path.0[path.0.len() - 1];
            let children = root
                .children_mut(parent_path)
                .map_err(|e| format!("SetText: {e}"))?;
            if text_idx >= children.len() {
                return Err(format!(
                    "SetText: index {text_idx} out of bounds (len={})",
                    children.len()
                ));
            }
            children[text_idx] = Content::Text(text.clone());
        }
        Patch::SetAttribute { path, name, value } => {
            let attrs = root
                .attrs_mut(&path.0)
                .map_err(|e| format!("SetAttribute: {e}"))?;
            attrs.insert(name.clone(), value.clone());
        }
        Patch::RemoveAttribute { path, name } => {
            let attrs = root
                .attrs_mut(&path.0)
                .map_err(|e| format!("RemoveAttribute: {e}"))?;
            attrs.remove(name);
        }
        Patch::Move {
            from,
            to,
            detach_to_slot,
        } => {
            debug!(?from, ?to, ?detach_to_slot, "apply Move");
            debug!(
                slots_before = ?slots.keys().collect::<Vec<_>>(),
                "apply Move slots state"
            );

            // Get the content to move (either from a path or from a slot)
            let content = match from {
                NodeRef::Path(from_path) => {
                    if from_path.0.is_empty() {
                        return Err("Move: cannot move root".to_string());
                    }
                    let from_parent_path = &from_path.0[..from_path.0.len() - 1];
                    let from_idx = from_path.0[from_path.0.len() - 1];
                    let from_children = root
                        .children_mut(from_parent_path)
                        .map_err(|e| format!("Move: {e}"))?;
                    if from_idx >= from_children.len() {
                        return Err(format!("Move: source index {from_idx} out of bounds"));
                    }
                    // Swap with placeholder instead of remove (no shifting!)
                    std::mem::replace(&mut from_children[from_idx], Content::Text(String::new()))
                }
                NodeRef::Slot(slot, _relative_path) => slots
                    .remove(slot)
                    .ok_or_else(|| format!("Move: slot {slot} not found"))?,
            };

            // Place the content at the target location (either in tree or in a slot)
            match to {
                NodeRef::Path(to_path) => {
                    if to_path.0.is_empty() {
                        return Err("Move: cannot move to root".to_string());
                    }
                    let to_parent_path = &to_path.0[..to_path.0.len() - 1];
                    let to_idx = to_path.0[to_path.0.len() - 1];

                    // Check if we need to detach the occupant at the target position
                    if let Some(slot) = detach_to_slot {
                        let to_children = root
                            .children_mut(to_parent_path)
                            .map_err(|e| format!("Move: {e}"))?;
                        debug!(
                            to_idx,
                            to_children_len = to_children.len(),
                            "Move detach check"
                        );
                        if to_idx < to_children.len() {
                            let occupant = std::mem::replace(
                                &mut to_children[to_idx],
                                Content::Text(String::new()),
                            );
                            debug!(slot, ?occupant, "Move detach: inserting occupant into slot");
                            slots.insert(*slot, occupant);
                        }
                    }

                    // Place the content at the target location
                    let to_children = root
                        .children_mut(to_parent_path)
                        .map_err(|e| format!("Move: {e}"))?;
                    // Grow the array with empty text placeholders if needed
                    while to_children.len() <= to_idx {
                        to_children.push(Content::Text(String::new()));
                    }
                    to_children[to_idx] = content;
                }
                NodeRef::Slot(target_slot, rel_path) => {
                    // Move into a slot (detached subtree)
                    // Get the target index from the relative path
                    let to_idx = rel_path
                        .as_ref()
                        .and_then(|p| p.0.last().copied())
                        .ok_or_else(|| "Move: slot target missing position index".to_string())?;

                    // Handle displacement if needed (in separate scope to release borrow)
                    if let Some(slot) = detach_to_slot {
                        let slot_content = slots
                            .get_mut(target_slot)
                            .ok_or_else(|| format!("Move: target slot {target_slot} not found"))?;

                        let target_children = match slot_content {
                            Content::Element(e) => {
                                navigate_to_children_in_slot(e, rel_path.as_ref())?
                            }
                            Content::Text(_) => {
                                return Err(
                                    "Move: target slot contains text, not element".to_string()
                                );
                            }
                        };

                        if to_idx < target_children.len() {
                            let occupant = std::mem::replace(
                                &mut target_children[to_idx],
                                Content::Text(String::new()),
                            );
                            slots.insert(*slot, occupant);
                        }
                    }

                    // Re-get the slot element (previous borrow was released)
                    let slot_content = slots
                        .get_mut(target_slot)
                        .ok_or_else(|| format!("Move: target slot {target_slot} not found"))?;

                    let target_children = match slot_content {
                        Content::Element(e) => navigate_to_children_in_slot(e, rel_path.as_ref())?,
                        Content::Text(_) => {
                            return Err("Move: target slot contains text, not element".to_string());
                        }
                    };

                    // Grow and place
                    while target_children.len() <= to_idx {
                        target_children.push(Content::Text(String::new()));
                    }
                    target_children[to_idx] = content;
                }
            }
        }
        Patch::UpdateProps { path, changes } => {
            apply_update_props(root, path, changes)?;
        }
    }
    Ok(())
}

/// Apply property updates, handling `_text` specially.
fn apply_update_props(
    root: &mut Element,
    path: &NodePath,
    changes: &[PropChange],
) -> Result<(), String> {
    // Get the content at path
    let content = root
        .get_content_mut(&path.0)
        .map_err(|e| format!("UpdateProps: {e}"))?;

    for change in changes {
        if change.name == "_text" {
            // Special handling for _text: update the text content directly
            match content {
                Content::Text(t) => {
                    if let Some(text) = &change.value {
                        *t = text.clone();
                    } else {
                        *t = String::new();
                    }
                }
                Content::Element(elem) => {
                    // Update text child of element
                    if let Some(text) = &change.value {
                        if elem.children.is_empty() {
                            elem.children.push(Content::Text(text.clone()));
                        } else {
                            let mut found_text = false;
                            for child in &mut elem.children {
                                if let Content::Text(t) = child {
                                    *t = text.clone();
                                    found_text = true;
                                    break;
                                }
                            }
                            if !found_text {
                                elem.children[0] = Content::Text(text.clone());
                            }
                        }
                    } else {
                        elem.children.retain(|c| !matches!(c, Content::Text(_)));
                    }
                }
            }
        } else {
            // Regular attribute - only valid on elements
            match content {
                Content::Element(elem) => {
                    if let Some(value) = &change.value {
                        elem.attrs.insert(change.name.clone(), value.clone());
                    } else {
                        elem.attrs.remove(&change.name);
                    }
                }
                Content::Text(_) => {
                    return Err(format!(
                        "UpdateProps: cannot set attribute '{}' on text node",
                        change.name
                    ));
                }
            }
        }
    }

    Ok(())
}

/// Helper to insert content at a position, handling displacement to slots.
fn insert_at_position(
    root: &mut Element,
    slots: &mut HashMap<u32, Content>,
    parent: &NodeRef,
    position: usize,
    new_content: Content,
    detach_to_slot: Option<u32>,
) -> Result<(), String> {
    match parent {
        NodeRef::Path(path) => {
            let children = root
                .children_mut(&path.0)
                .map_err(|e| format!("Insert: {e}"))?;

            // In Chawathe semantics, Insert does NOT shift - it places at position
            // and whatever was there gets displaced (detached to a slot).
            if let Some(slot) = detach_to_slot
                && position < children.len()
            {
                let occupant =
                    std::mem::replace(&mut children[position], Content::Text(String::new()));
                slots.insert(slot, occupant);
            }

            // Grow the array with empty text placeholders if needed
            while children.len() <= position {
                children.push(Content::Text(String::new()));
            }
            children[position] = new_content;
        }
        NodeRef::Slot(parent_slot, relative_path) => {
            // Parent is in a slot - inserting into a detached subtree
            let slot_elem = match slots.get_mut(parent_slot) {
                Some(Content::Element(e)) => e,
                Some(Content::Text(_)) => {
                    return Err(format!(
                        "Insert: slot {parent_slot} contains text, not an element"
                    ));
                }
                None => return Err(format!("Insert: slot {parent_slot} not found")),
            };

            // First handle displacement if needed
            if let Some(slot) = detach_to_slot {
                let children = navigate_to_children_in_slot(slot_elem, relative_path.as_ref())?;
                if position < children.len() {
                    let occupant =
                        std::mem::replace(&mut children[position], Content::Text(String::new()));
                    slots.insert(slot, occupant);
                }
            }

            // Re-get the slot element (borrow was released)
            let slot_elem = match slots.get_mut(parent_slot) {
                Some(Content::Element(e)) => e,
                _ => return Err(format!("Insert: slot {parent_slot} not found")),
            };
            let children = navigate_to_children_in_slot(slot_elem, relative_path.as_ref())?;

            // Grow the array with empty text placeholders if needed
            while children.len() <= position {
                children.push(Content::Text(String::new()));
            }
            children[position] = new_content;
        }
    }
    Ok(())
}

/// Convert InsertContent to Content.
fn insert_content_to_content(ic: &InsertContent) -> Content {
    match ic {
        InsertContent::Element {
            tag,
            attrs,
            children,
        } => Content::Element(Element {
            tag: tag.clone(),
            attrs: attrs.iter().cloned().collect(),
            children: children.iter().map(insert_content_to_content).collect(),
        }),
        InsertContent::Text(s) => Content::Text(s.clone()),
    }
}
