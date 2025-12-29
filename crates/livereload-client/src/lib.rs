//! WASM client for dodeca live reload
//!
//! Provides DOM patching functionality used by dodeca-devtools.

use wasm_bindgen::prelude::*;
use web_sys::{Document, Element, Node};

// Re-export patch types for consumers
pub use dodeca_protocol::{NodePath, Patch};

/// Apply patches to the DOM
/// Returns the number of patches applied, or an error message
pub fn apply_patches(patches: Vec<Patch>) -> Result<usize, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let count = patches.len();
    web_sys::console::log_1(&JsValue::from_str(&format!(
        "[livereload] applying {} patches",
        count
    )));
    for (i, patch) in patches.into_iter().enumerate() {
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[livereload] patch {}: {:?}",
            i, patch
        )));
        apply_patch(&document, &patch)?;
    }

    Ok(count)
}

fn apply_patch(doc: &Document, patch: &Patch) -> Result<(), JsValue> {
    match patch {
        Patch::SetText { path, text } => {
            let node = find_node(doc, path)?;
            node.set_text_content(Some(text));
        }
        Patch::SetAttribute { path, name, value } => {
            let el = find_element(doc, path)?;
            el.set_attribute(name, value)?;
        }
        Patch::RemoveAttribute { path, name } => {
            let el = find_element(doc, path)?;
            el.remove_attribute(name)?;
        }
        Patch::Remove { path } => {
            let node = find_node(doc, path)?;
            if let Some(parent) = node.parent_node() {
                parent.remove_child(&node)?;
            }
        }
        Patch::Replace { path, html } => {
            let el = find_element(doc, path)?;
            // For body element (empty path), use innerHTML since outerHTML doesn't work reliably
            if path.0.is_empty() {
                // Extract innerHTML from the html string (strip <body> tags)
                let inner = extract_body_inner(html).unwrap_or_else(|| html.clone());
                el.set_inner_html(&inner);
            } else {
                el.set_outer_html(html);
            }
        }
        Patch::InsertBefore { path, html } => {
            let el = find_element(doc, path)?;
            el.insert_adjacent_html("beforebegin", html)?;
        }
        Patch::InsertAfter { path, html } => {
            let el = find_element(doc, path)?;
            el.insert_adjacent_html("afterend", html)?;
        }
        Patch::AppendChild { path, html } => {
            let el = find_element(doc, path)?;
            el.insert_adjacent_html("beforeend", html)?;
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

/// Extract the inner content of a <body> tag
fn extract_body_inner(html: &str) -> Option<String> {
    let start = html.find("<body")?.checked_add(5)?;
    let after_tag = html.get(start..)?;
    let content_start = after_tag.find('>')?.checked_add(1)?;
    let content = after_tag.get(content_start..)?;
    let end = content.rfind("</body>")?;
    Some(content.get(..end)?.to_string())
}

/// Log a message to the browser console (for debugging)
#[wasm_bindgen]
pub fn log(msg: &str) {
    web_sys::console::log_1(&JsValue::from_str(msg));
}
