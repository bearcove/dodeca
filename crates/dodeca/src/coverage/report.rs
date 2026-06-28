//! Coverage analysis and reporting

use super::lexer::{RefVerb, ReqReference, Reqs};
use super::rule_id::{RuleId, RuleIdMatch, classify_reference_for_rule};
use facet::Facet;
use std::collections::{HashMap, HashSet};

/// A reference that points at an older version of a known rule.
#[derive(Debug, Clone, Facet)]
pub struct StaleReference {
    /// Current rule definition that supersedes the reference.
    pub current_rule_id: RuleId,
    /// Reference found in code.
    pub reference: ReqReference,
}

/// A code unit without any nearby requirement reference.
#[derive(Debug, Clone, Facet)]
pub struct UnmappedCodeUnit {
    pub file: String,
    pub line: usize,
    pub end_line: usize,
    pub kind: String,
    pub name: Option<String>,
}

/// A requirement definition as extracted from markdown.
#[derive(Debug, Clone, Facet)]
pub struct RuleDefinition {
    pub id: RuleId,
    pub source_name: String,
    pub route: String,
    pub anchor_id: String,
    pub line: usize,
    pub raw: String,
    pub html: String,
}

/// One configured source implementation used for coverage scanning.
#[derive(Debug, Clone, Facet)]
pub struct CoverageConfigImpl {
    pub source_name: String,
    pub mount: String,
    pub impl_name: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub test_include: Vec<String>,
}

/// Coverage analysis results for a single spec
#[derive(Debug, Clone, Facet)]
pub struct CoverageReport {
    /// Name of the spec
    pub spec_name: String,

    /// Total number of rules in the spec
    pub total_rules: usize,

    /// Rules that are referenced at least once
    pub covered_rules: HashSet<RuleId>,

    /// Rules that have no references (orphaned)
    pub uncovered_rules: HashSet<RuleId>,

    /// References to rules that don't exist in the spec
    pub invalid_references: Vec<ReqReference>,

    /// References to older versions of known rules
    pub stale_references: Vec<StaleReference>,

    /// Implementation references found in files configured as test files.
    pub test_impl_references: Vec<ReqReference>,

    /// Code units with no requirement references.
    pub unmapped_units: Vec<UnmappedCodeUnit>,

    /// Rule definitions grouped by canonical rule ID.
    pub definitions_by_rule: HashMap<RuleId, Vec<RuleDefinition>>,

    /// Coverage configuration entries selected for this report.
    pub config_impls: Vec<CoverageConfigImpl>,

    /// All valid references, grouped by rule ID
    pub references_by_rule: HashMap<RuleId, Vec<ReqReference>>,

    /// References grouped by verb type, then by rule ID
    pub references_by_verb: HashMap<RefVerb, HashMap<RuleId, Vec<ReqReference>>>,
}

impl CoverageReport {
    /// Compute coverage from rules and a set of known rule IDs
    ///
    /// r[impl coverage.compute.covered+2]
    /// r[impl coverage.compute.uncovered]
    /// r[impl coverage.compute.invalid]
    /// r[impl validation.broken-refs]
    pub fn compute(
        spec_name: impl Into<String>,
        known_rule_ids: &HashSet<RuleId>,
        reqs: &Reqs,
    ) -> Self {
        let spec_name = spec_name.into();
        let mut covered_rules = HashSet::new();
        let mut invalid_references = Vec::new();
        let mut stale_references = Vec::new();
        let mut references_by_rule: HashMap<RuleId, Vec<ReqReference>> = HashMap::new();
        let mut references_by_verb: HashMap<RefVerb, HashMap<RuleId, Vec<ReqReference>>> =
            HashMap::new();

        for reference in &reqs.references {
            if known_rule_ids.contains(&reference.req_id) {
                covered_rules.insert(reference.req_id.clone());
                references_by_rule
                    .entry(reference.req_id.clone())
                    .or_default()
                    .push(reference.clone());

                // Also group by verb
                references_by_verb
                    .entry(reference.verb)
                    .or_default()
                    .entry(reference.req_id.clone())
                    .or_default()
                    .push(reference.clone());
            } else {
                let current_rule_id = known_rule_ids
                    .iter()
                    .filter(|rule_id| {
                        classify_reference_for_rule(rule_id, &reference.req_id)
                            == RuleIdMatch::Stale
                    })
                    .max_by_key(|rule_id| rule_id.version)
                    .cloned();

                if let Some(current_rule_id) = current_rule_id {
                    stale_references.push(StaleReference {
                        current_rule_id,
                        reference: reference.clone(),
                    });
                } else {
                    invalid_references.push(reference.clone());
                }
            }
        }

        let uncovered_rules: HashSet<RuleId> =
            known_rule_ids.difference(&covered_rules).cloned().collect();

        CoverageReport {
            spec_name,
            total_rules: known_rule_ids.len(),
            covered_rules,
            uncovered_rules,
            invalid_references,
            stale_references,
            test_impl_references: Vec::new(),
            unmapped_units: Vec::new(),
            definitions_by_rule: HashMap::new(),
            config_impls: Vec::new(),
            references_by_rule,
            references_by_verb,
        }
    }

    pub fn with_test_impl_references(mut self, references: Vec<ReqReference>) -> Self {
        self.test_impl_references = references;
        self
    }

    pub fn with_unmapped_units(mut self, units: Vec<UnmappedCodeUnit>) -> Self {
        self.unmapped_units = units;
        self
    }

    pub fn with_definitions(mut self, definitions: HashMap<RuleId, Vec<RuleDefinition>>) -> Self {
        self.definitions_by_rule = definitions;
        self
    }

    pub fn with_config_impls(mut self, impls: Vec<CoverageConfigImpl>) -> Self {
        self.config_impls = impls;
        self
    }

    /// Coverage percentage (0.0 - 100.0)
    ///
    /// r[impl coverage.compute.percentage]
    pub fn coverage_percent(&self) -> f64 {
        if self.total_rules == 0 {
            return 100.0;
        }
        (self.covered_rules.len() as f64 / self.total_rules as f64) * 100.0
    }

    /// Whether the coverage is "passing" (no invalid refs, >= threshold coverage)
    pub fn is_passing(&self, threshold: f64) -> bool {
        self.invalid_references.is_empty()
            && self.stale_references.is_empty()
            && self.test_impl_references.is_empty()
            && self.coverage_percent() >= threshold
    }
}
