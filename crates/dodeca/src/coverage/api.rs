use facet::Facet;
use std::collections::{BTreeMap, BTreeSet};

use super::{CoverageReport, RefVerb, ReqReference, RuleId, StaleReference};

const COVERAGE_NAV_CSS: &str = r#"
:root {
  color-scheme: light dark;
  --bg: #f6f4ef;
  --panel: #fffdfa;
  --panel-alt: #f1eee7;
  --text: #1e2420;
  --muted: #68706a;
  --line: #d8d2c5;
  --line-strong: #b7ad9b;
  --accent: #0f766e;
  --accent-bg: #dff3ef;
  --ok: #177245;
  --ok-bg: #e1f3e7;
  --warn: #a45b00;
  --warn-bg: #fff2cc;
  --bad: #b42318;
  --bad-bg: #ffe2dc;
  --code: #ede8dd;
}

@media (prefers-color-scheme: dark) {
  :root {
    --bg: #141613;
    --panel: #1d211d;
    --panel-alt: #242a24;
    --text: #eceee8;
    --muted: #a5ad9f;
    --line: #343b33;
    --line-strong: #4a5549;
    --accent: #4fd1bd;
    --accent-bg: #153c38;
    --ok: #67d391;
    --ok-bg: #173b24;
    --warn: #f2bf4d;
    --warn-bg: #3b2d12;
    --bad: #ff8a7a;
    --bad-bg: #431d1a;
    --code: #2b3129;
  }
}

* { box-sizing: border-box; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--text);
  font: 14px/1.45 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
main { max-width: 1320px; margin: 0 auto; padding: 28px 20px 52px; }
header { display: flex; align-items: flex-start; justify-content: space-between; gap: 20px; margin-bottom: 22px; }
h1 { font-size: 28px; line-height: 1.1; margin: 0 0 6px; }
h2 { font-size: 18px; margin: 0; }
h3 { font-size: 15px; margin: 0; }
.muted { color: var(--muted); }
.views, .metrics, .badges { display: flex; gap: 8px; flex-wrap: wrap; }
.pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--line);
  border-radius: 6px;
  padding: 6px 10px;
  background: var(--panel);
  color: var(--text);
  text-decoration: none;
}
.metric {
  min-width: 142px;
  border: 1px solid var(--line);
  border-radius: 8px;
  padding: 10px 12px;
  background: var(--panel);
}
.metric strong { display: block; font-size: 20px; }
section { border-top: 1px solid var(--line); padding-top: 18px; margin-top: 24px; }
.section-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
}
.queue-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 10px;
}
.queue {
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--panel);
  overflow: hidden;
}
.queue h3 {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  padding: 10px 12px;
  background: var(--panel-alt);
  border-bottom: 1px solid var(--line);
}
.queue ul { list-style: none; margin: 0; padding: 8px 12px 10px; }
.queue li + li { margin-top: 6px; }
.queue a { text-decoration: none; }
.route {
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--panel);
  overflow: hidden;
  margin-bottom: 12px;
}
.route summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  cursor: pointer;
  padding: 12px 14px;
  background: var(--panel-alt);
  border-bottom: 1px solid var(--line);
}
.route-source { margin-left: 8px; color: var(--muted); font-size: 12px; }
.rule-list { display: grid; gap: 10px; padding: 12px; }
.rule-card {
  border: 1px solid var(--line);
  border-left: 4px solid var(--line-strong);
  border-radius: 8px;
  background: var(--panel);
  overflow: hidden;
}
.rule-card.is-covered { border-left-color: var(--ok); }
.rule-card.is-unverified, .rule-card.is-stale { border-left-color: var(--warn); }
.rule-card.is-unimplemented { border-left-color: var(--bad); }
.rule-head {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 10px;
  align-items: start;
  padding: 11px 12px;
  background: color-mix(in srgb, var(--panel-alt) 68%, transparent);
  border-bottom: 1px solid var(--line);
}
.rule-title {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.rule-title code { font-size: 13px; }
.rule-meta { color: var(--muted); font-size: 12px; }
.rule-body { padding: 12px 14px; }
.rule-body > *:first-child { margin-top: 0; }
.rule-body > *:last-child { margin-bottom: 0; }
.rule-body p, .rule-body li { color: var(--text); }
.rule-body pre {
  overflow: auto;
  border: 1px solid var(--line);
  border-radius: 6px;
  padding: 10px;
  background: var(--code);
}
.badge {
  display: inline-flex;
  align-items: center;
  border: 1px solid var(--line);
  border-radius: 5px;
  padding: 2px 6px;
  font-size: 12px;
  font-weight: 650;
  line-height: 1.25;
  white-space: nowrap;
}
.badge.ok { color: var(--ok); background: var(--ok-bg); border-color: color-mix(in srgb, var(--ok) 35%, var(--line)); }
.badge.warn { color: var(--warn); background: var(--warn-bg); border-color: color-mix(in srgb, var(--warn) 35%, var(--line)); }
.badge.bad { color: var(--bad); background: var(--bad-bg); border-color: color-mix(in srgb, var(--bad) 35%, var(--line)); }
table {
  width: 100%;
  border-collapse: collapse;
  background: var(--panel);
  border: 1px solid var(--line);
  border-radius: 8px;
  overflow: hidden;
}
th, td {
  padding: 8px 10px;
  border-bottom: 1px solid var(--line);
  text-align: left;
  vertical-align: top;
}
th {
  font-size: 12px;
  text-transform: uppercase;
  color: var(--muted);
  background: color-mix(in srgb, var(--panel), var(--line) 20%);
}
tr:last-child td { border-bottom: 0; }
code { background: var(--code); padding: 1px 4px; border-radius: 4px; }
a { color: var(--accent); }
.bad { color: var(--bad); }
.warn { color: var(--warn); }
.ok { color: var(--accent); }
.empty { color: var(--muted); padding: 10px 0; }

@media (max-width: 700px) {
  main { padding: 20px 12px; }
  header { display: block; }
  .rule-head { grid-template-columns: 1fr; }
  table { display: block; overflow-x: auto; white-space: nowrap; }
  .metric { flex: 1 1 120px; }
}
"#;

const COVERAGE_MARKDOWN_CSS: &str = r#"
body {
  margin: 0;
  background: #f7f7f5;
  color: #171717;
  font: 14px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
main { max-width: 1100px; margin: 0 auto; padding: 24px; }
pre {
  white-space: pre-wrap;
  background: #fff;
  border: 1px solid #d9d7d1;
  border-radius: 8px;
  padding: 16px;
  overflow: auto;
}
@media (prefers-color-scheme: dark) {
  body { background: #181818; color: #ededed; }
  pre { background: #222; border-color: #3b3b3b; }
}
"#;

/// Coverage route selected by the URL path or CLI subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverageEndpoint {
    Nav,
    Status,
    Config,
    Uncovered,
    Untested,
    Unmapped,
    Stale,
    Invalid,
    Validate { threshold: Option<u8> },
    Rule { id: String },
}

/// Coverage source/impl filter selected by HTTP query params or CLI options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverageSelector {
    pub source_name: Option<String>,
    pub impl_name: Option<String>,
}

impl CoverageSelector {
    pub fn new(source_name: Option<String>, impl_name: Option<String>) -> Self {
        Self {
            source_name: source_name.filter(|name| !name.is_empty()),
            impl_name: impl_name.filter(|name| !name.is_empty()),
        }
    }
}

/// Output representation selected by the URL suffix or CLI format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageOutputFormat {
    Json,
    Markdown,
    Html,
}

impl CoverageOutputFormat {
    pub fn mime(self) -> &'static str {
        match self {
            CoverageOutputFormat::Json => "application/json; charset=utf-8",
            CoverageOutputFormat::Markdown => "text/markdown; charset=utf-8",
            CoverageOutputFormat::Html => "text/html; charset=utf-8",
        }
    }
}

/// Rendered coverage response ready for HTTP or CLI output.
#[derive(Debug, Clone)]
pub struct CoverageOutput {
    pub body: String,
    pub format: CoverageOutputFormat,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageStatusResponse {
    pub spec_name: String,
    pub total_rules: usize,
    pub referenced_rules: usize,
    pub implemented_rules: usize,
    pub verified_rules: usize,
    pub uncovered_rules: usize,
    pub untested_rules: usize,
    pub invalid_references: usize,
    pub stale_references: usize,
    pub test_impl_references: usize,
    pub reference_coverage_percent: f64,
    pub implementation_coverage_percent: f64,
    pub verification_coverage_percent: f64,
    pub rules: Vec<CoverageRuleSummary>,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageNavigationResponse {
    pub spec_name: String,
    pub status: CoverageStatusResponse,
    pub config: CoverageConfigResponse,
    pub views: Vec<CoverageNavigationView>,
    pub spec_routes: Vec<CoverageSpecRouteNav>,
    pub coverage_rules: Vec<CoverageRuleSummary>,
    pub source_files: Vec<CoverageSourceFileNav>,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageNavigationView {
    pub id: String,
    pub title: String,
    pub markdown_href: String,
    pub json_href: String,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageSpecRouteNav {
    pub source_name: String,
    pub route: String,
    pub rules: Vec<CoverageSpecRuleNav>,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageSpecRuleNav {
    pub id: String,
    pub rule_href: String,
    pub route_href: String,
    pub anchor_id: String,
    pub line: usize,
    pub implemented: bool,
    pub verified: bool,
    pub stale_refs: usize,
    pub raw: String,
    pub html: String,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageSourceFileNav {
    pub file: String,
    pub rules: Vec<String>,
    pub total_references: usize,
    pub impl_refs: usize,
    pub verify_refs: usize,
    pub depends_refs: usize,
    pub related_refs: usize,
    pub invalid_refs: usize,
    pub stale_refs: usize,
    pub unmapped_units: Vec<CoverageUnmappedUnit>,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageRuleSummary {
    pub id: String,
    pub referenced: bool,
    pub implemented: bool,
    pub verified: bool,
    pub impl_refs: usize,
    pub verify_refs: usize,
    pub depends_refs: usize,
    pub related_refs: usize,
    pub stale_refs: usize,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageConfigResponse {
    pub spec_name: String,
    pub impls: Vec<CoverageConfigImplResponse>,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageConfigImplResponse {
    pub source_name: String,
    pub mount: String,
    pub impl_name: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub test_include: Vec<String>,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageRuleResponse {
    pub id: String,
    pub referenced: bool,
    pub implemented: bool,
    pub verified: bool,
    pub impl_refs: Vec<CoverageReference>,
    pub verify_refs: Vec<CoverageReference>,
    pub depends_refs: Vec<CoverageReference>,
    pub related_refs: Vec<CoverageReference>,
    pub stale_refs: Vec<CoverageStaleReference>,
    pub definitions: Vec<CoverageRuleDefinition>,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageReference {
    pub prefix: String,
    pub verb: String,
    pub rule_id: String,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageStaleReference {
    pub current_rule_id: String,
    pub reference: CoverageReference,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageRuleDefinition {
    pub source_name: String,
    pub route: String,
    pub anchor_id: String,
    pub line: usize,
    pub raw: String,
    pub html: String,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageReferenceListResponse {
    pub spec_name: String,
    pub references: Vec<CoverageReference>,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageStaleListResponse {
    pub spec_name: String,
    pub references: Vec<CoverageStaleReference>,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageRuleListResponse {
    pub spec_name: String,
    pub rules: Vec<CoverageRuleSummary>,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageUnmappedResponse {
    pub spec_name: String,
    pub units: Vec<CoverageUnmappedUnit>,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageUnmappedUnit {
    pub file: String,
    pub line: usize,
    pub end_line: usize,
    pub kind: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
pub struct CoverageValidationResponse {
    pub spec_name: String,
    pub passing: bool,
    pub threshold: Option<u8>,
    pub status: CoverageStatusResponse,
}

pub fn coverage_output(
    report: &CoverageReport,
    endpoint: CoverageEndpoint,
    format: CoverageOutputFormat,
) -> Result<Option<CoverageOutput>, String> {
    let body = match endpoint {
        CoverageEndpoint::Nav => {
            let response = navigation_response(report);
            match format {
                CoverageOutputFormat::Json => json(&response)?,
                CoverageOutputFormat::Markdown => render_navigation_markdown(&response),
                CoverageOutputFormat::Html => render_navigation_html(&response),
            }
        }
        CoverageEndpoint::Status => match format {
            CoverageOutputFormat::Json => json(&status_response(report))?,
            CoverageOutputFormat::Markdown => render_status_markdown(report),
            CoverageOutputFormat::Html => {
                render_markdown_html("Coverage Status", &render_status_markdown(report))
            }
        },
        CoverageEndpoint::Config => {
            let response = config_response(report);
            match format {
                CoverageOutputFormat::Json => json(&response)?,
                CoverageOutputFormat::Markdown => render_config_markdown(&response),
                CoverageOutputFormat::Html => {
                    render_markdown_html("Coverage Config", &render_config_markdown(&response))
                }
            }
        }
        CoverageEndpoint::Uncovered => {
            let response = CoverageRuleListResponse {
                spec_name: report.spec_name.clone(),
                rules: status_response(report)
                    .rules
                    .into_iter()
                    .filter(|rule| !rule.implemented)
                    .collect(),
            };
            match format {
                CoverageOutputFormat::Json => json(&response)?,
                CoverageOutputFormat::Markdown => render_rule_list_markdown(
                    "Uncovered Rules",
                    "Rules without implementation references.",
                    &response,
                ),
                CoverageOutputFormat::Html => render_markdown_html(
                    "Uncovered Rules",
                    &render_rule_list_markdown(
                        "Uncovered Rules",
                        "Rules without implementation references.",
                        &response,
                    ),
                ),
            }
        }
        CoverageEndpoint::Untested => {
            let response = CoverageRuleListResponse {
                spec_name: report.spec_name.clone(),
                rules: status_response(report)
                    .rules
                    .into_iter()
                    .filter(|rule| !rule.verified)
                    .collect(),
            };
            match format {
                CoverageOutputFormat::Json => json(&response)?,
                CoverageOutputFormat::Markdown => render_rule_list_markdown(
                    "Untested Rules",
                    "Rules without verification references.",
                    &response,
                ),
                CoverageOutputFormat::Html => render_markdown_html(
                    "Untested Rules",
                    &render_rule_list_markdown(
                        "Untested Rules",
                        "Rules without verification references.",
                        &response,
                    ),
                ),
            }
        }
        CoverageEndpoint::Unmapped => {
            let response = CoverageUnmappedResponse {
                spec_name: report.spec_name.clone(),
                units: unmapped_units(report),
            };
            match format {
                CoverageOutputFormat::Json => json(&response)?,
                CoverageOutputFormat::Markdown => render_unmapped_markdown(&response),
                CoverageOutputFormat::Html => render_markdown_html(
                    "Unmapped Code Units",
                    &render_unmapped_markdown(&response),
                ),
            }
        }
        CoverageEndpoint::Stale => {
            let response = CoverageStaleListResponse {
                spec_name: report.spec_name.clone(),
                references: stale_references(report),
            };
            match format {
                CoverageOutputFormat::Json => json(&response)?,
                CoverageOutputFormat::Markdown => render_stale_markdown(&response),
                CoverageOutputFormat::Html => {
                    render_markdown_html("Stale References", &render_stale_markdown(&response))
                }
            }
        }
        CoverageEndpoint::Invalid => {
            let response = CoverageReferenceListResponse {
                spec_name: report.spec_name.clone(),
                references: references(&report.invalid_references),
            };
            match format {
                CoverageOutputFormat::Json => json(&response)?,
                CoverageOutputFormat::Markdown => render_references_markdown(
                    "Invalid References",
                    "References that do not match a known current or older rule.",
                    &response,
                ),
                CoverageOutputFormat::Html => render_markdown_html(
                    "Invalid References",
                    &render_references_markdown(
                        "Invalid References",
                        "References that do not match a known current or older rule.",
                        &response,
                    ),
                ),
            }
        }
        CoverageEndpoint::Validate { threshold } => {
            let status = status_response(report);
            let passing = report.invalid_references.is_empty()
                && report.stale_references.is_empty()
                && report.test_impl_references.is_empty()
                && threshold
                    .map(|threshold| status.implementation_coverage_percent >= f64::from(threshold))
                    .unwrap_or(true);
            let response = CoverageValidationResponse {
                spec_name: report.spec_name.clone(),
                passing,
                threshold,
                status,
            };
            match format {
                CoverageOutputFormat::Json => json(&response)?,
                CoverageOutputFormat::Markdown => render_validation_markdown(&response),
                CoverageOutputFormat::Html => render_markdown_html(
                    "Coverage Validation",
                    &render_validation_markdown(&response),
                ),
            }
        }
        CoverageEndpoint::Rule { id } => {
            let Some(response) = rule_response(report, &id) else {
                return Ok(None);
            };
            match format {
                CoverageOutputFormat::Json => json(&response)?,
                CoverageOutputFormat::Markdown => render_rule_markdown(&response),
                CoverageOutputFormat::Html => render_markdown_html(
                    &format!("Rule {}", response.id),
                    &render_rule_markdown(&response),
                ),
            }
        }
    };

    Ok(Some(CoverageOutput { body, format }))
}

fn json<T: Facet<'static>>(value: &T) -> Result<String, String> {
    facet_json::to_string_pretty(value).map_err(|err| format!("{err:?}"))
}

pub fn status_response(report: &CoverageReport) -> CoverageStatusResponse {
    let mut rules: Vec<_> = report
        .covered_rules
        .iter()
        .chain(report.uncovered_rules.iter())
        .cloned()
        .collect();
    rules.sort();
    rules.dedup();

    let summaries: Vec<_> = rules.iter().map(|id| rule_summary(report, id)).collect();
    let total_rules = summaries.len();
    let referenced_rules = summaries.iter().filter(|rule| rule.referenced).count();
    let implemented_rules = summaries.iter().filter(|rule| rule.implemented).count();
    let verified_rules = summaries.iter().filter(|rule| rule.verified).count();

    CoverageStatusResponse {
        spec_name: report.spec_name.clone(),
        total_rules,
        referenced_rules,
        implemented_rules,
        verified_rules,
        uncovered_rules: total_rules.saturating_sub(implemented_rules),
        untested_rules: total_rules.saturating_sub(verified_rules),
        invalid_references: report.invalid_references.len(),
        stale_references: report.stale_references.len(),
        test_impl_references: report.test_impl_references.len(),
        reference_coverage_percent: percent(referenced_rules, total_rules),
        implementation_coverage_percent: percent(implemented_rules, total_rules),
        verification_coverage_percent: percent(verified_rules, total_rules),
        rules: summaries,
    }
}

pub fn config_response(report: &CoverageReport) -> CoverageConfigResponse {
    let impls = report
        .config_impls
        .iter()
        .map(|impl_| CoverageConfigImplResponse {
            source_name: impl_.source_name.clone(),
            mount: impl_.mount.clone(),
            impl_name: impl_.impl_name.clone(),
            include: impl_.include.clone(),
            exclude: impl_.exclude.clone(),
            test_include: impl_.test_include.clone(),
        })
        .collect();

    CoverageConfigResponse {
        spec_name: report.spec_name.clone(),
        impls,
    }
}

pub fn navigation_response(report: &CoverageReport) -> CoverageNavigationResponse {
    let status = status_response(report);
    let config = config_response(report);
    let coverage_rules = status.rules.clone();
    CoverageNavigationResponse {
        spec_name: report.spec_name.clone(),
        status,
        config,
        views: vec![
            CoverageNavigationView {
                id: "spec".to_string(),
                title: "Spec View".to_string(),
                markdown_href: "nav.md#spec-view".to_string(),
                json_href: "nav.json".to_string(),
            },
            CoverageNavigationView {
                id: "coverage".to_string(),
                title: "Coverage View".to_string(),
                markdown_href: "nav.md#coverage-view".to_string(),
                json_href: "nav.json".to_string(),
            },
            CoverageNavigationView {
                id: "sources".to_string(),
                title: "Sources View".to_string(),
                markdown_href: "nav.md#sources-view".to_string(),
                json_href: "nav.json".to_string(),
            },
        ],
        spec_routes: spec_routes(report),
        coverage_rules,
        source_files: source_files(report),
    }
}

pub fn rule_response(report: &CoverageReport, id: &str) -> Option<CoverageRuleResponse> {
    let rule_id = super::parse_rule_id(id)?;
    if !report.covered_rules.contains(&rule_id) && !report.uncovered_rules.contains(&rule_id) {
        return None;
    }

    let impl_refs = refs_for(report, &rule_id, RefVerb::Impl);
    let verify_refs = refs_for(report, &rule_id, RefVerb::Verify);
    let depends_refs = refs_for(report, &rule_id, RefVerb::Depends);
    let related_refs = refs_for(report, &rule_id, RefVerb::Related);
    let stale_refs: Vec<_> = report
        .stale_references
        .iter()
        .filter(|stale| stale.current_rule_id == rule_id)
        .map(stale_reference)
        .collect();

    Some(CoverageRuleResponse {
        id: rule_id.to_string(),
        referenced: report.covered_rules.contains(&rule_id),
        implemented: !impl_refs.is_empty(),
        verified: !verify_refs.is_empty(),
        impl_refs,
        verify_refs,
        depends_refs,
        related_refs,
        stale_refs,
        definitions: rule_definitions(report, &rule_id),
    })
}

fn rule_summary(report: &CoverageReport, id: &RuleId) -> CoverageRuleSummary {
    let impl_refs = ref_count(report, id, RefVerb::Impl);
    let verify_refs = ref_count(report, id, RefVerb::Verify);
    let depends_refs = ref_count(report, id, RefVerb::Depends);
    let related_refs = ref_count(report, id, RefVerb::Related);
    CoverageRuleSummary {
        id: id.to_string(),
        referenced: report.covered_rules.contains(id),
        implemented: impl_refs > 0,
        verified: verify_refs > 0,
        impl_refs,
        verify_refs,
        depends_refs,
        related_refs,
        stale_refs: report
            .stale_references
            .iter()
            .filter(|stale| stale.current_rule_id == *id)
            .count(),
    }
}

fn spec_routes(report: &CoverageReport) -> Vec<CoverageSpecRouteNav> {
    let mut routes: BTreeMap<(String, String), Vec<CoverageSpecRuleNav>> = BTreeMap::new();
    for definitions in report.definitions_by_rule.values() {
        for definition in definitions {
            let summary = rule_summary(report, &definition.id);
            let id = definition.id.to_string();
            let route_href = if definition.anchor_id.is_empty() {
                definition.route.clone()
            } else {
                format!("{}#{}", definition.route, definition.anchor_id)
            };
            routes
                .entry((definition.source_name.clone(), definition.route.clone()))
                .or_default()
                .push(CoverageSpecRuleNav {
                    rule_href: rule_href(&id),
                    route_href,
                    anchor_id: definition.anchor_id.clone(),
                    line: definition.line,
                    implemented: summary.implemented,
                    verified: summary.verified,
                    stale_refs: summary.stale_refs,
                    raw: definition.raw.clone(),
                    html: definition.html.clone(),
                    id,
                });
        }
    }

    routes
        .into_iter()
        .map(|((source_name, route), mut rules)| {
            rules.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.id.cmp(&b.id)));
            CoverageSpecRouteNav {
                source_name,
                route,
                rules,
            }
        })
        .collect()
}

#[derive(Debug, Default)]
struct SourceFileNavBuilder {
    rules: BTreeSet<String>,
    total_references: usize,
    impl_refs: usize,
    verify_refs: usize,
    depends_refs: usize,
    related_refs: usize,
    invalid_refs: usize,
    stale_refs: usize,
    unmapped_units: Vec<CoverageUnmappedUnit>,
}

fn source_files(report: &CoverageReport) -> Vec<CoverageSourceFileNav> {
    let mut files: BTreeMap<String, SourceFileNavBuilder> = BTreeMap::new();

    for refs in report.references_by_rule.values() {
        for reference in refs {
            let entry = files
                .entry(reference.file.display().to_string())
                .or_default();
            entry.total_references += 1;
            entry.rules.insert(reference.req_id.to_string());
            match reference.verb {
                RefVerb::Impl => entry.impl_refs += 1,
                RefVerb::Verify => entry.verify_refs += 1,
                RefVerb::Depends => entry.depends_refs += 1,
                RefVerb::Related => entry.related_refs += 1,
                RefVerb::Define => {}
            }
        }
    }

    for reference in &report.invalid_references {
        let entry = files
            .entry(reference.file.display().to_string())
            .or_default();
        entry.invalid_refs += 1;
        entry.rules.insert(reference.req_id.to_string());
    }

    for stale in &report.stale_references {
        let entry = files
            .entry(stale.reference.file.display().to_string())
            .or_default();
        entry.stale_refs += 1;
        entry.rules.insert(stale.current_rule_id.to_string());
        entry.rules.insert(stale.reference.req_id.to_string());
    }

    for unit in unmapped_units(report) {
        files
            .entry(unit.file.clone())
            .or_default()
            .unmapped_units
            .push(unit);
    }

    files
        .into_iter()
        .map(|(file, builder)| CoverageSourceFileNav {
            file,
            rules: builder.rules.into_iter().collect(),
            total_references: builder.total_references,
            impl_refs: builder.impl_refs,
            verify_refs: builder.verify_refs,
            depends_refs: builder.depends_refs,
            related_refs: builder.related_refs,
            invalid_refs: builder.invalid_refs,
            stale_refs: builder.stale_refs,
            unmapped_units: builder.unmapped_units,
        })
        .collect()
}

fn ref_count(report: &CoverageReport, id: &RuleId, verb: RefVerb) -> usize {
    report
        .references_by_verb
        .get(&verb)
        .and_then(|by_rule| by_rule.get(id))
        .map(Vec::len)
        .unwrap_or(0)
}

fn refs_for(report: &CoverageReport, id: &RuleId, verb: RefVerb) -> Vec<CoverageReference> {
    report
        .references_by_verb
        .get(&verb)
        .and_then(|by_rule| by_rule.get(id))
        .map(|refs| references(refs))
        .unwrap_or_default()
}

fn references(references: &[ReqReference]) -> Vec<CoverageReference> {
    let mut out: Vec<_> = references.iter().map(reference).collect();
    out.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.rule_id.cmp(&b.rule_id))
    });
    out
}

fn reference(reference: &ReqReference) -> CoverageReference {
    CoverageReference {
        prefix: reference.prefix.clone(),
        verb: reference.verb.as_str().to_string(),
        rule_id: reference.req_id.to_string(),
        file: reference.file.display().to_string(),
        line: reference.line,
    }
}

fn stale_references(report: &CoverageReport) -> Vec<CoverageStaleReference> {
    let mut out: Vec<_> = report
        .stale_references
        .iter()
        .map(stale_reference)
        .collect();
    out.sort_by(|a, b| {
        a.current_rule_id
            .cmp(&b.current_rule_id)
            .then_with(|| a.reference.file.cmp(&b.reference.file))
            .then_with(|| a.reference.line.cmp(&b.reference.line))
    });
    out
}

fn stale_reference(stale: &StaleReference) -> CoverageStaleReference {
    CoverageStaleReference {
        current_rule_id: stale.current_rule_id.to_string(),
        reference: reference(&stale.reference),
    }
}

fn rule_definitions(report: &CoverageReport, id: &RuleId) -> Vec<CoverageRuleDefinition> {
    report
        .definitions_by_rule
        .get(id)
        .map(|definitions| {
            definitions
                .iter()
                .map(|definition| CoverageRuleDefinition {
                    source_name: definition.source_name.clone(),
                    route: definition.route.clone(),
                    anchor_id: definition.anchor_id.clone(),
                    line: definition.line,
                    raw: definition.raw.clone(),
                    html: definition.html.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn unmapped_units(report: &CoverageReport) -> Vec<CoverageUnmappedUnit> {
    report
        .unmapped_units
        .iter()
        .map(|unit| CoverageUnmappedUnit {
            file: unit.file.clone(),
            line: unit.line,
            end_line: unit.end_line,
            kind: unit.kind.clone(),
            name: unit.name.clone(),
        })
        .collect()
}

fn percent(count: usize, total: usize) -> f64 {
    if total == 0 {
        100.0
    } else {
        (count as f64 / total as f64) * 100.0
    }
}

fn render_status_markdown(report: &CoverageReport) -> String {
    let status = status_response(report);
    let mut out = String::new();
    out.push_str("# Coverage Status\n\n");
    out.push_str(&format!("Spec: `{}`\n\n", status.spec_name));
    out.push_str("| Metric | Count | Percent |\n");
    out.push_str("| --- | ---: | ---: |\n");
    out.push_str(&format!(
        "| Referenced | {}/{} | {:.1}% |\n",
        status.referenced_rules, status.total_rules, status.reference_coverage_percent
    ));
    out.push_str(&format!(
        "| Implemented | {}/{} | {:.1}% |\n",
        status.implemented_rules, status.total_rules, status.implementation_coverage_percent
    ));
    out.push_str(&format!(
        "| Verified | {}/{} | {:.1}% |\n",
        status.verified_rules, status.total_rules, status.verification_coverage_percent
    ));
    out.push_str(&format!(
        "| Invalid refs | {} |  |\n",
        status.invalid_references
    ));
    out.push_str(&format!(
        "| Stale refs | {} |  |\n",
        status.stale_references
    ));
    out.push_str(&format!(
        "| Test impl refs | {} |  |\n",
        status.test_impl_references
    ));

    let next = [
        ("Navigation", "nav.md"),
        ("Config", "config.md"),
        ("Uncovered", "uncovered.md"),
        ("Untested", "untested.md"),
        ("Unmapped", "unmapped.md"),
        ("Stale", "stale.md"),
        ("Invalid", "invalid.md"),
        ("Validate", "validate.md"),
    ];
    out.push_str("\n## Queries\n\n");
    for (label, href) in next {
        out.push_str(&format!("- [{label}]({href})\n"));
    }
    out.push_str(
        "\n## Agent Guide\n\nAgents: run `ddc agent` for the Dodeca mental model, Zola differences, and coverage workflow. Run `ddc agent install` to install or refresh the thin skill.\n",
    );
    out
}

fn render_navigation_markdown(response: &CoverageNavigationResponse) -> String {
    let mut out = String::new();
    out.push_str("# Coverage Navigation\n\n");
    out.push_str(&format!("Spec: `{}`\n\n", response.spec_name));
    out.push_str("| Metric | Count | Percent |\n");
    out.push_str("| --- | ---: | ---: |\n");
    out.push_str(&format!(
        "| Implemented | {}/{} | {:.1}% |\n",
        response.status.implemented_rules,
        response.status.total_rules,
        response.status.implementation_coverage_percent
    ));
    out.push_str(&format!(
        "| Verified | {}/{} | {:.1}% |\n",
        response.status.verified_rules,
        response.status.total_rules,
        response.status.verification_coverage_percent
    ));
    out.push_str(&format!(
        "| Invalid refs | {} |  |\n",
        response.status.invalid_references
    ));
    out.push_str(&format!(
        "| Stale refs | {} |  |\n",
        response.status.stale_references
    ));

    out.push_str("\n## Views\n\n");
    for view in &response.views {
        out.push_str(&format!(
            "- [{}]({}) (`{}`)\n",
            view.title, view.markdown_href, view.id
        ));
    }

    out.push_str("\n## Query Anchors\n\n");
    let queries = [
        ("Status", "status.md", "status.json"),
        ("Config", "config.md", "config.json"),
        ("Uncovered", "uncovered.md", "uncovered.json"),
        ("Untested", "untested.md", "untested.json"),
        ("Unmapped", "unmapped.md", "unmapped.json"),
        ("Stale", "stale.md", "stale.json"),
        ("Invalid", "invalid.md", "invalid.json"),
        ("Validate", "validate.md", "validate.json"),
    ];
    for (label, md, json) in queries {
        out.push_str(&format!("- {label}: [{md}]({md}) / [{json}]({json})\n"));
    }

    out.push_str("\n## Spec View\n\n");
    if response.spec_routes.is_empty() {
        out.push_str("No spec rules found.\n\n");
    } else {
        for route in &response.spec_routes {
            out.push_str(&format!("### `{}`\n\n", route.route));
            if !route.source_name.is_empty() {
                out.push_str(&format!("Source: `{}`\n\n", route.source_name));
            }
            out.push_str("| Rule | Line | Impl | Verify | Definition |\n");
            out.push_str("| --- | ---: | --- | --- | --- |\n");
            for rule in &route.rules {
                out.push_str(&format!(
                    "| [`{}`]({}) | {} | {} | {} | [route]({}) |\n",
                    rule.id,
                    rule.rule_href,
                    rule.line,
                    yes_no(rule.implemented),
                    yes_no(rule.verified),
                    rule.route_href
                ));
            }
            out.push('\n');
        }
    }

    out.push_str("## Coverage View\n\n");
    if response.coverage_rules.is_empty() {
        out.push_str("No rules found.\n\n");
    } else {
        out.push_str("| Rule | Impl refs | Verify refs | Stale refs |\n");
        out.push_str("| --- | ---: | ---: | ---: |\n");
        for rule in &response.coverage_rules {
            out.push_str(&format!(
                "| [`{}`]({}) | {} | {} | {} |\n",
                rule.id,
                rule_href(&rule.id),
                rule.impl_refs,
                rule.verify_refs,
                rule.stale_refs
            ));
        }
        out.push('\n');
    }

    out.push_str("## Sources View\n\n");
    if response.source_files.is_empty() {
        out.push_str("No source files found.\n");
    } else {
        out.push_str("| File | Rules | Refs | Impl | Verify | Stale | Invalid | Unmapped |\n");
        out.push_str("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |\n");
        for file in &response.source_files {
            let rules = if file.rules.is_empty() {
                String::new()
            } else {
                file.rules
                    .iter()
                    .map(|rule| format!("`{rule}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            out.push_str(&format!(
                "| `{}` | {} | {} | {} | {} | {} | {} | {} |\n",
                file.file,
                rules,
                file.total_references,
                file.impl_refs,
                file.verify_refs,
                file.stale_refs,
                file.invalid_refs,
                file.unmapped_units.len()
            ));
        }
    }
    out
}

fn render_navigation_html(response: &CoverageNavigationResponse) -> String {
    let mut out = String::new();
    out.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    out.push_str(&format!(
        "<title>Coverage Navigation - {}</title>",
        html_escape(&response.spec_name)
    ));
    out.push_str("<style>");
    out.push_str(COVERAGE_NAV_CSS);
    out.push_str("</style>");
    out.push_str("</head><body><main>");
    out.push_str("<header><div>");
    out.push_str("<h1>Coverage Navigation</h1>");
    out.push_str(&format!(
        "<div class=\"muted\">Spec <code>{}</code></div>",
        html_escape(&response.spec_name)
    ));
    out.push_str("</div><nav class=\"views\">");
    for view in &response.views {
        out.push_str(&format!(
            "<a class=\"pill\" href=\"#{}\">{}</a>",
            html_escape(&view.id),
            html_escape(&view.title)
        ));
    }
    out.push_str("<a class=\"pill\" href=\"nav.md\">Markdown</a>");
    out.push_str("<a class=\"pill\" href=\"nav.json\">JSON</a>");
    out.push_str("</nav></header>");

    out.push_str("<div class=\"metrics\">");
    render_metric(
        &mut out,
        "Implemented",
        response.status.implemented_rules,
        response.status.total_rules,
        response.status.implementation_coverage_percent,
        "ok",
    );
    render_metric(
        &mut out,
        "Verified",
        response.status.verified_rules,
        response.status.total_rules,
        response.status.verification_coverage_percent,
        "ok",
    );
    render_count_metric(
        &mut out,
        "Invalid refs",
        response.status.invalid_references,
        "bad",
    );
    render_count_metric(
        &mut out,
        "Stale refs",
        response.status.stale_references,
        "warn",
    );
    out.push_str("</div>");

    render_review_queues_html(&mut out, response);
    render_spec_view_html(&mut out, response);
    render_coverage_view_html(&mut out, response);
    render_sources_view_html(&mut out, response);
    out.push_str("</main></body></html>");
    out
}

fn render_metric(out: &mut String, label: &str, count: usize, total: usize, pct: f64, class: &str) {
    out.push_str(&format!(
        "<div class=\"metric\"><span class=\"muted\">{}</span><strong class=\"{}\">{}/{}</strong><span>{:.1}%</span></div>",
        html_escape(label),
        class,
        count,
        total,
        pct
    ));
}

fn render_count_metric(out: &mut String, label: &str, count: usize, class: &str) {
    out.push_str(&format!(
        "<div class=\"metric\"><span class=\"muted\">{}</span><strong class=\"{}\">{}</strong></div>",
        html_escape(label),
        class,
        count
    ));
}

fn render_review_queues_html(out: &mut String, response: &CoverageNavigationResponse) {
    let uncovered = response
        .coverage_rules
        .iter()
        .filter(|rule| !rule.implemented)
        .collect::<Vec<_>>();
    let untested = response
        .coverage_rules
        .iter()
        .filter(|rule| rule.implemented && !rule.verified)
        .collect::<Vec<_>>();
    let stale = response
        .coverage_rules
        .iter()
        .filter(|rule| rule.stale_refs > 0)
        .collect::<Vec<_>>();

    out.push_str("<section id=\"review\"><div class=\"section-head\"><h2>Review Queues</h2>");
    out.push_str("<div class=\"views\"><a class=\"pill\" href=\"uncovered.html\">Uncovered</a><a class=\"pill\" href=\"untested.html\">Untested</a><a class=\"pill\" href=\"stale.html\">Stale</a><a class=\"pill\" href=\"invalid.html\">Invalid</a></div></div>");
    out.push_str("<div class=\"queue-grid\">");
    render_rule_queue(out, "Uncovered", "bad", "uncovered.html", &uncovered);
    render_rule_queue(out, "Untested", "warn", "untested.html", &untested);
    render_rule_queue(out, "Stale", "warn", "stale.html", &stale);
    out.push_str("<div class=\"queue\"><h3><span>Invalid refs</span>");
    out.push_str(&format!(
        "<span class=\"badge {}\">{}</span>",
        if response.status.invalid_references == 0 {
            "ok"
        } else {
            "bad"
        },
        response.status.invalid_references
    ));
    out.push_str(
        "</h3><ul><li><a href=\"invalid.html\">Open invalid reference report</a></li></ul></div>",
    );
    out.push_str("</div></section>");
}

fn render_rule_queue(
    out: &mut String,
    title: &str,
    class: &str,
    href: &str,
    rules: &[&CoverageRuleSummary],
) {
    out.push_str("<div class=\"queue\"><h3>");
    out.push_str(&format!(
        "<span>{}</span><span class=\"badge {}\">{}</span>",
        html_escape(title),
        class,
        rules.len()
    ));
    out.push_str("</h3><ul>");
    if rules.is_empty() {
        out.push_str("<li class=\"muted\">No rules matched.</li>");
    } else {
        for rule in rules.iter().take(8) {
            out.push_str(&format!(
                "<li><a href=\"{}\"><code>{}</code></a></li>",
                html_escape(&rule_html_href(&rule.id)),
                html_escape(&rule.id)
            ));
        }
        if rules.len() > 8 {
            out.push_str(&format!(
                "<li><a href=\"{}\">{} more</a></li>",
                html_escape(href),
                rules.len() - 8
            ));
        }
    }
    out.push_str("</ul></div>");
}

fn render_spec_view_html(out: &mut String, response: &CoverageNavigationResponse) {
    out.push_str("<section id=\"spec\"><div class=\"section-head\"><h2>Spec View</h2><span class=\"muted\">Rules rendered in source order</span></div>");
    if response.spec_routes.is_empty() {
        out.push_str("<div class=\"empty\">No spec rules found.</div></section>");
        return;
    }
    for route in &response.spec_routes {
        out.push_str("<details class=\"route\" open><summary>");
        out.push_str(&format!("<span><code>{}</code>", html_escape(&route.route)));
        if !route.source_name.is_empty() {
            out.push_str(&format!(
                "<span class=\"route-source\">source <code>{}</code></span>",
                html_escape(&route.source_name)
            ));
        }
        out.push_str("</span>");
        out.push_str(&format!(
            "<span class=\"muted\">{} rules</span>",
            route.rules.len()
        ));
        out.push_str("</summary><div class=\"rule-list\">");
        for rule in &route.rules {
            render_rule_card_html(out, rule);
        }
        out.push_str("</div></details>");
    }
    out.push_str("</section>");
}

fn render_rule_card_html(out: &mut String, rule: &CoverageSpecRuleNav) {
    out.push_str(&format!(
        "<article class=\"rule-card {}\">",
        rule_state_class(rule.implemented, rule.verified, rule.stale_refs)
    ));
    out.push_str("<div class=\"rule-head\"><div>");
    out.push_str("<div class=\"rule-title\">");
    out.push_str(&format!(
        "<a href=\"{}\"><code>{}</code></a>",
        html_escape(&rule_html_href(&rule.id)),
        html_escape(&rule.id)
    ));
    out.push_str("<div class=\"badges\">");
    render_bool_badge(out, "impl", rule.implemented);
    render_bool_badge(out, "verify", rule.verified);
    if rule.stale_refs > 0 {
        out.push_str(&format!(
            "<span class=\"badge warn\">{} stale</span>",
            rule.stale_refs
        ));
    }
    out.push_str("</div></div>");
    out.push_str(&format!("<div class=\"rule-meta\">line {}", rule.line));
    if !rule.anchor_id.is_empty() {
        out.push_str(&format!(
            " - anchor <code>{}</code>",
            html_escape(&rule.anchor_id)
        ));
    }
    out.push_str("</div></div>");
    out.push_str(&format!(
        "<a class=\"pill\" href=\"{}\">Open source route</a>",
        html_escape(&rule.route_href)
    ));
    out.push_str("</div>");
    if rule.html.trim().is_empty() {
        out.push_str("<div class=\"rule-body empty\">No rendered definition body.</div>");
    } else {
        out.push_str("<div class=\"rule-body\">");
        out.push_str(&rule.html);
        out.push_str("</div>");
    }
    out.push_str("</article>");
}

fn render_coverage_view_html(out: &mut String, response: &CoverageNavigationResponse) {
    out.push_str("<section id=\"coverage\"><div class=\"section-head\"><h2>Coverage View</h2><span class=\"muted\">Rule reference counts</span></div>");
    if response.coverage_rules.is_empty() {
        out.push_str("<div class=\"empty\">No rules found.</div></section>");
        return;
    }
    out.push_str("<table><thead><tr><th>Rule</th><th>Status</th><th>Impl refs</th><th>Verify refs</th><th>Depends</th><th>Related</th><th>Stale</th></tr></thead><tbody>");
    for rule in &response.coverage_rules {
        out.push_str(&format!(
            "<tr><td><a href=\"{}\"><code>{}</code></a></td><td>",
            html_escape(&rule_html_href(&rule.id)),
            html_escape(&rule.id)
        ));
        render_summary_badges(out, rule);
        out.push_str(&format!(
            "</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            rule.impl_refs, rule.verify_refs, rule.depends_refs, rule.related_refs, rule.stale_refs
        ));
    }
    out.push_str("</tbody></table></section>");
}

fn render_sources_view_html(out: &mut String, response: &CoverageNavigationResponse) {
    out.push_str("<section id=\"sources\"><div class=\"section-head\"><h2>Sources View</h2><span class=\"muted\">Code files scanned from configured impl globs</span></div>");
    if response.source_files.is_empty() {
        out.push_str("<div class=\"empty\">No source files found.</div></section>");
        return;
    }
    out.push_str("<table><thead><tr><th>File</th><th>Rules</th><th>Refs</th><th>Impl</th><th>Verify</th><th>Stale</th><th>Invalid</th><th>Unmapped</th></tr></thead><tbody>");
    for file in &response.source_files {
        let rules = file
            .rules
            .iter()
            .map(|rule| format!("<code>{}</code>", html_escape(rule)))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            html_escape(&file.file),
            rules,
            file.total_references,
            file.impl_refs,
            file.verify_refs,
            file.stale_refs,
            file.invalid_refs,
            file.unmapped_units.len()
        ));
    }
    out.push_str("</tbody></table></section>");
}

fn rule_state_class(implemented: bool, verified: bool, stale_refs: usize) -> &'static str {
    if !implemented {
        "is-unimplemented"
    } else if stale_refs > 0 {
        "is-stale"
    } else if !verified {
        "is-unverified"
    } else {
        "is-covered"
    }
}

fn render_summary_badges(out: &mut String, rule: &CoverageRuleSummary) {
    out.push_str("<div class=\"badges\">");
    render_bool_badge(out, "impl", rule.implemented);
    render_bool_badge(out, "verify", rule.verified);
    if rule.stale_refs > 0 {
        out.push_str(&format!(
            "<span class=\"badge warn\">{} stale</span>",
            rule.stale_refs
        ));
    }
    out.push_str("</div>");
}

fn render_bool_badge(out: &mut String, label: &str, value: bool) {
    let (class, text) = if value { ("ok", "yes") } else { ("bad", "no") };
    out.push_str(&format!(
        "<span class=\"badge {}\">{} {}</span>",
        class,
        html_escape(label),
        text
    ));
}

fn render_markdown_html(title: &str, markdown: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{}</style></head><body><main><pre>{}</pre></main></body></html>",
        html_escape(title),
        COVERAGE_MARKDOWN_CSS,
        html_escape(markdown)
    )
}

fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn render_config_markdown(response: &CoverageConfigResponse) -> String {
    let mut out = String::new();
    out.push_str("# Coverage Config\n\n");
    out.push_str(&format!("Spec: `{}`\n\n", response.spec_name));
    if response.impls.is_empty() {
        out.push_str("No coverage implementations are configured.\n");
        return out;
    }
    for impl_ in &response.impls {
        out.push_str(&format!(
            "## `{}` / `{}`\n\n",
            impl_.source_name, impl_.impl_name
        ));
        out.push_str(&format!("- Mount: `{}`\n", impl_.mount));
        render_globs(&mut out, "Include", &impl_.include);
        render_globs(&mut out, "Exclude", &impl_.exclude);
        render_globs(&mut out, "Test include", &impl_.test_include);
        out.push('\n');
    }
    out
}

fn render_globs(out: &mut String, label: &str, globs: &[String]) {
    if globs.is_empty() {
        out.push_str(&format!("- {label}: none\n"));
        return;
    }
    out.push_str(&format!("- {label}:\n"));
    for glob in globs {
        out.push_str(&format!("  - `{glob}`\n"));
    }
}

fn render_rule_list_markdown(
    title: &str,
    description: &str,
    response: &CoverageRuleListResponse,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {title}\n\n"));
    out.push_str(&format!("Spec: `{}`\n\n", response.spec_name));
    out.push_str(description);
    out.push_str("\n\n");
    if response.rules.is_empty() {
        out.push_str("No rules matched.\n");
        return out;
    }
    out.push_str("| Rule | Impl refs | Verify refs | Stale refs |\n");
    out.push_str("| --- | ---: | ---: | ---: |\n");
    for rule in &response.rules {
        out.push_str(&format!(
            "| [`{}`]({}) | {} | {} | {} |\n",
            rule.id,
            rule_href(&rule.id),
            rule.impl_refs,
            rule.verify_refs,
            rule.stale_refs
        ));
    }
    out
}

fn render_unmapped_markdown(response: &CoverageUnmappedResponse) -> String {
    let mut out = String::new();
    out.push_str("# Unmapped Code Units\n\n");
    out.push_str(&format!("Spec: `{}`\n\n", response.spec_name));
    if response.units.is_empty() {
        out.push_str("No unmapped code units found.\n");
        return out;
    }
    out.push_str("| Unit | Kind | Location |\n");
    out.push_str("| --- | --- | --- |\n");
    for unit in &response.units {
        let name = unit.name.as_deref().unwrap_or("(anonymous)");
        out.push_str(&format!(
            "| `{}` | `{}` | `{}`:{} |\n",
            name, unit.kind, unit.file, unit.line
        ));
    }
    out
}

fn render_stale_markdown(response: &CoverageStaleListResponse) -> String {
    let mut out = String::new();
    out.push_str("# Stale References\n\n");
    out.push_str(&format!("Spec: `{}`\n\n", response.spec_name));
    if response.references.is_empty() {
        out.push_str("No stale references found.\n");
        return out;
    }
    out.push_str("| Current rule | Referenced rule | Location |\n");
    out.push_str("| --- | --- | --- |\n");
    for stale in &response.references {
        out.push_str(&format!(
            "| [`{}`]({}) | `{}` | `{}`:{} |\n",
            stale.current_rule_id,
            rule_href(&stale.current_rule_id),
            stale.reference.rule_id,
            stale.reference.file,
            stale.reference.line
        ));
    }
    out
}

fn render_references_markdown(
    title: &str,
    description: &str,
    response: &CoverageReferenceListResponse,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {title}\n\n"));
    out.push_str(&format!("Spec: `{}`\n\n", response.spec_name));
    out.push_str(description);
    out.push_str("\n\n");
    if response.references.is_empty() {
        out.push_str("No references matched.\n");
        return out;
    }
    out.push_str("| Referenced rule | Verb | Location |\n");
    out.push_str("| --- | --- | --- |\n");
    for reference in &response.references {
        out.push_str(&format!(
            "| `{}` | `{}` | `{}`:{} |\n",
            reference.rule_id, reference.verb, reference.file, reference.line
        ));
    }
    out
}

fn render_validation_markdown(response: &CoverageValidationResponse) -> String {
    let mut out = String::new();
    out.push_str("# Coverage Validation\n\n");
    out.push_str(&format!("Spec: `{}`\n\n", response.spec_name));
    out.push_str(&format!(
        "Result: **{}**\n\n",
        if response.passing {
            "passing"
        } else {
            "failing"
        }
    ));
    if let Some(threshold) = response.threshold {
        out.push_str(&format!(
            "Implementation threshold: `{}%` (actual `{:.1}%`)\n\n",
            threshold, response.status.implementation_coverage_percent
        ));
    }
    out.push_str(&format!(
        "- Invalid references: `{}`\n",
        response.status.invalid_references
    ));
    out.push_str(&format!(
        "- Stale references: `{}`\n",
        response.status.stale_references
    ));
    out.push_str(&format!(
        "- Test impl references: `{}`\n",
        response.status.test_impl_references
    ));
    out.push_str(&format!(
        "- Uncovered rules: `{}`\n",
        response.status.uncovered_rules
    ));
    out
}

fn render_rule_markdown(response: &CoverageRuleResponse) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Rule `{}`\n\n", response.id));
    out.push_str(&format!(
        "- Implemented: `{}`\n",
        if response.implemented { "yes" } else { "no" }
    ));
    out.push_str(&format!(
        "- Verified: `{}`\n",
        if response.verified { "yes" } else { "no" }
    ));
    out.push_str(&format!("- Stale refs: `{}`\n", response.stale_refs.len()));
    out.push('\n');

    render_rule_definitions(&mut out, &response.definitions);
    render_rule_refs(&mut out, "Implementation References", &response.impl_refs);
    render_rule_refs(&mut out, "Verification References", &response.verify_refs);
    render_rule_refs(&mut out, "Dependency References", &response.depends_refs);
    render_rule_refs(&mut out, "Related References", &response.related_refs);

    if !response.stale_refs.is_empty() {
        out.push_str("## Stale References\n\n");
        out.push_str("| Referenced rule | Location |\n");
        out.push_str("| --- | --- |\n");
        for stale in &response.stale_refs {
            out.push_str(&format!(
                "| `{}` | `{}`:{} |\n",
                stale.reference.rule_id, stale.reference.file, stale.reference.line
            ));
        }
        out.push('\n');
    }

    out
}

fn render_rule_definitions(out: &mut String, definitions: &[CoverageRuleDefinition]) {
    out.push_str("## Definitions\n\n");
    if definitions.is_empty() {
        out.push_str("None.\n\n");
        return;
    }
    for definition in definitions {
        out.push_str(&format!(
            "- Route: [`{}`]({}#{})\n",
            definition.route, definition.route, definition.anchor_id
        ));
        if !definition.source_name.is_empty() {
            out.push_str(&format!("- Source: `{}`\n", definition.source_name));
        }
        out.push_str(&format!("- Line: `{}`\n\n", definition.line));
        if definition.raw.is_empty() {
            out.push_str("_No definition body._\n\n");
        } else {
            out.push_str("```markdown\n");
            out.push_str(&definition.raw);
            if !definition.raw.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("```\n\n");
        }
    }
}

fn render_rule_refs(out: &mut String, title: &str, refs: &[CoverageReference]) {
    out.push_str(&format!("## {title}\n\n"));
    if refs.is_empty() {
        out.push_str("None.\n\n");
        return;
    }
    out.push_str("| Location | Verb |\n");
    out.push_str("| --- | --- |\n");
    for reference in refs {
        out.push_str(&format!(
            "| `{}`:{} | `{}` |\n",
            reference.file, reference.line, reference.verb
        ));
    }
    out.push('\n');
}

fn rule_href(id: &str) -> String {
    format!("rule/{}.md", percent_encode_path_segment(id))
}

fn rule_html_href(id: &str) -> String {
    format!("rule/{}.html", percent_encode_path_segment(id))
}

fn percent_encode_path_segment(input: &str) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' => {
                out.push(char::from(byte));
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
