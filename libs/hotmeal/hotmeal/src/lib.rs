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
//! use hotmeal::{parse, NodeKind};
//!
//! // Parse a full document - uses zero-copy via Stem
//! let doc = parse("<!DOCTYPE html><html><body><p>Hello!</p></body></html>");
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

use tendril::{NonAtomic, Tendril, fmt::UTF8};

mod diff;
mod dom;
mod tracing_macros;

// Re-export arena_dom types and functions as the primary API
pub use cinereus::indextree::NodeId;
pub use diff::{
    AttrPair, DiffError, HtmlNodeKind, HtmlProps, HtmlTreeTypes, InsertContent, NodePath, NodeRef,
    Patch, PropChange, PropKey, diff, diff_html,
};
pub use dom::{Document, ElementData, Namespace, NodeData, NodeKind, parse};

/// Zero-copy string tendril
pub type Stem = Tendril<UTF8, NonAtomic>;
