//! Dodeca HTML diff cell (cell-html-diff)
//!
//! This cell handles HTML DOM diffing for live reload using hotmeal
//! for parsing and diffing.

use dodeca_cell_runtime::run_cell;
use hotmeal::StrTendril;

use cell_html_diff_proto::{DiffError, DiffInput, DiffOutcome, HtmlDiffer, HtmlDifferDispatcher};

use dodeca_protocol::facet_postcard;

// ============================================================================
// HTML Differ Implementation
// ============================================================================

/// HTML differ implementation using hotmeal.
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

        let old_tendril = StrTendril::from(input.old_html.as_str());
        let new_tendril = StrTendril::from(input.new_html.as_str());
        let patches = hotmeal::diff_html(&old_tendril, &new_tendril)
            .map_err(|e| DiffError::Generic(e.to_string()))?;

        tracing::debug!(count = patches.len(), "generated patches");
        for (i, patch) in patches.iter().enumerate() {
            tracing::debug!(index = i, ?patch, "patch");
        }

        // Convert to owned so we can serialize after tendrils are dropped
        let patches_owned: Vec<hotmeal::Patch<'static>> =
            patches.into_iter().map(|p| p.into_owned()).collect();

        let patches_blob = facet_postcard::to_vec(&patches_owned)
            .map_err(|e| DiffError::Generic(e.to_string()))?;

        Ok(DiffOutcome { patches_blob })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("html_diff", |_handle| HtmlDifferDispatcher::new(
        HtmlDifferImpl
    ))
}
