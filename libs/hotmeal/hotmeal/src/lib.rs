//! HTML toolkit based on facet, html5ever, and cinereus.
//!
//! hotmeal provides:
//! - **Untyped DOM**: Simple Element/Node tree for flexible parsing
//! - **Parsing**: Browser-compatible HTML5 parsing via html5ever with full error recovery
//! - **Serialization**: HTML5-correct serialization with proper escaping
//! - **Diffing**: DOM patch generation for live-reloading
//!
//! # Example
//!
//! ```rust
//! use hotmeal::{parse_document, parse_body};
//! use hotmeal::untyped_dom::{Element, Node};
//!
//! // Parse a full document
//! let doc = parse_document("<!DOCTYPE html><html><body><p>Hello!</p></body></html>");
//! assert_eq!(doc.doctype, Some("html".to_string()));
//!
//! if let Some(body) = doc.body() {
//!     for child in &body.children {
//!         if let Node::Element(elem) = child {
//!             println!("Found {} element", elem.tag);
//!         }
//!     }
//! }
//!
//! // Serialize back to HTML
//! let html = doc.to_html();
//!
//! // Or parse just a body fragment
//! let body = parse_body("<p>Hello!</p><p>World!</p>");
//! assert_eq!(body.tag, "body");
//! ```

mod tracing_macros;

pub mod diff;
mod parser;
pub mod serialize;
pub mod untyped_dom;

// Re-export parsing functions
pub use parser::{parse_body, parse_document, parse_untyped};

// Re-export serialization
pub use serialize::{SerializeOptions, serialize_document, serialize_element, serialize_fragment};

// Re-export untyped DOM types at crate root for convenience
pub use untyped_dom::{Document, Element, Namespace, Node};
