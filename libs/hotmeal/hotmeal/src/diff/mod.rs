//! HTML diffing with DOM patch generation.
//!
//! This module uses cinereus (GumTree/Chawathe) to compute tree diffs and translates
//! them into DOM patches that can be applied to update an HTML document incrementally.
//!
//! # Example
//!
//! ```rust
//! use hotmeal::arena_dom;
//! use hotmeal::diff::diff_arena_documents;
//!
//! let mut old = arena_dom::parse("<div><p>Hello</p></div>");
//! let new = arena_dom::parse("<div><p>World</p></div>");
//!
//! // Get the patches needed to transform old into new
//! let patches = diff_arena_documents(&old, &new).expect("diffing should succeed");
//!
//! // Apply patches to the old document
//! old.apply_patches(&patches).expect("patches should apply");
//!
//! // Verify the result
//! assert!(old.to_html().contains("World"));
//! ```

mod apply;
mod tree;

pub use apply::{Content, Element, apply_patches, parse_html};
pub use tree::{diff_arena_documents, diff_elements};

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
    /// A comment node
    Comment(String),
}

/// A property in the final state within an UpdateProps operation.
/// The vec position determines the final ordering.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct PropChange {
    /// The property name (field name)
    pub name: String,
    /// The value: None means "keep existing value", Some means "update to this value".
    /// Properties not in the list are implicitly removed.
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

    /// Insert a comment node at a position.
    /// The `at` NodeRef includes the position as the last path segment.
    InsertComment {
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

/// Diff two arena documents and return DOM patches.
///
/// This is the primary diffing API for arena_dom documents.
pub fn diff(
    old: &crate::arena_dom::Document,
    new: &crate::arena_dom::Document,
) -> Result<Vec<Patch>, String> {
    diff_arena_documents(old, new)
}

/// Diff two HTML strings and return DOM patches.
///
/// Parses both HTML strings and diffs them.
pub fn diff_html(old_html: &str, new_html: &str) -> Result<Vec<Patch>, String> {
    let old_doc = crate::arena_dom::parse(old_html);
    let new_doc = crate::arena_dom::parse(new_html);
    diff_arena_documents(&old_doc, &new_doc)
}

/// Legacy: Diff two untyped_dom Element trees and return DOM patches.
///
/// For backwards compatibility. New code should use `diff()` with arena_dom.
pub fn diff_untyped(
    old: &crate::untyped_dom::Element,
    new: &crate::untyped_dom::Element,
) -> Result<Vec<Patch>, String> {
    // Convert to diff module types
    let old_elem: Element = old.into();
    let new_elem: Element = new.into();

    diff_elements(&old_elem, &new_elem)
}
