//! Rule definition extraction for specification traceability.
//!
//! Supports the `r[rule.id]` syntax used by mdbook-spec and similar tools.

use std::collections::HashSet;

use crate::handler::BoxedRuleHandler;
use crate::{Error, Result};

/// A rule definition extracted from the markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDefinition {
    /// The rule identifier (e.g., "channel.id.allocation")
    pub id: String,
    /// The anchor ID for HTML linking (e.g., "r-channel.id.allocation")
    pub anchor_id: String,
}

/// Extract and transform rule definitions in markdown.
///
/// Rules are lines matching `r[rule.id]` at the start of a line.
/// They are replaced with HTML anchor divs for linking.
///
/// # Arguments
/// * `content` - The markdown content to process
/// * `rule_handler` - Optional custom handler for rendering rules. If None, uses default rendering.
///
/// Returns the transformed content and the list of extracted rules.
pub(crate) async fn extract_rules(
    content: &str,
    rule_handler: Option<&BoxedRuleHandler>,
) -> Result<(String, Vec<RuleDefinition>)> {
    let mut output = String::with_capacity(content.len());
    let mut rules = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Check for rule marker: r[rule.id] on its own line
        if trimmed.starts_with("r[") && trimmed.ends_with(']') && trimmed.len() > 3 {
            let rule_id = &trimmed[2..trimmed.len() - 1];

            // Validate rule ID (alphanumeric, dots, hyphens, underscores)
            if !is_valid_rule_id(rule_id) {
                // Not a valid rule, keep as-is
                output.push_str(line);
                output.push('\n');
                continue;
            }

            // Check for duplicates
            if !seen_ids.insert(rule_id.to_string()) {
                return Err(Error::DuplicateRule(rule_id.to_string()));
            }

            let anchor_id = format!("r-{}", rule_id);

            let rule = RuleDefinition {
                id: rule_id.to_string(),
                anchor_id: anchor_id.clone(),
            };

            // Render the rule using the handler or default
            let rendered = if let Some(handler) = rule_handler {
                handler.render(&rule).await?
            } else {
                default_rule_html(&rule)
            };

            rules.push(rule);

            output.push_str(&rendered);
            output.push_str("\n\n"); // Ensure following text becomes a paragraph
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    Ok((output, rules))
}

/// Check if a rule ID is valid.
///
/// Valid rule IDs contain only alphanumeric characters, dots, hyphens, and underscores.
fn is_valid_rule_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == '_')
}

/// Generate default HTML for a rule anchor.
pub(crate) fn default_rule_html(rule: &RuleDefinition) -> String {
    // Insert <wbr> after dots for better line breaking in narrow displays
    let display_id = rule.id.replace('.', ".<wbr>");

    format!(
        "<div class=\"rule\" id=\"{}\"><a class=\"rule-link\" href=\"#{}\" title=\"{}\"><span>[{}]</span></a></div>",
        rule.anchor_id, rule.anchor_id, rule.id, display_id
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_extract_single_rule() {
        let content = "# Heading\n\nr[my.rule]\nThis is the rule text.\n";
        let (output, rules) = extract_rules(content, None).await.unwrap();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "my.rule");
        assert_eq!(rules[0].anchor_id, "r-my.rule");
        assert!(output.contains("id=\"r-my.rule\""));
    }

    #[tokio::test]
    async fn test_extract_multiple_rules() {
        let content = "r[first.rule]\nText.\n\nr[second.rule]\nMore text.\n";
        let (_, rules) = extract_rules(content, None).await.unwrap();

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].id, "first.rule");
        assert_eq!(rules[1].id, "second.rule");
    }

    #[tokio::test]
    async fn test_duplicate_rule_error() {
        let content = "r[dup.rule]\nFirst.\n\nr[dup.rule]\nSecond.\n";
        let result = extract_rules(content, None).await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::DuplicateRule(id) if id == "dup.rule"));
    }

    #[tokio::test]
    async fn test_inline_rule_ignored() {
        // Rule marker inline within text should not be extracted
        let content = "This is r[inline.rule] in text.\n";
        let (output, rules) = extract_rules(content, None).await.unwrap();

        assert!(rules.is_empty());
        assert!(output.contains("r[inline.rule]"));
    }

    #[tokio::test]
    async fn test_rule_with_hyphens_underscores() {
        let content = "r[my-rule_id.sub-part]\nText.\n";
        let (_, rules) = extract_rules(content, None).await.unwrap();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "my-rule_id.sub-part");
    }

    #[tokio::test]
    async fn test_wbr_insertion() {
        let content = "r[a.b.c]\n";
        let (output, _) = extract_rules(content, None).await.unwrap();

        assert!(output.contains("[a.<wbr>b.<wbr>c]"));
    }

    #[tokio::test]
    async fn test_invalid_rule_id_ignored() {
        // Rule with invalid characters should be left as-is
        let content = "r[invalid rule!]\n";
        let (output, rules) = extract_rules(content, None).await.unwrap();

        assert!(rules.is_empty());
        assert!(output.contains("r[invalid rule!]"));
    }
}
