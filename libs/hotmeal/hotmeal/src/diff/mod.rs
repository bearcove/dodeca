//! HTML diffing with DOM patch generation.
//!
//! This module uses cinereus (GumTree/Chawathe) to compute tree diffs and translates
//! them into DOM patches that can be applied to update an HTML document incrementally.
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

mod apply;
mod tree;

pub use apply::{Content, Element, apply_patches, parse_html};
pub use tree::diff_elements;

// Re-export patch types
pub use apply::Content as ApplyContent;

/// A path to a node in the DOM tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
#[facet(transparent)]
pub struct NodePath(pub Vec<usize>);

impl std::fmt::Display for NodePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, idx) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{}", idx)?;
        }
        Ok(())
    }
}

/// Reference to a node - either by path or by slot number.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum NodeRef {
    /// Node at a path in the DOM
    Path(NodePath),
    /// Node in a slot (previously detached).
    Slot(u32, Option<NodePath>),
}

/// Content that can be inserted as part of a new subtree.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum InsertContent {
    /// An element with its tag, attributes, and nested children
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
        children: Vec<InsertContent>,
    },
    /// A text node
    Text(String),
}

/// A single property change within an UpdateProps operation.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct PropChange {
    /// The property name (field name)
    pub name: String,
    /// The new value (None if property is being removed)
    pub value: Option<String>,
}

/// Operations to transform the DOM.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum Patch {
    /// Insert an element at a position.
    /// The `at` NodeRef includes the position as the last path segment.
    /// For Path(NodePath(\[a, b, c\])), insert at position c within parent at path \[a, b\].
    /// For Slot(n, Some(NodePath(\[a, b\]))), insert at position b within element at path \[a\] in slot n.
    InsertElement {
        at: NodeRef,
        tag: String,
        attrs: Vec<(String, String)>,
        children: Vec<InsertContent>,
        detach_to_slot: Option<u32>,
    },

    /// Insert a text node at a position.
    /// The `at` NodeRef includes the position as the last path segment.
    InsertText {
        at: NodeRef,
        text: String,
        detach_to_slot: Option<u32>,
    },

    /// Remove a node
    Remove { node: NodeRef },

    /// Update text content of a text node at path.
    SetText { path: NodePath, text: String },

    /// Set attribute on element at path
    SetAttribute {
        path: NodePath,
        name: String,
        value: String,
    },

    /// Remove attribute from element at path
    RemoveAttribute { path: NodePath, name: String },

    /// Move a node from one location to another.
    Move {
        from: NodeRef,
        to: NodeRef,
        detach_to_slot: Option<u32>,
    },

    /// Update multiple properties on an element.
    UpdateProps {
        path: NodePath,
        changes: Vec<PropChange>,
    },
}

/// Diff two HTML documents and return DOM patches.
pub fn diff_html(old_html: &str, new_html: &str) -> Result<Vec<Patch>, String> {
    // parse_untyped returns diff::Element directly
    let old_elem = crate::parse_untyped(old_html);
    let new_elem = crate::parse_untyped(new_html);

    diff_elements(&old_elem, &new_elem)
}

/// Diff two Element trees and return DOM patches.
pub fn diff(
    old: &crate::untyped_dom::Element,
    new: &crate::untyped_dom::Element,
) -> Result<Vec<Patch>, String> {
    // Convert to diff module types
    let old_elem: Element = old.into();
    let new_elem: Element = new.into();

    diff_elements(&old_elem, &new_elem)
}
