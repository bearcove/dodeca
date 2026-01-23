//! Dodeca HTML diff cell (cell-html-diff)
//!
//! This cell handles HTML DOM diffing for live reload using facet-format-html
//! for parsing and facet-diff for computing structural differences.

use dodeca_cell_runtime::run_cell;

use cell_html_diff_proto::{DiffError, DiffInput, DiffOutcome, HtmlDiffer, HtmlDifferDispatcher};

use dodeca_protocol::facet_postcard;
// Re-export protocol types
pub use dodeca_protocol::{NodePath, Patch};

// ============================================================================
// HTML Differ Implementation
// ============================================================================

/// HTML differ implementation using facet-format-html and facet-diff.
#[derive(Clone)]
pub struct HtmlDifferImpl;

impl HtmlDiffer for HtmlDifferImpl {
    async fn diff_html(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        input: DiffInput,
    ) -> Result<DiffOutcome, DiffError> {
        tracing::debug!(
            old_len = input.old_html.len(),
            new_len = input.new_html.len(),
            "diffing HTML"
        );

        let patches = hotmeal::diff_html(&input.old_html, &input.new_html)
            .map_err(|e| DiffError::Generic(e.to_string()))?;

        tracing::debug!(count = patches.len(), "generated patches");
        for (i, patch) in patches.iter().enumerate() {
            tracing::debug!(index = i, ?patch, "patch");
        }

        let patches =
            facet_postcard::to_vec(&patches).map_err(|e| DiffError::Generic(e.to_string()))?;

        Ok(DiffOutcome {
            patches_blob: patches,
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("html_diff", |_handle| HtmlDifferDispatcher::new(
        HtmlDifferImpl
    ))
}
