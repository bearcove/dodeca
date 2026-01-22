//! HTML toolkit based on facet, html5ever, and cinereus.
//!
//! hotmeal provides:
//! - **DOM types**: Strongly-typed HTML element definitions
//! - **Parsing**: Browser-compatible HTML5 parsing via html5ever with full error recovery
//! - **Diffing**: DOM patch generation for live-reloading (via hotmeal-diff)
//!
//! # Example
//!
//! ```rust,ignore
//! use hotmeal::{Html, Body, Div, FlowContent};
//!
//! // Parse HTML with browser-compatible error recovery
//! let html = hotmeal::parse("<div><p>Hello</div>");
//!
//! // Access the typed DOM
//! if let Some(body) = &html.body {
//!     for child in &body.children {
//!         if let FlowContent::Div(div) = child {
//!             println!("Found div with class: {:?}", div.attrs.class);
//!         }
//!     }
//! }
//! ```

pub mod diff;
mod dom;
mod parser;
mod serializer;

pub use dom::*;
pub use parser::{parse, parse_untyped};
pub use serializer::{SerializeOptions, to_string, to_string_pretty, to_string_with_options};
