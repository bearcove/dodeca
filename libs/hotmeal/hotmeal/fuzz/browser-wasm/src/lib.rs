use browser_proto::{
    BrowserFuzzer, BrowserFuzzerDispatcher, DomAttr, DomNode, OwnedPatches, Patch, PatchStep,
    RoundtripResult, TestPatchResult,
};
use roam::Context;
use roam_session::initiate_framed;
use roam_websocket::WsTransport;
use wasm_bindgen::prelude::*;

#[derive(Clone)]
struct Handler;

impl BrowserFuzzer for Handler {
    async fn test_patch(
        &self,
        _cx: &Context,
        old_html: String,
        patches: OwnedPatches,
    ) -> Result<TestPatchResult, String> {
        run_test(&old_html, patches.0)
    }

    async fn test_roundtrip(
        &self,
        _cx: &Context,
        old_html: String,
        new_html: String,
    ) -> Result<RoundtripResult, String> {
        run_roundtrip(&old_html, &new_html)
    }

    async fn parse_to_dom(&self, _cx: &Context, html: String) -> DomNode {
        parse_html_to_dom(&html)
    }
}

fn log(msg: &str) {
    web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(msg));
}

/// Parse HTML using the browser's DOMParser and convert to DomNode tree.
fn parse_html_to_dom(html: &str) -> DomNode {
    use web_sys::{DomParser, SupportedType};

    let parser = DomParser::new().expect("DOMParser::new failed");

    // Wrap in full HTML document with DOCTYPE for no-quirks mode
    let doc_html = format!("<!DOCTYPE html><html><body>{}</body></html>", html);

    let doc = parser
        .parse_from_string(&doc_html, SupportedType::TextHtml)
        .expect("parse_from_string failed");

    let body = doc.body().expect("no body");

    // Convert body's children to DomNode tree
    node_to_dom_node(&body.into())
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
                    if let hotmeal_wasm::PropKey::Attr(ref qn) = change.name {
                        if !is_valid_attr_name(qn.local.as_ref()) {
                            return true;
                        }
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
        if let Patch::InsertElement { tag, .. } = patch {
            if !is_valid_tag_name(tag.as_ref()) {
                return true;
            }
        }
    }
    false
}

fn run_roundtrip(old_html: &str, new_html: &str) -> Result<RoundtripResult, String> {
    use web_sys::{DomParser, SupportedType};

    log(&format!(
        "[browser-wasm] roundtrip: old={:?} new={:?}",
        old_html, new_html
    ));

    let parser = DomParser::new().map_err(|e| format!("DOMParser::new failed: {:?}", e))?;

    // Wrap inputs in full HTML documents for parsing
    // IMPORTANT: Include DOCTYPE to ensure no-quirks mode
    let old_doc_html = format!("<!DOCTYPE html><html><body>{}</body></html>", old_html);
    let new_doc_html = format!("<!DOCTYPE html><html><body>{}</body></html>", new_html);

    let old_doc = parser
        .parse_from_string(&old_doc_html, SupportedType::TextHtml)
        .map_err(|e| format!("parse old_html failed: {:?}", e))?;

    let new_doc = parser
        .parse_from_string(&new_doc_html, SupportedType::TextHtml)
        .map_err(|e| format!("parse new_html failed: {:?}", e))?;

    let old_body = old_doc.body().ok_or("old_doc has no body")?;
    let new_body = new_doc.body().ok_or("new_doc has no body")?;

    let normalized_old = old_body.inner_html();
    let normalized_new = new_body.inner_html();

    // Capture initial DOM tree
    let initial_dom_tree = node_to_dom_node(&old_body.into());

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

    // Set the document body to the old content so we can patch it
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let body = document.body().ok_or("no body")?;
    body.set_inner_html(&normalized_old);

    // Apply patches one at a time, capturing state after each
    let mut slots = hotmeal_wasm::Slots::new();
    let mut patch_trace = Vec::with_capacity(patches.len());

    for (i, patch) in patches.iter().enumerate() {
        log(&format!("[browser-wasm] applying patch {}: {:?}", i, patch));
        hotmeal_wasm::apply_patches_with_slots(&[patch.clone()], &mut slots)
            .map_err(|e| format!("patch {} failed: {:?}", i, e))?;

        let html_after = body.inner_html();
        let dom_tree = node_to_dom_node(&body.clone().into());

        patch_trace.push(PatchStep {
            index: i as u32,
            patch_debug: format!("{:?}", patch),
            html_after,
            dom_tree,
        });
    }

    let result_html = body.inner_html();
    let patch_count = patches.len() as u32;

    log(&format!(
        "[browser-wasm] result: {:?}, expected: {:?}",
        result_html, normalized_new
    ));

    if result_html != normalized_new {
        return Err("result doesn't match expected".to_string());
    }

    Ok(RoundtripResult {
        normalized_old,
        normalized_new,
        result_html,
        patch_count,
        initial_dom_tree,
        patch_trace,
    })
}

fn run_test(old_html: &str, patches: Vec<Patch<'static>>) -> Result<TestPatchResult, String> {
    use web_sys::{DomParser, SupportedType};

    log(&format!(
        "[browser-wasm] run_test starting, old_html={:?}",
        old_html
    ));

    // Parse the full HTML document using DOMParser (same as html5ever)
    // This gives us the initial_dom_tree for comparison
    let parser = DomParser::new().map_err(|e| format!("DOMParser::new failed: {:?}", e))?;
    let parsed_doc = parser
        .parse_from_string(old_html, SupportedType::TextHtml)
        .map_err(|e| format!("parse_from_string failed: {:?}", e))?;
    let parsed_body = parsed_doc.body().ok_or("parsed doc has no body")?;
    let initial_dom_tree = node_to_dom_node(&parsed_body.clone().into());
    let normalized_old_html = parsed_body.inner_html();

    log(&format!(
        "[browser-wasm] normalized_old_html={:?}",
        normalized_old_html
    ));

    // Set the actual document body to the normalized content for patching
    hotmeal_wasm::set_body_inner_html(&normalized_old_html)
        .map_err(|e| format!("set_body_inner_html failed: {:?}", e))?;

    log(&format!(
        "[browser-wasm] applying {} patches",
        patches.len()
    ));

    let mut slots = hotmeal_wasm::Slots::new();
    let mut patch_trace = Vec::with_capacity(patches.len());

    for (i, patch) in patches.iter().enumerate() {
        log(&format!("[browser-wasm] applying patch {}: {:?}", i, patch));
        hotmeal_wasm::apply_patches_with_slots(&[patch.clone()], &mut slots)
            .map_err(|e| format!("patch {} failed: {:?}", i, e))?;

        let html_after = hotmeal_wasm::get_body_inner_html()
            .map_err(|e| format!("get_body_inner_html after patch {} failed: {:?}", i, e))?;

        let body = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .body()
            .unwrap();
        let dom_tree = node_to_dom_node(&body.into());

        patch_trace.push(PatchStep {
            index: i as u32,
            patch_debug: format!("{:?}", patch),
            html_after,
            dom_tree,
        });
    }

    let result_html = patch_trace
        .last()
        .map(|s| s.html_after.clone())
        .unwrap_or_else(|| normalized_old_html.clone());

    log("[browser-wasm] run_test complete");
    Ok(TestPatchResult {
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

    let dispatcher = BrowserFuzzerDispatcher::new(Handler);
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
