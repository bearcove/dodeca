//! Protocol definitions for dodeca dialoguer cell
//!
//! This cell provides interactive terminal prompts (select, confirm, input).

use facet::Facet;

/// Result of a select prompt
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum SelectResult {
    /// User selected an item (0-indexed)
    Selected { index: usize },
    /// User cancelled (e.g., pressed Escape)
    Cancelled,
}

/// Result of a confirm prompt
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum ConfirmResult {
    /// User confirmed
    Yes,
    /// User declined
    No,
    /// User cancelled
    Cancelled,
}

/// Interactive prompt service
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait Dialoguer {
    /// Show a select prompt with a list of items.
    /// Returns the index of the selected item, or None if cancelled.
    async fn select(&self, prompt: String, items: Vec<String>) -> SelectResult;

    /// Show a confirmation prompt.
    async fn confirm(&self, prompt: String, default: bool) -> ConfirmResult;
}
