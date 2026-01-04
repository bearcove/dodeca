//! Rule definition extraction for specification traceability.
//!
//! Supports the `r[rule.id]` syntax used by mdbook-spec and similar tools.
//!
//! # Rule Syntax
//!
//! Rules are defined using the `r[rule.id]` syntax on their own line:
//!
//! ```markdown
//! r[channel.id.allocation]
//! Channel IDs MUST be allocated sequentially starting from 0.
//! ```
//!
//! Rules can also include metadata attributes:
//!
//! ```markdown
//! r[channel.id.allocation status=stable level=must since=1.0]
//! Channel IDs MUST be allocated sequentially.
//!
//! r[experimental.feature status=draft]
//! This feature is under development.
//!
//! r[old.behavior status=deprecated until=3.0]
//! This behavior is deprecated and will be removed.
//!
//! r[optional.feature level=may tags=optional,experimental]
//! This feature is optional.
//! ```
//!
//! ## Supported Metadata Attributes
//!
//! | Attribute | Values | Description |
//! |-----------|--------|-------------|
//! | `status`  | `draft`, `stable`, `deprecated`, `removed` | Lifecycle stage |
//! | `level`   | `must`, `should`, `may` | RFC 2119 requirement level |
//! | `since`   | version string | When the rule was introduced |
//! | `until`   | version string | When the rule will be deprecated/removed |
//! | `tags`    | comma-separated | Custom tags for categorization |

use std::collections::HashSet;
use std::path::PathBuf;

use facet::Facet;
use pulldown_cmark::{Options, Parser};

use crate::handler::BoxedRuleHandler;
use crate::{Error, Result};

/// Byte offset and length in source content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
pub struct SourceSpan {
    /// Byte offset from start of content
    pub offset: usize,
    /// Length in bytes
    pub length: usize,
}

/// RFC 2119 keyword found in rule text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum Rfc2119Keyword {
    /// MUST, SHALL, REQUIRED
    Must,
    /// MUST NOT, SHALL NOT
    MustNot,
    /// SHOULD, RECOMMENDED
    Should,
    /// SHOULD NOT, NOT RECOMMENDED
    ShouldNot,
    /// MAY, OPTIONAL
    May,
}

impl Rfc2119Keyword {
    /// Returns true if this is a negative keyword (MUST NOT, SHOULD NOT).
    pub fn is_negative(&self) -> bool {
        matches!(self, Rfc2119Keyword::MustNot | Rfc2119Keyword::ShouldNot)
    }

    /// Human-readable name for this keyword.
    pub fn as_str(&self) -> &'static str {
        match self {
            Rfc2119Keyword::Must => "MUST",
            Rfc2119Keyword::MustNot => "MUST NOT",
            Rfc2119Keyword::Should => "SHOULD",
            Rfc2119Keyword::ShouldNot => "SHOULD NOT",
            Rfc2119Keyword::May => "MAY",
        }
    }
}

/// Detect RFC 2119 keywords in text.
///
/// Returns all keywords found, checking for negative forms first.
/// Keywords must be uppercase to match RFC 2119 conventions.
pub fn detect_rfc2119_keywords(text: &str) -> Vec<Rfc2119Keyword> {
    let mut keywords = Vec::new();
    let words: Vec<&str> = text.split_whitespace().collect();

    let mut i = 0;
    while i < words.len() {
        let word = words[i].trim_matches(|c: char| !c.is_alphanumeric());

        // Check for two-word negative forms
        if i + 1 < words.len() {
            let next_word = words[i + 1].trim_matches(|c: char| !c.is_alphanumeric());
            if (word == "MUST" || word == "SHALL") && next_word == "NOT" {
                keywords.push(Rfc2119Keyword::MustNot);
                i += 2;
                continue;
            }
            if word == "SHOULD" && next_word == "NOT" {
                keywords.push(Rfc2119Keyword::ShouldNot);
                i += 2;
                continue;
            }
            if word == "NOT" && next_word == "RECOMMENDED" {
                keywords.push(Rfc2119Keyword::ShouldNot);
                i += 2;
                continue;
            }
        }

        // Check single-word forms
        match word {
            "MUST" | "SHALL" | "REQUIRED" => keywords.push(Rfc2119Keyword::Must),
            "SHOULD" | "RECOMMENDED" => keywords.push(Rfc2119Keyword::Should),
            "MAY" | "OPTIONAL" => keywords.push(Rfc2119Keyword::May),
            _ => {}
        }
        i += 1;
    }

    keywords
}

/// Lifecycle status of a rule.
///
/// Rules progress through these states as the specification evolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Facet)]
#[repr(u8)]
pub enum RuleStatus {
    /// Rule is proposed but not yet finalized
    Draft,
    /// Rule is active and enforced
    #[default]
    Stable,
    /// Rule is being phased out
    Deprecated,
    /// Rule has been removed (kept for historical reference)
    Removed,
}

impl RuleStatus {
    /// Parse a status from its string representation.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(RuleStatus::Draft),
            "stable" => Some(RuleStatus::Stable),
            "deprecated" => Some(RuleStatus::Deprecated),
            "removed" => Some(RuleStatus::Removed),
            _ => None,
        }
    }

    /// Get the string representation of this status.
    pub fn as_str(&self) -> &'static str {
        match self {
            RuleStatus::Draft => "draft",
            RuleStatus::Stable => "stable",
            RuleStatus::Deprecated => "deprecated",
            RuleStatus::Removed => "removed",
        }
    }
}

impl std::fmt::Display for RuleStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// RFC 2119 requirement level for a rule.
///
/// See <https://www.ietf.org/rfc/rfc2119.txt> for the specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Facet)]
#[repr(u8)]
pub enum RequirementLevel {
    /// Absolute requirement (MUST, SHALL, REQUIRED)
    #[default]
    Must,
    /// Recommended but not required (SHOULD, RECOMMENDED)
    Should,
    /// Truly optional (MAY, OPTIONAL)
    May,
}

impl RequirementLevel {
    /// Parse a level from its string representation.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "must" | "shall" | "required" => Some(RequirementLevel::Must),
            "should" | "recommended" => Some(RequirementLevel::Should),
            "may" | "optional" => Some(RequirementLevel::May),
            _ => None,
        }
    }

    /// Get the string representation of this level.
    pub fn as_str(&self) -> &'static str {
        match self {
            RequirementLevel::Must => "must",
            RequirementLevel::Should => "should",
            RequirementLevel::May => "may",
        }
    }
}

impl std::fmt::Display for RequirementLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Metadata attributes for a rule.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct RuleMetadata {
    /// Lifecycle status (draft, stable, deprecated, removed)
    pub status: Option<RuleStatus>,
    /// RFC 2119 requirement level (must, should, may)
    pub level: Option<RequirementLevel>,
    /// Version when this rule was introduced
    pub since: Option<String>,
    /// Version when this rule will be/was deprecated or removed
    pub until: Option<String>,
    /// Custom tags for categorization
    pub tags: Vec<String>,
}

impl RuleMetadata {
    /// Returns true if this rule should be counted in coverage by default.
    ///
    /// Draft and removed rules are excluded from coverage by default.
    pub fn counts_for_coverage(&self) -> bool {
        !matches!(
            self.status,
            Some(RuleStatus::Draft) | Some(RuleStatus::Removed)
        )
    }

    /// Returns true if this rule is required (must be covered for passing builds).
    ///
    /// Only `must` level rules are required; `should` and `may` are optional.
    pub fn is_required(&self) -> bool {
        match self.level {
            Some(RequirementLevel::Must) | None => true,
            Some(RequirementLevel::Should) | Some(RequirementLevel::May) => false,
        }
    }
}

/// A rule definition extracted from the markdown.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct RuleDefinition {
    /// The rule identifier (e.g., "channel.id.allocation")
    pub id: String,
    /// The anchor ID for HTML linking (e.g., "r-channel.id.allocation")
    pub anchor_id: String,
    /// Source location of this rule in the original markdown
    pub span: SourceSpan,
    /// Line number where this rule is defined (1-indexed)
    pub line: usize,
    /// Rule metadata (status, level, since, until, tags)
    pub metadata: RuleMetadata,
    /// The markdown text content following the rule marker (first paragraph)
    pub text: String,
    /// The rendered HTML of the paragraph following the rule marker
    pub paragraph_html: String,
}

/// Warning about rule quality.
#[derive(Debug, Clone, Facet)]
pub struct RuleWarning {
    /// File where the warning occurred
    pub file: PathBuf,
    /// Rule ID this warning relates to
    pub rule_id: String,
    /// Line number (1-indexed)
    pub line: usize,
    /// Byte span of the rule
    pub span: SourceSpan,
    /// What kind of warning
    pub kind: RuleWarningKind,
}

/// Types of rule warnings.
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum RuleWarningKind {
    /// Rule text contains no RFC 2119 keywords
    NoRfc2119Keyword,
    /// Rule text contains a negative requirement (MUST NOT, SHALL NOT, etc.) — these are hard to test
    NegativeRequirement(Rfc2119Keyword),
}

/// Result of extracting rules from markdown.
#[derive(Debug, Clone, Facet)]
pub struct ExtractedRules {
    /// Transformed markdown with rule markers replaced by HTML
    pub output: String,
    /// All rules found in the document
    pub rules: Vec<RuleDefinition>,
    /// Warnings about rule quality
    pub warnings: Vec<RuleWarning>,
}

/// Extract and transform rule definitions in markdown.
///
/// Rules are lines matching `r[rule.id]` or `r[rule.id attr=value ...]` at the start of a line.
/// They are replaced with HTML anchor divs for linking.
///
/// # Arguments
/// * `content` - The markdown content to process
/// * `rule_handler` - Optional custom handler for rendering rules. If None, uses default rendering.
/// * `source_path` - Optional source file path for warnings
///
/// Returns the transformed content, list of extracted rules, and any warnings.
pub(crate) async fn extract_rules(
    content: &str,
    rule_handler: Option<&BoxedRuleHandler>,
) -> Result<(String, Vec<RuleDefinition>)> {
    let result = extract_rules_with_warnings(content, rule_handler, None).await?;
    Ok((result.output, result.rules))
}

/// Extract rules with full warning support.
///
/// This is the full version that also returns warnings about rule quality.
pub async fn extract_rules_with_warnings(
    content: &str,
    rule_handler: Option<&BoxedRuleHandler>,
    source_path: Option<&std::path::Path>,
) -> Result<ExtractedRules> {
    let mut output = String::with_capacity(content.len());
    let mut rules = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    let file_path = source_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("<unknown>"));

    // Collect lines for lookahead (to extract rule text)
    let lines: Vec<&str> = content.lines().collect();
    let mut byte_offset = 0usize;
    let mut skip_until_line = 0usize; // Track lines consumed by rules

    for (line_idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_byte_len = line.len();

        // Skip lines that were consumed by a previous rule's paragraph
        if line_idx < skip_until_line {
            byte_offset += line_byte_len + 1;
            continue;
        }

        // Check for rule marker: r[rule.id] or r[rule.id attrs...] on its own line
        if trimmed.starts_with("r[") && trimmed.ends_with(']') && trimmed.len() > 3 {
            let inner = &trimmed[2..trimmed.len() - 1];

            // Parse rule ID and optional attributes
            let (rule_id, metadata) = match parse_rule_marker(inner) {
                Ok(result) => result,
                Err(_) => {
                    // Not a valid rule marker, keep as-is
                    output.push_str(line);
                    output.push('\n');
                    byte_offset += line_byte_len + 1;
                    continue;
                }
            };

            // Validate rule ID
            if !is_valid_rule_id(rule_id) {
                output.push_str(line);
                output.push('\n');
                byte_offset += line_byte_len + 1;
                continue;
            }

            // Check for duplicates
            if !seen_ids.insert(rule_id.to_string()) {
                return Err(Error::DuplicateRule(rule_id.to_string()));
            }

            let anchor_id = format!("r-{}", rule_id);

            // Calculate span
            let span = SourceSpan {
                offset: byte_offset,
                length: line_byte_len,
            };

            // Extract rule text (paragraph after rule marker)
            let (text, lines_consumed) = extract_rule_text(&lines[line_idx + 1..]);

            // Mark lines as consumed so we skip them in the main loop
            skip_until_line = line_idx + 1 + lines_consumed;

            // Render the paragraph text to HTML
            let paragraph_html = if text.is_empty() {
                String::new()
            } else {
                let parser = Parser::new_ext(&text, Options::empty());
                let mut html = String::new();
                pulldown_cmark::html::push_html(&mut html, parser);
                html
            };

            // Check for RFC 2119 keywords and emit warnings
            let keywords = detect_rfc2119_keywords(&text);

            if keywords.is_empty() && !text.is_empty() {
                warnings.push(RuleWarning {
                    file: file_path.clone(),
                    rule_id: rule_id.to_string(),
                    line: line_idx + 1,
                    span,
                    kind: RuleWarningKind::NoRfc2119Keyword,
                });
            }

            for keyword in &keywords {
                if keyword.is_negative() {
                    warnings.push(RuleWarning {
                        file: file_path.clone(),
                        rule_id: rule_id.to_string(),
                        line: line_idx + 1,
                        span,
                        kind: RuleWarningKind::NegativeRequirement(*keyword),
                    });
                }
            }

            let rule = RuleDefinition {
                id: rule_id.to_string(),
                anchor_id: anchor_id.clone(),
                span,
                line: line_idx + 1, // 1-indexed
                metadata,
                text,
                paragraph_html,
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

        byte_offset += line_byte_len + 1;
    }

    Ok(ExtractedRules {
        output,
        rules,
        warnings,
    })
}

/// Extract the rule text from lines following a rule marker.
///
/// Collects text until we hit:
/// - A blank line
/// - Another rule marker (r[...])
/// - A heading (# ...)
/// - End of content
///
/// Returns (text, lines_consumed) where lines_consumed is the number of lines
/// that should be skipped in the main loop.
fn extract_rule_text(lines: &[&str]) -> (String, usize) {
    let mut text_lines = Vec::new();
    let mut lines_consumed = 0;

    for line in lines {
        let trimmed = line.trim();

        // Stop at blank line
        if trimmed.is_empty() {
            break;
        }

        // Stop at another rule marker
        if trimmed.starts_with("r[") && trimmed.ends_with(']') {
            break;
        }

        // Stop at headings
        if trimmed.starts_with('#') {
            break;
        }

        text_lines.push(trimmed);
        lines_consumed += 1;
    }

    (text_lines.join(" "), lines_consumed)
}

/// Parse a rule marker content (inside r[...]).
///
/// Supports formats:
/// - `rule.id` - simple rule ID
/// - `rule.id status=stable level=must` - rule ID with attributes
fn parse_rule_marker(inner: &str) -> Result<(&str, RuleMetadata)> {
    let inner = inner.trim();

    // Find where the rule ID ends (at first space or end of string)
    let (rule_id, attrs_str) = match inner.find(' ') {
        Some(idx) => (&inner[..idx], inner[idx + 1..].trim()),
        None => (inner, ""),
    };

    if rule_id.is_empty() {
        return Err(Error::DuplicateRule("empty rule identifier".to_string()));
    }

    // Parse attributes if present
    let mut metadata = RuleMetadata::default();

    if !attrs_str.is_empty() {
        for attr in attrs_str.split_whitespace() {
            if let Some((key, value)) = attr.split_once('=') {
                match key {
                    "status" => {
                        metadata.status = Some(RuleStatus::parse(value).ok_or_else(|| {
                            Error::CodeBlockHandler {
                                language: "rule".to_string(),
                                message: format!(
                                    "invalid status '{}' for rule '{}', expected: draft, stable, deprecated, removed",
                                    value, rule_id
                                ),
                            }
                        })?);
                    }
                    "level" => {
                        metadata.level = Some(RequirementLevel::parse(value).ok_or_else(|| {
                            Error::CodeBlockHandler {
                                language: "rule".to_string(),
                                message: format!(
                                    "invalid level '{}' for rule '{}', expected: must, should, may",
                                    value, rule_id
                                ),
                            }
                        })?);
                    }
                    "since" => {
                        metadata.since = Some(value.to_string());
                    }
                    "until" => {
                        metadata.until = Some(value.to_string());
                    }
                    "tags" => {
                        metadata.tags = value.split(',').map(|s| s.trim().to_string()).collect();
                    }
                    _ => {
                        return Err(Error::CodeBlockHandler {
                            language: "rule".to_string(),
                            message: format!(
                                "unknown attribute '{}' for rule '{}', expected: status, level, since, until, tags",
                                key, rule_id
                            ),
                        });
                    }
                }
            } else {
                return Err(Error::CodeBlockHandler {
                    language: "rule".to_string(),
                    message: format!(
                        "invalid attribute format '{}' for rule '{}', expected: key=value",
                        attr, rule_id
                    ),
                });
            }
        }
    }

    Ok((rule_id, metadata))
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
        assert_eq!(rules[0].text, "This is the rule text.");
        assert_eq!(rules[0].paragraph_html, "<p>This is the rule text.</p>\n");
        assert!(output.contains("id=\"r-my.rule\""));
        // Verify the paragraph was consumed and not duplicated in output
        assert!(!output.contains("This is the rule text."));
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

    #[tokio::test]
    async fn test_rule_with_metadata() {
        let content = "r[test.rule status=stable level=must since=1.0]\nThis MUST be done.\n";
        let result = extract_rules_with_warnings(content, None, None)
            .await
            .unwrap();

        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].id, "test.rule");
        assert_eq!(result.rules[0].metadata.status, Some(RuleStatus::Stable));
        assert_eq!(result.rules[0].metadata.level, Some(RequirementLevel::Must));
        assert_eq!(result.rules[0].metadata.since, Some("1.0".to_string()));
    }

    #[tokio::test]
    async fn test_rule_with_tags() {
        let content = "r[test.rule tags=foo,bar,baz]\nOptional.\n";
        let result = extract_rules_with_warnings(content, None, None)
            .await
            .unwrap();

        assert_eq!(result.rules[0].metadata.tags, vec!["foo", "bar", "baz"]);
    }

    #[tokio::test]
    async fn test_rule_text_extraction() {
        let content = "r[multi.line]\nFirst line.\nSecond line.\n\nNext paragraph.\n";
        let result = extract_rules_with_warnings(content, None, None)
            .await
            .unwrap();

        assert_eq!(result.rules[0].text, "First line. Second line.");
        assert_eq!(
            result.rules[0].paragraph_html,
            "<p>First line. Second line.</p>\n"
        );
        // Verify the "Next paragraph" is still in the output (wasn't consumed)
        assert!(result.output.contains("Next paragraph"));
    }

    #[tokio::test]
    async fn test_rule_paragraph_html_with_formatting() {
        let content =
            "r[formatted.rule]\nThis is **bold** and *italic* text with a `code` snippet.\n";
        let result = extract_rules_with_warnings(content, None, None)
            .await
            .unwrap();

        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].id, "formatted.rule");
        // Verify the paragraph HTML contains proper formatting
        assert!(
            result.rules[0]
                .paragraph_html
                .contains("<strong>bold</strong>")
        );
        assert!(result.rules[0].paragraph_html.contains("<em>italic</em>"));
        assert!(result.rules[0].paragraph_html.contains("<code>code</code>"));
        // Verify the formatted text was consumed and not duplicated
        assert!(!result.output.contains("**bold**"));
    }

    // RFC 2119 keyword detection tests

    #[test]
    fn test_detect_rfc2119_must() {
        let keywords = detect_rfc2119_keywords("Channel IDs MUST be allocated sequentially.");
        assert_eq!(keywords, vec![Rfc2119Keyword::Must]);
    }

    #[test]
    fn test_detect_rfc2119_must_not() {
        let keywords = detect_rfc2119_keywords("Clients MUST NOT send invalid data.");
        assert_eq!(keywords, vec![Rfc2119Keyword::MustNot]);
    }

    #[test]
    fn test_detect_rfc2119_should() {
        let keywords = detect_rfc2119_keywords("Implementations SHOULD use TLS.");
        assert_eq!(keywords, vec![Rfc2119Keyword::Should]);
    }

    #[test]
    fn test_detect_rfc2119_should_not() {
        let keywords = detect_rfc2119_keywords("Clients SHOULD NOT retry immediately.");
        assert_eq!(keywords, vec![Rfc2119Keyword::ShouldNot]);
    }

    #[test]
    fn test_detect_rfc2119_may() {
        let keywords = detect_rfc2119_keywords("Implementations MAY cache responses.");
        assert_eq!(keywords, vec![Rfc2119Keyword::May]);
    }

    #[test]
    fn test_detect_rfc2119_multiple() {
        let keywords =
            detect_rfc2119_keywords("Clients MUST validate input and SHOULD log errors.");
        assert_eq!(keywords, vec![Rfc2119Keyword::Must, Rfc2119Keyword::Should]);
    }

    #[test]
    fn test_detect_rfc2119_case_sensitive() {
        // Only uppercase keywords should match per RFC 2119
        let keywords = detect_rfc2119_keywords("The server must respond.");
        assert!(keywords.is_empty());
    }

    #[tokio::test]
    async fn test_warning_no_keyword() {
        let content = "r[missing.keyword]\nThis rule has no RFC 2119 keyword.\n";
        let result = extract_rules_with_warnings(content, None, None)
            .await
            .unwrap();

        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].rule_id, "missing.keyword");
        assert!(matches!(
            result.warnings[0].kind,
            RuleWarningKind::NoRfc2119Keyword
        ));
    }

    #[tokio::test]
    async fn test_warning_negative_requirement() {
        let content = "r[negative.must.not]\nClients MUST NOT send invalid data.\n";
        let result = extract_rules_with_warnings(content, None, None)
            .await
            .unwrap();

        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].rule_id, "negative.must.not");
        assert!(matches!(
            result.warnings[0].kind,
            RuleWarningKind::NegativeRequirement(Rfc2119Keyword::MustNot)
        ));
    }

    #[tokio::test]
    async fn test_no_warning_for_positive_must() {
        let content = "r[positive.must]\nClients MUST validate input.\n";
        let result = extract_rules_with_warnings(content, None, None)
            .await
            .unwrap();

        assert!(result.warnings.is_empty());
    }

    // Metadata coverage tests

    #[test]
    fn test_metadata_counts_for_coverage() {
        let mut meta = RuleMetadata::default();
        assert!(meta.counts_for_coverage()); // default is stable

        meta.status = Some(RuleStatus::Stable);
        assert!(meta.counts_for_coverage());

        meta.status = Some(RuleStatus::Deprecated);
        assert!(meta.counts_for_coverage());

        meta.status = Some(RuleStatus::Draft);
        assert!(!meta.counts_for_coverage());

        meta.status = Some(RuleStatus::Removed);
        assert!(!meta.counts_for_coverage());
    }

    #[test]
    fn test_metadata_is_required() {
        let mut meta = RuleMetadata::default();
        assert!(meta.is_required()); // default level is Must

        meta.level = Some(RequirementLevel::Must);
        assert!(meta.is_required());

        meta.level = Some(RequirementLevel::Should);
        assert!(!meta.is_required());

        meta.level = Some(RequirementLevel::May);
        assert!(!meta.is_required());
    }
}
