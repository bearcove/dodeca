//! Dodeca HTML diff cell (cell-html-diff)
//!
//! This cell handles HTML DOM diffing for live reload using facet-format-html
//! for parsing and facet-diff for computing structural differences.

use dodeca_cell_runtime::run_cell;

use cell_html_diff_proto::{
    DiffInput, DiffResult, HtmlDiffResult, HtmlDiffer, HtmlDifferDispatcher,
};

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
    ) -> HtmlDiffResult {
        tracing::debug!(
            old_len = input.old_html.len(),
            new_len = input.new_html.len(),
            "diffing HTML"
        );

        match facet_html_diff::diff_html(&input.old_html, &input.new_html) {
            Ok(patches) => {
                tracing::debug!(count = patches.len(), "generated patches");
                for (i, patch) in patches.iter().enumerate() {
                    tracing::debug!(index = i, ?patch, "patch");
                }

                let nodes_compared = patches.len();
                HtmlDiffResult::Success {
                    result: DiffResult {
                        patches,
                        nodes_compared,
                        nodes_skipped: 0,
                    },
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "diff failed");
                HtmlDiffResult::Error { message: e }
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("html_diff", |_handle| HtmlDifferDispatcher::new(
        HtmlDifferImpl
    ))
}
