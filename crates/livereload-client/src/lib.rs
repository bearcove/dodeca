//! WASM client for dodeca live reload
//!
//! Receives serialized DOM patches from the server and applies them to the real DOM.

#![allow(clippy::disallowed_types)] // serde needed for postcard serialization

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use web_sys::{Document, Element, Node};

/// A path to a node in the DOM tree
/// e.g., [0, 2, 1] means: body's child 0, then child 2, then child 1
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodePath(pub Vec<usize>);

/// Operations to transform the DOM
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Patch {
    /// Replace node at path with new HTML
    Replace { path: NodePath, html: String },

    /// Insert HTML before the node at path
    InsertBefore { path: NodePath, html: String },

    /// Insert HTML after the node at path
    InsertAfter { path: NodePath, html: String },

    /// Append HTML as last child of node at path
    AppendChild { path: NodePath, html: String },

    /// Remove the node at path
    Remove { path: NodePath },

    /// Update text content of node at path
    SetText { path: NodePath, text: String },

    /// Set attribute on node at path
    SetAttribute {
        path: NodePath,
        name: String,
        value: String,
    },

    /// Remove attribute from node at path
    RemoveAttribute { path: NodePath, name: String },
}

/// Apply serialized patches to the DOM
/// Returns the number of patches applied, or an error message
#[wasm_bindgen]
pub fn apply_patches(data: &[u8]) -> Result<usize, JsValue> {
    let patches: Vec<Patch> = postcard::from_bytes(data)
        .map_err(|e| JsValue::from_str(&format!("deserialize error: {e}")))?;

    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window.document().ok_or_else(|| JsValue::from_str("no document"))?;

    let count = patches.len();
    for patch in patches {
        apply_patch(&document, patch)?;
    }

    Ok(count)
}

fn apply_patch(doc: &Document, patch: Patch) -> Result<(), JsValue> {
    match patch {
        Patch::SetText { path, text } => {
            let node = find_node(doc, &path)?;
            node.set_text_content(Some(&text));
        }
        Patch::SetAttribute { path, name, value } => {
            let el = find_element(doc, &path)?;
            el.set_attribute(&name, &value)?;
        }
        Patch::RemoveAttribute { path, name } => {
            let el = find_element(doc, &path)?;
            el.remove_attribute(&name)?;
        }
        Patch::Remove { path } => {
            let node = find_node(doc, &path)?;
            if let Some(parent) = node.parent_node() {
                parent.remove_child(&node)?;
            }
        }
        Patch::Replace { path, html } => {
            let el = find_element(doc, &path)?;
            el.set_outer_html(&html);
        }
        Patch::InsertBefore { path, html } => {
            let el = find_element(doc, &path)?;
            el.insert_adjacent_html("beforebegin", &html)?;
        }
        Patch::InsertAfter { path, html } => {
            let el = find_element(doc, &path)?;
            el.insert_adjacent_html("afterend", &html)?;
        }
        Patch::AppendChild { path, html } => {
            let el = find_element(doc, &path)?;
            el.insert_adjacent_html("beforeend", &html)?;
        }
    }
    Ok(())
}

fn find_node(doc: &Document, path: &NodePath) -> Result<Node, JsValue> {
    let body = doc.body().ok_or_else(|| JsValue::from_str("no body"))?;
    let mut current: Node = body.into();

    for &idx in &path.0 {
        let children = current.child_nodes();
        current = children
            .item(idx as u32)
            .ok_or_else(|| JsValue::from_str(&format!("child {idx} not found")))?;
    }

    Ok(current)
}

fn find_element(doc: &Document, path: &NodePath) -> Result<Element, JsValue> {
    let node = find_node(doc, path)?;
    node.dyn_into::<Element>()
        .map_err(|_| JsValue::from_str("node is not an element"))
}

/// Log a message to the browser console (for debugging)
#[wasm_bindgen]
pub fn log(msg: &str) {
    web_sys::console::log_1(&JsValue::from_str(msg));
}
