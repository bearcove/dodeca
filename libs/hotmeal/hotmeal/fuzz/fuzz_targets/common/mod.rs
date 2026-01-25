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

/// Check if an HTML string should be skipped due to known browser bugs.
/// Returns true if the string is valid for browser parity testing.
fn is_valid_for_browser_parity(html: &str) -> bool {
    // Skip empty inputs
    if html.is_empty() {
        return false;
    }

    // Skip inputs with null bytes
    if html.contains('\0') {
        return false;
    }

    // Skip inputs containing CR (\r) - Chrome has numerous bugs with CR normalization
    // in various contexts (LF-CR, CR-CR, /\r, space-CR, etc.). Per INFRA spec, CR should
    // be normalized to LF, but Chrome handles it incorrectly in many edge cases.
    // Firefox and Safari are spec-compliant, html5ever is spec-compliant.
    // See: https://infra.spec.whatwg.org/#normalize-newlines
    if html.contains('\r') {
        return false;
    }

    // Skip inputs containing C0 control characters (0x01-0x1F except tab, LF, FF)
    // There are complex structural differences between html5ever and Chrome involving
    // control characters in edge cases. Needs further investigation.
    if html
        .bytes()
        .any(|b| matches!(b, 0x01..=0x08 | 0x0B | 0x0E..=0x1F))
    {
        return false;
    }

    true
}

/// Prepare a single HTML string for browser parity testing.
/// Returns None if the input should be skipped.
pub fn prepare_single_html_input(data: &[u8]) -> Option<String> {
    let html = std::str::from_utf8(data).ok()?;

    if !is_valid_for_browser_parity(html) {
        return None;
    }

    // Wrap in full HTML document with DOCTYPE for no-quirks mode
    Some(format!("<!DOCTYPE html><html><body>{}</body></html>", html))
}

/// Splits the input data into two sanitized HTML documents wrapped in a standard template.
pub fn prepare_html_inputs(data: &[u8]) -> Option<(String, String)> {
    // Split at 0xFF delimiter
    let (html_a, html_b) = split_input(data)?;

    if !is_valid_for_browser_parity(&html_a) || !is_valid_for_browser_parity(&html_b) {
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
