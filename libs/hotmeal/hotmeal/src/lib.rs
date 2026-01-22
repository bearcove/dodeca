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
//! use hotmeal::arena_dom;
//!
//! // Parse a full document - uses zero-copy via StrTendril
//! let doc = arena_dom::parse("<!DOCTYPE html><html><body><p>Hello!</p></body></html>");
//! assert_eq!(doc.doctype.as_ref().map(|s| s.as_ref()), Some("html"));
//!
//! if let Some(body_id) = doc.body() {
//!     for child_id in body_id.children(&doc.arena) {
//!         let node = doc.get(child_id);
//!         if let arena_dom::NodeKind::Element(elem) = &node.kind {
//!             println!("Found {} element", elem.tag);
//!         }
//!     }
//! }
//!
//! // Serialize back to HTML
//! let html = doc.to_html();
//! ```

mod tracing_macros;

pub mod arena_dom;
pub mod diff;
pub mod parser;
pub mod serialize;
pub mod untyped_dom;

// Re-export arena_dom types and functions as the primary API
pub use arena_dom::{Document, ElementData, Namespace, NodeData, NodeKind, parse};

// Legacy: keep untyped_dom for backwards compatibility but don't promote it
// Users can still access via hotmeal::untyped_dom::* if needed

// Re-export serialization (will be updated to work with arena_dom)
pub use serialize::{SerializeOptions, serialize_document, serialize_element, serialize_fragment};
