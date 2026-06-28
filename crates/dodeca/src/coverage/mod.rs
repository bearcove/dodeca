//! Requirement traceability on the dodeca stack.
//!
//! Scans code files for `r[verb rule.id]` references in comments and folds them
//! against the spec rules dodeca already extracts from markdown (via marq) to
//! produce coverage. The text lexer handles extensions with no registered
//! tree-sitter grammar; `code_units` handles the rest.

mod api;
mod code_units;
mod languages;
mod lexer;
mod markdown;
mod positions;
mod report;
mod rule_id;

pub use api::{
    CoverageEndpoint, CoverageOutput, CoverageOutputFormat, CoverageRuleResponse, CoverageSelector,
    CoverageStatusResponse, coverage_output, status_response,
};
pub use code_units::{
    CodeUnit, CodeUnitKind, CodeUnits, ExtractedRefs, extract, extract_refs,
    extract_refs_with_warnings,
};
pub use lexer::{ParseWarning, RefVerb, ReqReference, Reqs, SourceSpan, WarningKind};
pub use report::{CoverageReport, StaleReference};
pub use rule_id::{
    RuleId, RuleIdMatch, classify_reference_for_rule, classify_reference_for_rule_str,
    parse_rule_id,
};

pub fn has_tree_sitter_grammar(ext: &str) -> bool {
    languages::for_ext(ext).is_some()
}

/// Extract requirement references (`r[verb rule.id]`) from a code buffer —
/// tree-sitter where a grammar exists, text lexer otherwise. Shared by the
/// `references_in_file` query (on a registered `CodeFile`) and the LSP (on an
/// unsaved editor buffer).
pub fn extract_references(path: &std::path::Path, content: &str) -> Reqs {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !has_tree_sitter_grammar(ext) {
        return Reqs::extract_from_content(path, content);
    }
    let extracted = extract_refs_with_warnings(path, content);
    let mut reqs = Reqs::new();
    for full_ref in extracted.references {
        let verb = match full_ref.verb.as_str() {
            "define" => RefVerb::Define,
            "impl" => RefVerb::Impl,
            "verify" => RefVerb::Verify,
            "depends" => RefVerb::Depends,
            "related" => RefVerb::Related,
            _ => continue,
        };
        reqs.references.push(ReqReference {
            prefix: full_ref.prefix,
            verb,
            req_id: full_ref.req_id,
            file: path.to_path_buf(),
            line: full_ref.line,
            span: SourceSpan::new(full_ref.byte_offset, full_ref.byte_length),
        });
    }
    for warning in extracted.warnings {
        reqs.warnings.push(ParseWarning {
            file: path.to_path_buf(),
            line: warning.line,
            span: SourceSpan::new(warning.byte_offset, warning.byte_length),
            kind: WarningKind::MalformedReference,
        });
    }
    reqs
}
