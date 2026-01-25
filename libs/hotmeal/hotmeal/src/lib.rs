//! HTML toolkit based on html5ever and cinereus.
//!
//! hotmeal provides:
//! - **Arena-based DOM**: Efficient arena-allocated tree with zero-copy parsing
//! - **Parsing**: Browser-compatible HTML5 parsing via html5ever with full error recovery
//! - **Serialization**: HTML5-correct serialization with proper escaping
//! - **Diffing**: DOM patch generation for live-reloading
//!
//! # Example
//!
//! ```rust
//! use hotmeal::{parse, NodeKind, StrTendril};
//!
//! let input = StrTendril::from("<!DOCTYPE html><html><body><p>Hello!</p></body></html>");
//! let doc = parse(&input);
//! assert_eq!(doc.doctype.as_ref().map(|s| s.as_ref()), Some("html"));
//!
//! if let Some(body_id) = doc.body() {
//!     for child_id in body_id.children(&doc.arena) {
//!         let node = doc.get(child_id);
//!         if let NodeKind::Element(elem) = &node.kind {
//!             println!("Found {} element", elem.tag);
//!         }
//!     }
//! }
//!
//! // Serialize back to HTML
//! let html = doc.to_html();
//! ```

mod diff;
mod dom;
#[cfg(any(test, feature = "tracing"))]
mod shadow_tree_dump;
mod stem;
mod tracing_macros;

pub use cinereus::indextree::NodeId;
pub use diff::{
    AttrPair, DiffError, HtmlNodeKind, HtmlProps, HtmlTreeTypes, InsertContent, NodePath, NodeRef,
    Patch, PropChange, PropKey, diff, diff_html,
};
pub use dom::{Document, ElementData, Namespace, NodeData, NodeKind, parse, parse_body_fragment};
pub use html5ever::{LocalName, QualName, local_name, namespace_url, ns};
pub use stem::Stem;
pub use tendril::StrTendril;

const _: () = {
    const fn assert_send<T: Send>() {}
    const fn assert_sync<T: Sync>() {}
    assert_send::<Document>();
    assert_sync::<Document>();
};

// Assert that Document<'a> is covariant in 'a
// (can shorten lifetime: Document<'long> -> Document<'short>)
fn _assert_document_covariant<'long, 'short>(x: Document<'long>) -> Document<'short>
where
    'long: 'short,
{
    x
}
