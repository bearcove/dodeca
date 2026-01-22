//! HTML diffing with DOM patch generation.
//!
//! This module translates facet-diff EditOps (from GumTree/Chawathe) into DOM Patches
//! that can be applied to update an HTML document incrementally.

#[macro_use]
mod tracing_macros;

mod apply;
mod translate;

pub use apply::{Content, Element, apply_patches};
pub use translate::{
    InsertContent, NodePath, NodeRef, Patch, PathTarget, PropChange, TranslateError, diff_html,
    translate_to_patches,
};
