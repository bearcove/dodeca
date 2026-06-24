//! Requirement traceability on the dodeca stack.
//!
//! Scans code files for `r[verb rule.id]` references in comments and folds them
//! against the spec rules dodeca already extracts from markdown (via marq) to
//! produce coverage. The lexer is lifted from tracey-core's text-based scanner;
//! the tree-sitter `code_units` path will arrive as a separate picante query.

mod lexer;
mod markdown;
mod positions;
mod report;
mod rule_id;

pub use lexer::{ParseWarning, RefVerb, ReqReference, Reqs, SourceSpan, WarningKind};
pub use report::CoverageReport;
pub use rule_id::{
    RuleId, RuleIdMatch, classify_reference_for_rule, classify_reference_for_rule_str,
    parse_rule_id,
};
