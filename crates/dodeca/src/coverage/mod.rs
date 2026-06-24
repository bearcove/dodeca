//! Requirement traceability on the dodeca stack.
//!
//! Scans code files for `r[verb rule.id]` references in comments and folds them
//! against the spec rules dodeca already extracts from markdown (via marq) to
//! produce coverage. The text lexer handles extensions with no registered
//! tree-sitter grammar; `code_units` handles the rest.

mod code_units;
mod languages;
mod lexer;
mod markdown;
mod positions;
mod report;
mod rule_id;

pub use code_units::{
    CodeUnit, CodeUnitKind, CodeUnits, ExtractedRefs, extract, extract_refs,
    extract_refs_with_warnings,
};
pub use lexer::{ParseWarning, RefVerb, ReqReference, Reqs, SourceSpan, WarningKind};
pub use report::CoverageReport;
pub use rule_id::{
    RuleId, RuleIdMatch, classify_reference_for_rule, classify_reference_for_rule_str,
    parse_rule_id,
};

pub fn has_tree_sitter_grammar(ext: &str) -> bool {
    languages::for_ext(ext).is_some()
}
