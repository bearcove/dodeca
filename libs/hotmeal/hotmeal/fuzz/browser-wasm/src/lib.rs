use browser_proto::{
    ApplyPatchesResult, Browser, BrowserDispatcher, ComputeAndApplyResult, DomAttr, DomNode,
    OwnedPatches, Patch, PatchStep,
};
use roam::Context;
use roam_session::initiate_framed;
use roam_websocket::WsTransport;
use wasm_bindgen::prelude::*;

#[derive(Clone)]
struct Handler;

impl Browser for Handler {
    async fn apply_patches(
        &self,
        _cx: &Context,
        old_html: String,
        patches: OwnedPatches,
    ) -> Result<ApplyPatchesResult, String> {
        run_apply_patches(&old_html, patches.0)
    }

    async fn compute_and_apply_patches(
        &self,
        _cx: &Context,
        old_html: String,
        new_html: String,
    ) -> Result<ComputeAndApplyResult, String> {
        run_compute_and_apply_patches(&old_html, &new_html)
    }

    async fn parse_to_dom(&self, _cx: &Context, html: String) -> DomNode {
        parse_html_to_dom(&html)
    }
}

fn log(msg: &str) {
    web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(msg));
}

/// Strip DOCTYPE prefix from HTML (innerHTML doesn't handle it)
fn strip_doctype(html: &str) -> &str {
    html.strip_prefix("<!DOCTYPE html>")
        .or_else(|| html.strip_prefix("<!doctype html>"))
        .unwrap_or(html)
}

/// Convert body element's children to a DomNode wrapped in `<html>`.
/// This matches html5ever's fragment parsing output structure.
fn body_to_html_wrapped_dom(body: &web_sys::HtmlElement) -> DomNode {
    let child_nodes = body.child_nodes();
    let mut children = Vec::with_capacity(child_nodes.length() as usize);
    for i in 0..child_nodes.length() {
        if let Some(child) = child_nodes.item(i) {
            children.push(node_to_dom_node(&child));
        }
    }

    // Wrap in <html> to match html5ever's fragment parsing output
    DomNode::Element {
        tag: "html".to_string(),
        attrs: vec![],
        children,
    }
}

/// Parse HTML using innerHTML on live document (NOT DOMParser).
/// DOMParser has scripting disabled, which changes noscript parsing behavior.
/// Using innerHTML on the live document ensures scripting is enabled.
///
/// Returns an `<html>` wrapper around the parsed content to match html5ever's
/// fragment parsing output (which also uses `<html>` as root).
fn parse_html_to_dom(html: &str) -> DomNode {
    // Get live document
    let window = web_sys::window().expect("no window");
    let document = window.document().expect("no document");
    let body = document.body().expect("no body");

    // Parse by setting innerHTML (scripting enabled, unlike DOMParser)
    body.set_inner_html(strip_doctype(html));

    body_to_html_wrapped_dom(&body)
}

/// Convert a web_sys::Node to DomNode recursively.
fn node_to_dom_node(node: &web_sys::Node) -> DomNode {
    use web_sys::Node;

    match node.node_type() {
        Node::ELEMENT_NODE => {
            let element: &web_sys::Element = node.unchecked_ref();
            let tag = element.tag_name().to_ascii_lowercase();

            // Collect attributes
            let attrs_named = element.attributes();
            let mut attrs = Vec::with_capacity(attrs_named.length() as usize);
            for i in 0..attrs_named.length() {
                if let Some(attr) = attrs_named.item(i) {
                    attrs.push(DomAttr {
                        name: attr.name(),
                        value: attr.value(),
                    });
                }
            }
            // Sort attributes by name for consistent comparison
            attrs.sort_by(|a, b| a.name.cmp(&b.name));

            // Collect children
            let child_nodes = node.child_nodes();
            let mut children = Vec::with_capacity(child_nodes.length() as usize);
            for i in 0..child_nodes.length() {
                if let Some(child) = child_nodes.item(i) {
                    children.push(node_to_dom_node(&child));
                }
            }

            DomNode::Element {
                tag,
                attrs,
                children,
            }
        }
        Node::TEXT_NODE => {
            let text = node.text_content().unwrap_or_default();
            DomNode::Text(text)
        }
        Node::COMMENT_NODE => {
            let text = node.text_content().unwrap_or_default();
            DomNode::Comment(text)
        }
        _ => {
            // For other node types, treat as empty text
            DomNode::Text(String::new())
        }
    }
}

/// Check if an attribute name is valid for the DOM setAttribute API.
/// The HTML parser is lenient, but setAttribute rejects names with =, <, >, etc.
fn is_valid_attr_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('=')
        && !name.contains('<')
        && !name.contains('>')
        && !name.contains('"')
        && !name.contains('\'')
        && !name.contains('/')
        && !name.starts_with(char::is_whitespace)
}

/// Check if a tag name is valid for the DOM createElement API.
/// The HTML parser is lenient, but createElement rejects names with <, >, etc.
fn is_valid_tag_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('<')
        && !name.contains('>')
        && !name.contains('=')
        && !name.contains('"')
        && !name.contains('\'')
        && !name.contains('/')
        && !name.contains(' ')
        && !name.contains('\t')
        && !name.contains('\n')
        && !name.contains('\r')
}

/// Check if any patch contains attributes with invalid names.
fn patches_have_invalid_attrs(patches: &[Patch]) -> bool {
    for patch in patches {
        match patch {
            Patch::InsertElement { attrs, .. } => {
                for attr in attrs {
                    let name = &attr.name.local;
                    if !is_valid_attr_name(name.as_ref()) {
                        return true;
                    }
                }
            }
            Patch::SetAttribute { name, .. } | Patch::RemoveAttribute { name, .. } => {
                if !is_valid_attr_name(name.local.as_ref()) {
                    return true;
                }
            }
            Patch::UpdateProps { changes, .. } => {
                for change in changes {
                    if let hotmeal_wasm::PropKey::Attr(ref qn) = change.name
                        && !is_valid_attr_name(qn.local.as_ref())
                    {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Check if any patch contains elements with invalid tag names.
fn patches_have_invalid_tags(patches: &[Patch]) -> bool {
    for patch in patches {
        if let Patch::InsertElement { tag, .. } = patch
            && !is_valid_tag_name(tag.as_ref())
        {
            return true;
        }
    }
    false
}

fn run_compute_and_apply_patches(
    old_html: &str,
    new_html: &str,
) -> Result<ComputeAndApplyResult, String> {
    log(&format!(
        "[browser-wasm] roundtrip: old={:?} new={:?}",
        old_html, new_html
    ));

    // Get live document (has DOCTYPE, scripting enabled)
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let body = document.body().ok_or("no body")?;

    // Parse old HTML using innerHTML (scripting enabled, unlike DOMParser)
    body.set_inner_html(strip_doctype(old_html));
    let normalized_old = body.inner_html();
    let initial_dom_tree = body_to_html_wrapped_dom(&body);

    // Parse new HTML to get normalized version
    body.set_inner_html(strip_doctype(new_html));
    let normalized_new = body.inner_html();

    log(&format!(
        "[browser-wasm] normalized: old={:?} new={:?}",
        normalized_old, normalized_new
    ));

    // Compute diff using hotmeal-wasm (on normalized HTML)
    let patches = hotmeal_wasm::diff_html_patches(
        &format!("<html><body>{}</body></html>", normalized_old),
        &format!("<html><body>{}</body></html>", normalized_new),
    )
    .map_err(|e| format!("diff_html failed: {:?}", e))?;

    log(&format!("[browser-wasm] {} patches", patches.len()));

    // Check for invalid attribute/tag names that can't be set via DOM API
    if patches_have_invalid_attrs(&patches) {
        return Err("patches contain invalid attribute names".to_string());
    }
    if patches_have_invalid_tags(&patches) {
        return Err("patches contain invalid tag names".to_string());
    }

    // Reset body to old content for patching
    body.set_inner_html(&normalized_old);

    // Apply patches one at a time, capturing state after each
    let mut slots = hotmeal_wasm::Slots::new();
    let mut patch_trace = Vec::with_capacity(patches.len());
    let mut had_error = false;

    for (i, patch) in patches.iter().enumerate() {
        log(&format!("[browser-wasm] applying patch {}: {:?}", i, patch));

        let error = if had_error {
            Some("skipped due to previous error".to_string())
        } else {
            match hotmeal_wasm::apply_patches_with_slots(std::slice::from_ref(patch), &mut slots) {
                Ok(_) => None,
                Err(e) => {
                    had_error = true;
                    Some(format!("{:?}", e))
                }
            }
        };

        let html_after = body.inner_html();
        let dom_tree = body_to_html_wrapped_dom(&body);

        patch_trace.push(PatchStep {
            index: i as u32,
            patch_debug: format!("{:?}", patch),
            html_after,
            dom_tree,
            error,
        });
    }

    let result_html = body.inner_html();
    let patch_count = patches.len() as u32;

    log(&format!(
        "[browser-wasm] result: {:?}, expected: {:?}",
        result_html, normalized_new
    ));

    // Don't fail early - let the fuzzer compare traces
    Ok(ComputeAndApplyResult {
        normalized_old,
        normalized_new,
        result_html,
        patch_count,
        initial_dom_tree,
        patch_trace,
    })
}

fn run_apply_patches(
    old_html: &str,
    patches: Vec<Patch<'static>>,
) -> Result<ApplyPatchesResult, String> {
    log(&format!(
        "[browser-wasm] run_test starting, old_html={:?}",
        old_html
    ));

    // Get live document (has DOCTYPE, scripting enabled)
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let live_body = document.body().ok_or("no body")?;

    // Parse using innerHTML on live document (scripting enabled, matches html5ever fragment parsing)
    live_body.set_inner_html(strip_doctype(old_html));

    // Capture initial tree from the live DOM, wrapped in <html> to match html5ever's fragment parsing
    let initial_dom_tree = body_to_html_wrapped_dom(&live_body);
    let normalized_old_html = live_body.inner_html();

    log(&format!(
        "[browser-wasm] normalized_old_html={:?}",
        normalized_old_html
    ));

    log(&format!(
        "[browser-wasm] applying {} patches",
        patches.len()
    ));

    let mut slots = hotmeal_wasm::Slots::new();
    let mut patch_trace = Vec::with_capacity(patches.len());
    let mut had_error = false;

    for (i, patch) in patches.iter().enumerate() {
        log(&format!("[browser-wasm] applying patch {}: {:?}", i, patch));

        let error = if had_error {
            Some("skipped due to previous error".to_string())
        } else {
            match hotmeal_wasm::apply_patches_with_slots(std::slice::from_ref(patch), &mut slots) {
                Ok(_) => None,
                Err(e) => {
                    had_error = true;
                    Some(format!("{:?}", e))
                }
            }
        };

        let body = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .body()
            .unwrap();
        let html_after = body.inner_html();
        let dom_tree = body_to_html_wrapped_dom(&body);

        patch_trace.push(PatchStep {
            index: i as u32,
            patch_debug: format!("{:?}", patch),
            html_after,
            dom_tree,
            error,
        });
    }

    let result_html = patch_trace
        .last()
        .map(|s| s.html_after.clone())
        .unwrap_or_else(|| normalized_old_html.clone());

    log("[browser-wasm] apply_patches complete");
    Ok(ApplyPatchesResult {
        result_html,
        normalized_old_html,
        initial_dom_tree,
        patch_trace,
    })
}

#[wasm_bindgen]
pub async fn connect(port: u32) -> Result<(), JsValue> {
    let url = format!("ws://127.0.0.1:{}", port);
    web_sys::console::log_1(&format!("[browser-wasm] connecting to {}", url).into());

    let transport = WsTransport::connect(&url)
        .await
        .map_err(|e| format!("connect failed: {}", e))?;

    web_sys::console::log_1(&"[browser-wasm] connected, starting handshake".into());

    let dispatcher = BrowserDispatcher::new(Handler);
    let (_handle, _incoming, driver) = initiate_framed(transport, Default::default(), dispatcher)
        .await
        .map_err(|e| format!("handshake failed: {:?}", e))?;

    web_sys::console::log_1(&"[browser-wasm] handshake complete, running driver".into());

    driver
        .run()
        .await
        .map_err(|e| format!("driver error: {:?}", e))?;

    Ok(())
}
