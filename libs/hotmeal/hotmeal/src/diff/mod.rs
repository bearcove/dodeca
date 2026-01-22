//! HTML diffing with DOM patch generation.
//!
//! This module translates facet-diff EditOps (from GumTree/Chawathe) into DOM Patches
//! that can be applied to update an HTML document incrementally.
//!
//! # Example
//!
//! ```rust
//! use hotmeal::{parse_body, diff::{diff, apply_patches}};
//!
//! let mut old = parse_body("<div><p>Hello</p></div>");
//! let new = parse_body("<div><p>World</p></div>");
//!
//! // Get the patches needed to transform old into new
//! let patches = hotmeal::diff::diff(&old, &new).expect("diffing should succeed");
//!
//! // Apply patches to the old document (need to convert to diff types)
//! let mut old_diff: hotmeal::diff::Element = (&old).into();
//! hotmeal::diff::apply_patches(&mut old_diff, &patches).expect("patches should apply");
//! ```

#[macro_use]
mod tracing_macros;

mod apply;
mod translate;

pub use apply::{Content, Element, apply_patches, parse_html};
pub use translate::{
    InsertContent, NodePath, NodeRef, Patch, PathTarget, PropChange, TranslateError, diff,
    diff_html, translate_to_patches,
};
