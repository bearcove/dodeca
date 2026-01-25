//! Patch trace capture for comparing native vs browser patch application.

use std::fmt;

use browser_proto::{ApplyPatchesResult, ComputeAndApplyResult, DomNode};
use hotmeal::{Document, Patch};

use crate::common::document_body_to_dom_node;

/// Result of applying a single patch.
#[derive(Debug, Clone)]
pub enum PatchStepResult {
    /// Patch applied successfully.
    Success {
        /// DOM tree after applying this patch.
        dom_tree: DomNode,
    },
    /// Patch application failed.
    Failure {
        /// Error message.
        error: String,
        /// DOM tree at the time of failure (before the failed patch).
        dom_tree: DomNode,
    },
}

/// One step in the patch trace.
#[derive(Debug, Clone)]
pub struct PatchStep {
    /// Index of the patch.
    pub index: usize,
    /// Debug representation of the patch.
    pub patch_debug: String,
    /// Result of applying this patch.
    pub result: PatchStepResult,
}

/// Complete trace of patch application.
#[derive(Debug, Clone)]
pub struct PatchTrace {
    /// Initial DOM tree before any patches.
    pub initial_tree: DomNode,
    /// Steps for each patch.
    pub steps: Vec<PatchStep>,
}

impl PatchTrace {
    /// Build a trace by applying patches to a document.
    /// Continues even after errors to capture the full trace.
    /// Returns None if the document has no body.
    pub fn capture<'a>(doc: &mut Document<'a>, patches: &[Patch<'a>]) -> Option<Self> {
        let initial_tree = document_body_to_dom_node(doc)?;
        let mut steps = Vec::with_capacity(patches.len());
        let mut slots = doc.init_patch_slots();
        let mut had_error = false;

        for (index, patch) in patches.iter().enumerate() {
            let patch_debug = format!("{:?}", patch);

            if had_error {
                // After an error, we can't continue applying patches,
                // but we record that we couldn't try
                steps.push(PatchStep {
                    index,
                    patch_debug,
                    result: PatchStepResult::Failure {
                        error: "skipped due to previous error".to_string(),
                        dom_tree: document_body_to_dom_node(doc)?,
                    },
                });
            } else {
                match doc.apply_patch_with_slots(patch.clone(), &mut slots) {
                    Ok(()) => {
                        steps.push(PatchStep {
                            index,
                            patch_debug,
                            result: PatchStepResult::Success {
                                dom_tree: document_body_to_dom_node(doc)?,
                            },
                        });
                    }
                    Err(e) => {
                        had_error = true;
                        steps.push(PatchStep {
                            index,
                            patch_debug,
                            result: PatchStepResult::Failure {
                                error: format!("{:?}", e),
                                dom_tree: document_body_to_dom_node(doc)?,
                            },
                        });
                    }
                }
            }
        }

        Some(PatchTrace {
            initial_tree,
            steps,
        })
    }

    /// Check if all patches succeeded.
    pub fn all_succeeded(&self) -> bool {
        self.steps
            .iter()
            .all(|s| matches!(s.result, PatchStepResult::Success { .. }))
    }

    /// Get the final DOM tree (after last successful patch, or initial if none).
    pub fn final_tree(&self) -> &DomNode {
        for step in self.steps.iter().rev() {
            if let PatchStepResult::Success { dom_tree } = &step.result {
                return dom_tree;
            }
        }
        &self.initial_tree
    }
}

impl fmt::Display for PatchTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Initial tree:")?;
        writeln!(f, "{}", self.initial_tree)?;

        for step in &self.steps {
            writeln!(f, "\n--- Step {} ---", step.index)?;
            writeln!(f, "Patch: {}", step.patch_debug)?;
            match &step.result {
                PatchStepResult::Success { dom_tree } => {
                    writeln!(f, "Result: SUCCESS")?;
                    writeln!(f, "{}", dom_tree)?;
                }
                PatchStepResult::Failure { error, dom_tree } => {
                    writeln!(f, "Result: FAILURE")?;
                    writeln!(f, "Error: {}", error)?;
                    writeln!(f, "Tree at failure:")?;
                    writeln!(f, "{}", dom_tree)?;
                }
            }
        }

        Ok(())
    }
}

impl From<&ComputeAndApplyResult> for PatchTrace {
    fn from(result: &ComputeAndApplyResult) -> Self {
        PatchTrace {
            initial_tree: result.initial_dom_tree.clone(),
            steps: result
                .patch_trace
                .iter()
                .map(|step| PatchStep {
                    index: step.index as usize,
                    patch_debug: step.patch_debug.clone(),
                    result: match &step.error {
                        None => PatchStepResult::Success {
                            dom_tree: step.dom_tree.clone(),
                        },
                        Some(error) => PatchStepResult::Failure {
                            error: error.clone(),
                            dom_tree: step.dom_tree.clone(),
                        },
                    },
                })
                .collect(),
        }
    }
}

impl From<&ApplyPatchesResult> for PatchTrace {
    fn from(result: &ApplyPatchesResult) -> Self {
        PatchTrace {
            initial_tree: result.initial_dom_tree.clone(),
            steps: result
                .patch_trace
                .iter()
                .map(|step| PatchStep {
                    index: step.index as usize,
                    patch_debug: step.patch_debug.clone(),
                    result: match &step.error {
                        None => PatchStepResult::Success {
                            dom_tree: step.dom_tree.clone(),
                        },
                        Some(error) => PatchStepResult::Failure {
                            error: error.clone(),
                            dom_tree: step.dom_tree.clone(),
                        },
                    },
                })
                .collect(),
        }
    }
}

/// Compare two traces and format the differences.
pub fn compare_traces(native: &PatchTrace, browser: &PatchTrace) -> Option<String> {
    use similar::{ChangeTag, TextDiff};

    // Compare initial trees
    if native.initial_tree != browser.initial_tree {
        let mut out = String::new();
        out.push_str("Initial tree mismatch!\n\n");
        out.push_str("--- Native ---\n");
        out.push_str(&native.initial_tree.to_string());
        out.push_str("\n--- Browser ---\n");
        out.push_str(&browser.initial_tree.to_string());
        out.push_str("\n--- Diff ---\n");
        let native_str = native.initial_tree.to_string();
        let browser_str = browser.initial_tree.to_string();
        let diff = TextDiff::from_lines(&native_str, &browser_str);
        for change in diff.iter_all_changes() {
            let (sign, color) = match change.tag() {
                ChangeTag::Delete => ("-", "\x1b[31m"),
                ChangeTag::Insert => ("+", "\x1b[32m"),
                ChangeTag::Equal => (" ", ""),
            };
            out.push_str(&format!("{}{}{}\x1b[0m", color, sign, change.value()));
        }
        return Some(out);
    }

    // Compare step counts
    if native.steps.len() != browser.steps.len() {
        return Some(format!(
            "Step count mismatch: native={}, browser={}",
            native.steps.len(),
            browser.steps.len()
        ));
    }

    // Compare each step
    for (i, (n, b)) in native.steps.iter().zip(browser.steps.iter()).enumerate() {
        let mismatch = match (&n.result, &b.result) {
            (
                PatchStepResult::Success { dom_tree: n_tree },
                PatchStepResult::Success { dom_tree: b_tree },
            ) => {
                if n_tree != b_tree {
                    Some(format!(
                        "DOM tree differs after step {}\n\n--- Native ---\n{}\n--- Browser ---\n{}",
                        i, n_tree, b_tree
                    ))
                } else {
                    None
                }
            }
            (PatchStepResult::Success { .. }, PatchStepResult::Failure { error, .. }) => Some(
                format!("Step {}: Native succeeded, browser failed: {}", i, error),
            ),
            (PatchStepResult::Failure { error, .. }, PatchStepResult::Success { .. }) => Some(
                format!("Step {}: Native failed ({}), browser succeeded", i, error),
            ),
            (
                PatchStepResult::Failure { error: n_err, .. },
                PatchStepResult::Failure { error: b_err, .. },
            ) => {
                // Both failed - might be ok if errors are equivalent
                if n_err != b_err {
                    Some(format!(
                        "Step {}: Both failed but with different errors:\n  Native: {}\n  Browser: {}",
                        i, n_err, b_err
                    ))
                } else {
                    None
                }
            }
        };

        if let Some(msg) = mismatch {
            let mut out = String::new();
            out.push_str(&format!("Mismatch at step {}!\n", i));
            out.push_str(&format!("Patch: {}\n\n", n.patch_debug));
            out.push_str(&msg);
            return Some(out);
        }
    }

    None // No differences
}
