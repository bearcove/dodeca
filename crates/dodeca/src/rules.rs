//! Rule manifest generation for specification traceability.
//!
//! Collects all rule definitions from parsed markdown files and generates
//! a machine-readable manifest for tooling integration.

use crate::db::SiteTree;
use facet::Facet;
use std::collections::BTreeMap;

/// A rule entry in the manifest, with its target URL.
#[derive(Debug, Clone, Facet)]
pub struct RuleEntry {
    /// The full URL path including anchor (e.g., "/spec/core/#r-channel.id.allocation")
    pub url: String,
}

/// The rules manifest - maps rule IDs to their URLs.
#[derive(Debug, Clone, Facet)]
pub struct RulesManifest {
    /// Map from rule ID to rule entry
    pub rules: BTreeMap<String, RuleEntry>,
}

/// Collect all rules from the site tree and build the manifest.
///
/// Returns the manifest and a list of any duplicate rule IDs found
/// (duplicates across different files - within-file duplicates are
/// caught by the markdown cell).
pub fn collect_rules(site_tree: &SiteTree) -> (RulesManifest, Vec<DuplicateRule>) {
    let mut rules: BTreeMap<String, RuleEntry> = BTreeMap::new();
    let mut duplicates: Vec<DuplicateRule> = Vec::new();

    // Collect rules from sections
    for (route, section) in &site_tree.sections {
        for rule in &section.rules {
            let url = format!("{}#{}", route.as_str(), rule.anchor_id);
            if let Some(existing) = rules.get(&rule.id) {
                duplicates.push(DuplicateRule {
                    id: rule.id.clone(),
                    first_url: existing.url.clone(),
                    second_url: url,
                });
            } else {
                rules.insert(rule.id.clone(), RuleEntry { url });
            }
        }
    }

    // Collect rules from pages
    for (route, page) in &site_tree.pages {
        for rule in &page.rules {
            let url = format!("{}#{}", route.as_str(), rule.anchor_id);
            if let Some(existing) = rules.get(&rule.id) {
                duplicates.push(DuplicateRule {
                    id: rule.id.clone(),
                    first_url: existing.url.clone(),
                    second_url: url,
                });
            } else {
                rules.insert(rule.id.clone(), RuleEntry { url });
            }
        }
    }

    (RulesManifest { rules }, duplicates)
}

/// A duplicate rule ID found across different files.
#[derive(Debug, Clone)]
pub struct DuplicateRule {
    /// The rule ID that was duplicated
    pub id: String,
    /// URL where the rule was first defined
    pub first_url: String,
    /// URL where the duplicate was found
    pub second_url: String,
}

/// Serialize the rules manifest to pretty-printed JSON.
pub fn manifest_to_json(manifest: &RulesManifest) -> String {
    // This should never fail for our simple types
    facet_format_json::to_string(manifest).expect("failed to serialize rules manifest to JSON")
}

/// Generate an HTML redirect page for a rule.
///
/// This creates a simple HTML page with a meta refresh redirect,
/// suitable for static hosting where server-side redirects aren't available.
pub fn generate_redirect_html(rule_id: &str, target_url: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="refresh" content="0; url={target_url}">
<link rel="canonical" href="{target_url}">
<title>Redirecting to {rule_id}</title>
</head>
<body>
Redirecting to <a href="{target_url}">{rule_id}</a>...
</body>
</html>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_json_format() {
        let mut rules = BTreeMap::new();
        rules.insert(
            "channel.id.allocation".to_string(),
            RuleEntry {
                url: "/spec/core/#r-channel.id.allocation".to_string(),
            },
        );
        rules.insert(
            "channel.id.parity".to_string(),
            RuleEntry {
                url: "/spec/core/#r-channel.id.parity".to_string(),
            },
        );

        let manifest = RulesManifest { rules };
        let json = manifest_to_json(&manifest);

        // Should be valid JSON with pretty formatting
        assert!(json.contains("channel.id.allocation"));
        assert!(json.contains("/spec/core/#r-channel.id.allocation"));
    }
}
