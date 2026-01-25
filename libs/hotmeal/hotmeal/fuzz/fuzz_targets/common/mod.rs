#![allow(dead_code)]
#![allow(unused_imports)]

use std::sync::Once;

use hotmeal::Patch;

// Re-export thrall's browser control functions
pub use thrall::{ApplyPatchesResult, ComputeAndApplyResult, DomNode, OwnedPatches};
pub use thrall::{apply_patches, compute_and_apply_patches, parse_to_dom};

static THRALL_INIT: Once = Once::new();

/// Initialize thrall with quiet config for fuzzing (call once at startup).
pub fn init_thrall_quiet() {
    THRALL_INIT.call_once(|| {
        thrall::configure(thrall::ThrallConfig::quiet());
    });
}

mod dom_generator;
pub use dom_generator::*;

mod dom_node;
pub use dom_node::*;

mod patch_trace;
pub use patch_trace::*;

/// Split raw bytes at 0xFF delimiter into two HTML strings.
fn split_input(data: &[u8]) -> Option<(String, String)> {
    let pos = data.iter().position(|&b| b == 0xFF)?;
    let html_a = std::str::from_utf8(&data[..pos]).ok()?.to_owned();
    let html_b = std::str::from_utf8(&data[pos + 1..]).ok()?.to_owned();
    Some((html_a, html_b))
}

/// Splits the input data into two sanitized HTML documents wrapped in a standard template.
pub fn prepare_html_inputs(data: &[u8]) -> Option<(String, String)> {
    // Split at 0xFF delimiter
    let (html_a, html_b) = split_input(data)?;

    // Skip empty inputs
    if html_a.is_empty() || html_b.is_empty() {
        return None;
    }

    // Skip inputs with null bytes
    if html_a.contains('\0') || html_b.contains('\0') {
        return None;
    }

    // Wrap in full HTML document with DOCTYPE for no-quirks mode
    let full_a = format!("<!DOCTYPE html><html><body>{}</body></html>", html_a);
    let full_b = format!("<!DOCTYPE html><html><body>{}</body></html>", html_b);

    Some((full_a, full_b))
}

/// Check if an attribute name is valid for the DOM setAttribute API.
fn is_valid_attr_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('=')
        && !name.contains('<')
        && !name.contains('>')
        && !name.contains('"')
        && !name.contains('\'')
        && !name.contains('/')
        && !name.starts_with(char::is_whitespace)
        && !name.contains(char::is_control)
}

/// Check if a tag name is valid for the DOM createElement API.
fn is_valid_tag_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('<')
        && !name.contains('>')
        && !name.contains('=')
        && !name.contains('"')
        && !name.contains('\'')
        && !name.contains('/')
        && !name.contains(char::is_whitespace)
        && !name.contains(char::is_control)
}

/// Check if any patch contains attributes with invalid names.
fn patches_have_invalid_attrs(patches: &[Patch]) -> bool {
    for patch in patches {
        match patch {
            Patch::InsertElement {
                attrs, children, ..
            } => {
                for attr in attrs {
                    if !is_valid_attr_name(&attr.name.local) {
                        return true;
                    }
                }
                if insert_contents_have_invalid_attrs(children) {
                    return true;
                }
            }
            Patch::SetAttribute { name, .. } | Patch::RemoveAttribute { name, .. } => {
                if !is_valid_attr_name(&name.local) {
                    return true;
                }
            }
            Patch::UpdateProps { changes, .. } => {
                for change in changes {
                    if let hotmeal::PropKey::Attr(ref qn) = change.name {
                        if !is_valid_attr_name(&qn.local) {
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

fn insert_contents_have_invalid_attrs(contents: &[hotmeal::InsertContent]) -> bool {
    for content in contents {
        if let hotmeal::InsertContent::Element {
            attrs, children, ..
        } = content
        {
            for attr in attrs {
                if !is_valid_attr_name(&attr.name.local) {
                    return true;
                }
            }
            if insert_contents_have_invalid_attrs(children) {
                return true;
            }
        }
    }
    false
}

/// Check if any patch contains tags with invalid names.
fn patches_have_invalid_tags(patches: &[Patch]) -> bool {
    for patch in patches {
        if let Patch::InsertElement { tag, children, .. } = patch {
            if !is_valid_tag_name(tag) {
                return true;
            }
            if insert_contents_have_invalid_tags(children) {
                return true;
            }
        }
    }
    false
}

fn insert_contents_have_invalid_tags(contents: &[hotmeal::InsertContent]) -> bool {
    for content in contents {
        if let hotmeal::InsertContent::Element { tag, children, .. } = content {
            if !is_valid_tag_name(tag) {
                return true;
            }
            if insert_contents_have_invalid_tags(children) {
                return true;
            }
        }
    }
    false
}

/// Check if patches are valid for DOM APIs (no invalid attr/tag names).
pub fn patches_are_valid_for_dom(patches: &[Patch]) -> bool {
    !patches_have_invalid_attrs(patches) && !patches_have_invalid_tags(patches)
}
