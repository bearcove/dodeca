use facet::Facet;
use std::collections::{BTreeMap, BTreeSet};

use super::{CoverageReport, RefVerb, ReqReference, RuleId, StaleReference};

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
}

impl CoverageOutputFormat {
    pub fn mime(self) -> &'static str {
        match self {
            CoverageOutputFormat::Json => "application/json; charset=utf-8",
            CoverageOutputFormat::Markdown => "text/markdown; charset=utf-8",
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
            }
        }
        CoverageEndpoint::Status => match format {
            CoverageOutputFormat::Json => json(&status_response(report))?,
            CoverageOutputFormat::Markdown => render_status_markdown(report),
        },
        CoverageEndpoint::Config => {
            let response = config_response(report);
            match format {
                CoverageOutputFormat::Json => json(&response)?,
                CoverageOutputFormat::Markdown => render_config_markdown(&response),
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
            }
        }
        CoverageEndpoint::Rule { id } => {
            let Some(response) = rule_response(report, &id) else {
                return Ok(None);
            };
            match format {
                CoverageOutputFormat::Json => json(&response)?,
                CoverageOutputFormat::Markdown => render_rule_markdown(&response),
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
