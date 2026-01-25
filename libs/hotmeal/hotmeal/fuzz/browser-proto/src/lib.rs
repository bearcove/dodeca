#![deny(unsafe_code)]

use std::fmt;

use facet::Facet;
use roam::service;

// Re-export Patch from hotmeal for use in protocol
pub use hotmeal::Patch;

/// A DOM node in JSON-like format for comparing html5ever vs browser parsing.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[facet(recursive_type)]
#[repr(u8)]
pub enum DomNode {
    /// An element node with tag name, attributes, and children.
    Element {
        tag: String,
        attrs: Vec<DomAttr>,
        children: Vec<DomNode>,
    },
    /// A text node.
    Text(String),
    /// A comment node.
    Comment(String),
}

/// An attribute on a DOM element.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DomAttr {
    pub name: String,
    pub value: String,
}

impl fmt::Display for DomNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn write_indented(
            node: &DomNode,
            f: &mut fmt::Formatter<'_>,
            indent: usize,
        ) -> fmt::Result {
            let prefix = "  ".repeat(indent);
            match node {
                DomNode::Element {
                    tag,
                    attrs,
                    children,
                } => {
                    write!(f, "{}<{}", prefix, tag)?;
                    for attr in attrs {
                        write!(f, " {}={:?}", attr.name, attr.value)?;
                    }
                    writeln!(f, ">")?;
                    for child in children {
                        write_indented(child, f, indent + 1)?;
                    }
                    writeln!(f, "{}</{}>", prefix, tag)
                }
                DomNode::Text(text) => writeln!(f, "{}TEXT: {:?}", prefix, text),
                DomNode::Comment(text) => writeln!(f, "{}COMMENT: {:?}", prefix, text),
            }
        }
        write_indented(self, f, 0)
    }
}

/// Wrapper for owned patches that can be sent over roam.
#[derive(Debug, Clone, Facet)]
pub struct OwnedPatches(pub Vec<Patch<'static>>);

/// Browser fuzzer service - implemented by the browser, called by the fuzzer.
///
/// The browser receives old HTML and patches, applies them to the DOM,
/// and returns the resulting HTML.
#[service]
pub trait Browser {
    /// Apply patches to HTML in the browser.
    ///
    /// The browser will:
    /// 1. Set document.body.innerHTML to `old_html`
    /// 2. Apply the patches
    /// 3. Return the resulting document.body.innerHTML
    async fn test_patch(
        &self,
        old_html: String,
        patches: OwnedPatches,
    ) -> Result<TestPatchResult, String>;

    /// Full roundtrip in browser: parse both HTMLs with DOMParser, diff, apply, compare.
    ///
    /// The browser will:
    /// 1. Parse `old_html` with DOMParser, serialize back (normalized_old)
    /// 2. Parse `new_html` with DOMParser, serialize back (normalized_new)
    /// 3. Compute diff using hotmeal-wasm
    /// 4. Apply patches to old DOM
    /// 5. Compare result with normalized_new
    async fn test_roundtrip(
        &self,
        old_html: String,
        new_html: String,
    ) -> Result<RoundtripResult, String>;

    /// Parse HTML in the browser and return the DOM tree as JSON.
    /// Used for comparing html5ever parsing with browser parsing.
    async fn parse_to_dom(&self, html: String) -> DomNode;
}

/// Result of a full roundtrip test in the browser.
#[derive(Debug, Clone, Facet)]
pub struct RoundtripResult {
    /// The old HTML after browser normalization.
    pub normalized_old: String,
    /// The new HTML after browser normalization (expected result).
    pub normalized_new: String,
    /// The actual result after applying patches.
    pub result_html: String,
    /// Number of patches applied.
    pub patch_count: u32,
    /// DOM tree of old HTML before any patches.
    pub initial_dom_tree: DomNode,
    /// Step-by-step trace of patch application.
    pub patch_trace: Vec<PatchStep>,
}

/// Result of applying patches in the browser.
#[derive(Debug, Clone, Facet)]
pub struct TestPatchResult {
    /// The resulting HTML after applying patches.
    pub result_html: String,
    /// Normalized old HTML after browser parsing (innerHTML readback).
    pub normalized_old_html: String,
    /// DOM tree of old HTML before any patches.
    pub initial_dom_tree: DomNode,
    /// Step-by-step trace of patch application.
    /// Each entry is the innerHTML after applying that patch.
    pub patch_trace: Vec<PatchStep>,
}

/// One step in the patch application trace.
#[derive(Debug, Clone, Facet)]
pub struct PatchStep {
    /// Index of the patch that was applied.
    pub index: u32,
    /// Debug representation of the patch being applied.
    pub patch_debug: String,
    /// The innerHTML after applying this patch (or at failure).
    pub html_after: String,
    /// Full DOM tree after applying this patch (or at failure).
    pub dom_tree: DomNode,
    /// Error message if this patch failed, None if successful.
    pub error: Option<String>,
}
