//! # Cinereus
//!
//! GumTree-style tree diffing with Chawathe edit script generation.
//!
//! Named after *Phascolarctos cinereus* (the koala), which lives in gum trees.
//!
//! ## Algorithm Overview
//!
//! Cinereus implements a tree diff algorithm based on:
//! - **GumTree** (Falleri et al., ASE 2014) for node matching
//! - **Chawathe algorithm** (1996) for edit script generation
//!
//! The algorithm works in phases:
//!
//! 1. **Top-down matching**: Match identical subtrees by hash (Merkle-tree style)
//! 2. **Bottom-up matching**: Match remaining nodes by structural similarity (Dice coefficient)
//! 3. **Edit script generation**: Produce INSERT, DELETE, UPDATE, MOVE operations
//!
//! ## Usage
//!
//! ```ignore
//! use cinereus::{Tree, tree_diff};
//!
//! // Build trees from your data structure
//! let tree_a = Tree::build(/* ... */);
//! let tree_b = Tree::build(/* ... */);
//!
//! // Compute the diff
//! let edit_script = tree_diff(&tree_a, &tree_b);
//!
//! for op in edit_script {
//!     println!("{:?}", op);
//! }
//! ```

#![warn(missing_docs)]
#![warn(clippy::std_instead_of_core)]

pub use indextree;

mod tracing_macros;

mod chawathe;
/// GumTree matching algorithm
pub mod matching;
/// Tree representation with properties support
pub mod tree;

pub use chawathe::*;
pub use matching::*;
pub use tree::{
    DiffTree, NoKey, NoProps, NoVal, NodeData, NodeHash, PropValue, Properties,
    PropertyInFinalState, SimpleTypes, Tree, TreeTypes,
};
pub use tree::{get_position_stats, reset_position_counters};

/// Compute a diff between two trees.
///
/// This is the main entry point for tree diffing. It:
/// 1. Computes a matching between nodes using GumTree's two-phase algorithm
/// 2. Generates an edit script using Chawathe's algorithm
///
/// # Example
///
/// ```
/// use cinereus::{Tree, NodeData, diff_trees, MatchingConfig, SimpleTypes};
///
/// type TestTypes = SimpleTypes<&'static str>;
///
/// let mut tree_a: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
/// tree_a.add_child(tree_a.root, NodeData::simple_u64(1, "leaf"));
///
/// let mut tree_b: Tree<TestTypes> = Tree::new(NodeData::simple_u64(100, "root"));
/// tree_b.add_child(tree_b.root, NodeData::simple_u64(2, "leaf"));
///
/// let ops = diff_trees(&tree_a, &tree_b, &MatchingConfig::default());
/// // ops contains the edit operations to transform tree_a into tree_b
/// ```
pub fn diff_trees<T: DiffTree>(
    tree_a: &T,
    tree_b: &T,
    config: &MatchingConfig,
) -> Vec<EditOp<T::Types>> {
    let (ops, _matching) = diff_trees_with_matching(tree_a, tree_b, config);
    ops
}

/// Like [`diff_trees`], but also returns the node matching.
///
/// This is useful when you need to translate NodeId-based operations
/// into path-based operations, as you need to track which nodes in
/// tree_a correspond to nodes in tree_b.
pub fn diff_trees_with_matching<T: DiffTree>(
    tree_a: &T,
    tree_b: &T,
    config: &MatchingConfig,
) -> (Vec<EditOp<T::Types>>, Matching) {
    let matching = compute_matching(tree_a, tree_b, config);
    let ops = generate_edit_script(tree_a, tree_b, &matching);
    (ops, matching)
}
