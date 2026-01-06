//! Requirement definition extraction for specification traceability.
//!
//! Supports the `r[req.id]` syntax used by mdbook-spec and similar tools.
//!
//! # Requirement Syntax
//!
//! Requirements are defined using the `r[req.id]` syntax on their own line:
//!
//! ```markdown
//! r[channel.id.allocation]
//! Channel IDs MUST be allocated sequentially starting from 0.
//! ```
//!
//! Requirements can also include metadata attributes:
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
//! | `since`   | version string | When the requirement was introduced |
//! | `until`   | version string | When the requirement will be deprecated/removed |
//! | `tags`    | comma-separated | Custom tags for categorization |

use std::collections::HashSet;
use std::path::PathBuf;

use facet::Facet;
use pulldown_cmark::{Options, Parser};

use crate::handler::BoxedReqHandler;
use crate::{Error, Result};

/// Byte offset and length in source content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
pub struct SourceSpan {
    /// Byte offset from start of content
    pub offset: usize,
    /// Length in bytes
    pub length: usize,
}

/// RFC 2119 keyword found in requirement text.
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

/// Lifecycle status of a requirement.
///
/// Requirements progress through these states as the specification evolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Facet)]
#[repr(u8)]
pub enum ReqStatus {
    /// Requirement is proposed but not yet finalized
    Draft,
    /// Requirement is active and enforced
    #[default]
    Stable,
    /// Requirement is being phased out
    Deprecated,
    /// Requirement has been removed (kept for historical reference)
    Removed,
}

impl ReqStatus {
    /// Parse a status from its string representation.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(ReqStatus::Draft),
            "stable" => Some(ReqStatus::Stable),
            "deprecated" => Some(ReqStatus::Deprecated),
            "removed" => Some(ReqStatus::Removed),
            _ => None,
        }
    }

    /// Get the string representation of this status.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReqStatus::Draft => "draft",
            ReqStatus::Stable => "stable",
            ReqStatus::Deprecated => "deprecated",
            ReqStatus::Removed => "removed",
        }
    }
}

impl std::fmt::Display for ReqStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// RFC 2119 requirement level for a requirement.
///
/// See <https://www.ietf.org/rfc/rfc2119.txt> for the specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Facet)]
#[repr(u8)]
pub enum ReqLevel {
    /// Absolute requirement (MUST, SHALL, REQUIRED)
    #[default]
    Must,
    /// Recommended but not required (SHOULD, RECOMMENDED)
    Should,
    /// Truly optional (MAY, OPTIONAL)
    May,
}

impl ReqLevel {
    /// Parse a level from its string representation.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "must" | "shall" | "required" => Some(ReqLevel::Must),
            "should" | "recommended" => Some(ReqLevel::Should),
            "may" | "optional" => Some(ReqLevel::May),
            _ => None,
        }
    }

    /// Get the string representation of this level.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReqLevel::Must => "must",
            ReqLevel::Should => "should",
            ReqLevel::May => "may",
        }
    }
}

impl std::fmt::Display for ReqLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Metadata attributes for a requirement.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct ReqMetadata {
    /// Lifecycle status (draft, stable, deprecated, removed)
    pub status: Option<ReqStatus>,
    /// RFC 2119 requirement level (must, should, may)
    pub level: Option<ReqLevel>,
    /// Version when this requirement was introduced
    pub since: Option<String>,
    /// Version when this requirement will be/was deprecated or removed
    pub until: Option<String>,
    /// Custom tags for categorization
    pub tags: Vec<String>,
}

impl ReqMetadata {
    /// Returns true if this requirement should be counted in coverage by default.
    ///
    /// Draft and removed requirements are excluded from coverage by default.
    pub fn counts_for_coverage(&self) -> bool {
        !matches!(
            self.status,
            Some(ReqStatus::Draft) | Some(ReqStatus::Removed)
        )
    }

    /// Returns true if this requirement is required (must be covered for passing builds).
    ///
    /// Only `must` level requirements are required; `should` and `may` are optional.
    pub fn is_required(&self) -> bool {
        match self.level {
            Some(ReqLevel::Must) | None => true,
            Some(ReqLevel::Should) | Some(ReqLevel::May) => false,
        }
    }
}

/// A requirement definition extracted from the markdown.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ReqDefinition {
    /// The requirement identifier (e.g., "channel.id.allocation")
    pub id: String,
    /// The anchor ID for HTML linking (e.g., "r--channel.id.allocation")
    pub anchor_id: String,
    /// Source location of this requirement in the original markdown
    pub span: SourceSpan,
    /// Line number where this requirement is defined (1-indexed)
    pub line: usize,
    /// Requirement metadata (status, level, since, until, tags)
    pub metadata: ReqMetadata,
    /// The rendered HTML of the content following the requirement marker
    pub html: String,
}

/// Warning about requirement quality.
#[derive(Debug, Clone, Facet)]
pub struct ReqWarning {
    /// File where the warning occurred
    pub file: PathBuf,
    /// Requirement ID this warning relates to
    pub req_id: String,
    /// Line number (1-indexed)
    pub line: usize,
    /// Byte span of the requirement
    pub span: SourceSpan,
    /// What kind of warning
    pub kind: ReqWarningKind,
}

/// Types of requirement warnings.
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum ReqWarningKind {
    /// Requirement text contains no RFC 2119 keywords
    NoRfc2119Keyword,
    /// Requirement text contains a negative requirement (MUST NOT, SHALL NOT, etc.) — these are hard to test
    NegativeReq(Rfc2119Keyword),
}

/// Result of extracting requirements from markdown.
#[derive(Debug, Clone, Facet)]
pub struct ExtractedReqs {
    /// Transformed markdown with requirement markers replaced by HTML
    pub output: String,
    /// All requirements found in the document
    pub reqs: Vec<ReqDefinition>,
    /// Warnings about requirement quality
    pub warnings: Vec<ReqWarning>,
}

/// Extract and transform requirement definitions in markdown.
///
/// Requirements are lines matching `r[req.id]` or `r[req.id attr=value ...]` at the start of a line.
/// They are replaced with HTML anchor divs for linking.
///
/// # Arguments
/// * `content` - The markdown content to process
/// * `req_handler` - Optional custom handler for rendering requirements. If None, uses default rendering.
/// * `source_path` - Optional source file path for warnings
///
/// Returns the transformed content, list of extracted requirements, and any warnings.
#[allow(dead_code)]
pub(crate) async fn extract_reqs(
    content: &str,
    req_handler: Option<&BoxedReqHandler>,
) -> Result<(String, Vec<ReqDefinition>)> {
    let result = extract_reqs_with_warnings(content, req_handler, None).await?;
    Ok((result.output, result.reqs))
}

/// Extract requirements with full warning support.
///
/// This is the full version that also returns warnings about requirement quality.
pub async fn extract_reqs_with_warnings(
    content: &str,
    req_handler: Option<&BoxedReqHandler>,
    source_path: Option<&std::path::Path>,
) -> Result<ExtractedReqs> {
    let mut output = String::with_capacity(content.len());
    let mut reqs = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    let file_path = source_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("<unknown>"));

    // Collect lines for lookahead (to extract requirement text)
    let lines: Vec<&str> = content.lines().collect();
    let mut byte_offset = 0usize;
    let mut skip_until_line = 0usize; // Track lines consumed by requirements

    for (line_idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_byte_len = line.len();

        // Skip lines that were consumed by a previous requirement's paragraph
        if line_idx < skip_until_line {
            byte_offset += line_byte_len + 1;
            continue;
        }

        // Check for requirement marker: r[req.id] or r[req.id attrs...] on its own line
        // r[impl markdown.syntax.marker] - requirement definition written as r[req.id]
        // r[impl markdown.syntax.standalone] - must appear on its own line (after trimming whitespace)
        // r[impl markdown.syntax.inline-ignored] - inline markers aren't matched since we check the whole trimmed line
        if trimmed.starts_with("r[") && trimmed.ends_with(']') && trimmed.len() > 3 {
            let inner = &trimmed[2..trimmed.len() - 1];

            // Parse requirement ID and optional attributes
            let (req_id, metadata) = match parse_req_marker(inner) {
                Ok(result) => result,
                Err(_) => {
                    // Not a valid requirement marker, keep as-is
                    output.push_str(line);
                    output.push('\n');
                    byte_offset += line_byte_len + 1;
                    continue;
                }
            };

            // Validate requirement ID
            if !is_valid_req_id(req_id) {
                output.push_str(line);
                output.push('\n');
                byte_offset += line_byte_len + 1;
                continue;
            }

            // Check for duplicates
            if !seen_ids.insert(req_id.to_string()) {
                return Err(Error::DuplicateReq(req_id.to_string()));
            }

            let anchor_id = format!("r--{}", req_id);

            // Extract requirement text (paragraph after requirement marker)
            let (text, lines_consumed, content_bytes) = extract_req_text(&lines[line_idx + 1..]);

            // Calculate span including marker line + content lines
            let span = SourceSpan {
                offset: byte_offset,
                length: line_byte_len + content_bytes,
            };

            // Mark lines as consumed so we skip them in the main loop
            skip_until_line = line_idx + 1 + lines_consumed;

            // Render the paragraph text to HTML
            let html = if text.is_empty() {
                String::new()
            } else {
                let parser = Parser::new_ext(&text, Options::empty());
                let mut html_out = String::new();
                pulldown_cmark::html::push_html(&mut html_out, parser);
                html_out
            };

            // Check for RFC 2119 keywords and emit warnings
            let keywords = detect_rfc2119_keywords(&text);

            if keywords.is_empty() && !text.is_empty() {
                warnings.push(ReqWarning {
                    file: file_path.clone(),
                    req_id: req_id.to_string(),
                    line: line_idx + 1,
                    span,
                    kind: ReqWarningKind::NoRfc2119Keyword,
                });
            }

            for keyword in &keywords {
                if keyword.is_negative() {
                    warnings.push(ReqWarning {
                        file: file_path.clone(),
                        req_id: req_id.to_string(),
                        line: line_idx + 1,
                        span,
                        kind: ReqWarningKind::NegativeReq(*keyword),
                    });
                }
            }

            let req = ReqDefinition {
                id: req_id.to_string(),
                anchor_id: anchor_id.clone(),
                span,
                line: line_idx + 1, // 1-indexed
                metadata,
                html,
            };

            // Render the requirement using the handler or default
            let rendered = if let Some(handler) = req_handler {
                let start = handler.start(&req).await?;
                let end = handler.end(&req).await?;
                format!("{}{}{}", start, req.html, end)
            } else {
                let start = default_req_html_start(&req);
                let end = default_req_html_end();
                format!("{}{}{}", start, req.html, end)
            };

            reqs.push(req);

            output.push_str(&rendered);
            output.push_str("\n\n"); // Ensure following text becomes a paragraph
        } else {
            output.push_str(line);
            output.push('\n');
        }

        byte_offset += line_byte_len + 1;
    }

    Ok(ExtractedReqs {
        output,
        reqs,
        warnings,
    })
}

/// Extract the requirement text from lines following a requirement marker.
///
/// Collects text until we hit:
/// - A blank line
/// - Another requirement marker (r[...])
/// - A heading (# ...)
/// - End of content
///
/// Returns (text, lines_consumed, bytes_consumed) where:
/// - text: the concatenated requirement text
/// - lines_consumed: number of lines that should be skipped
/// - bytes_consumed: total bytes including newlines
fn extract_req_text(lines: &[&str]) -> (String, usize, usize) {
    let mut text_lines = Vec::new();
    let mut lines_consumed = 0;
    let mut bytes_consumed = 0;

    for line in lines {
        let trimmed = line.trim();

        // Stop at blank line
        if trimmed.is_empty() {
            break;
        }

        // Stop at another requirement marker
        if trimmed.starts_with("r[") && trimmed.ends_with(']') {
            break;
        }

        // Stop at headings
        if trimmed.starts_with('#') {
            break;
        }

        text_lines.push(trimmed);
        lines_consumed += 1;
        bytes_consumed += line.len() + 1; // +1 for newline
    }

    (text_lines.join(" "), lines_consumed, bytes_consumed)
}

/// Parse a requirement marker content (inside r[...]).
///
/// Supports formats:
/// - `req.id` - simple requirement ID
/// - `req.id status=stable level=must` - requirement ID with attributes
pub fn parse_req_marker(inner: &str) -> Result<(&str, ReqMetadata)> {
    let inner = inner.trim();

    // Find where the requirement ID ends (at first space or end of string)
    let (req_id, attrs_str) = match inner.find(' ') {
        Some(idx) => (&inner[..idx], inner[idx + 1..].trim()),
        None => (inner, ""),
    };

    if req_id.is_empty() {
        return Err(Error::DuplicateReq(
            "empty requirement identifier".to_string(),
        ));
    }

    // Parse attributes if present
    let mut metadata = ReqMetadata::default();

    if !attrs_str.is_empty() {
        for attr in attrs_str.split_whitespace() {
            if let Some((key, value)) = attr.split_once('=') {
                match key {
                    "status" => {
                        metadata.status = Some(ReqStatus::parse(value).ok_or_else(|| {
                            Error::CodeBlockHandler {
                                language: "req".to_string(),
                                message: format!(
                                    "invalid status '{}' for requirement '{}', expected: draft, stable, deprecated, removed",
                                    value, req_id
                                ),
                            }
                        })?);
                    }
                    "level" => {
                        metadata.level = Some(ReqLevel::parse(value).ok_or_else(|| {
                            Error::CodeBlockHandler {
                                language: "req".to_string(),
                                message: format!(
                                    "invalid level '{}' for requirement '{}', expected: must, should, may",
                                    value, req_id
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
                            language: "req".to_string(),
                            message: format!(
                                "unknown attribute '{}' for requirement '{}', expected: status, level, since, until, tags",
                                key, req_id
                            ),
                        });
                    }
                }
            } else {
                return Err(Error::CodeBlockHandler {
                    language: "req".to_string(),
                    message: format!(
                        "invalid attribute format '{}' for requirement '{}', expected: key=value",
                        attr, req_id
                    ),
                });
            }
        }
    }

    Ok((req_id, metadata))
}

/// Check if a requirement ID is valid.
///
/// Valid requirement IDs contain only alphanumeric characters, dots, hyphens, and underscores.
fn is_valid_req_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == '_')
}

/// Generate default opening HTML for a requirement.
pub fn default_req_html_start(req: &ReqDefinition) -> String {
    // Insert <wbr> after dots for better line breaking in narrow displays
    let display_id = req.id.replace('.', ".<wbr>");

    format!(
        "<div class=\"req\" id=\"{}\"><a class=\"req-link\" href=\"#{}\" title=\"{}\"><span>[{}]</span></a>",
        req.anchor_id, req.anchor_id, req.id, display_id
    )
}

/// Generate default closing HTML for a requirement.
pub fn default_req_html_end() -> &'static str {
    "</div>"
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify markdown.syntax.marker]
    // r[verify markdown.syntax.standalone]
    #[tokio::test]
    async fn test_extract_single_req() {
        let content = "# Heading\n\nr[my.req]\nThis is the requirement text.\n";
        let (output, reqs) = extract_reqs(content, None).await.unwrap();

        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].id, "my.req");
        assert_eq!(reqs[0].anchor_id, "r--my.req");
        assert_eq!(reqs[0].html, "<p>This is the requirement text.</p>\n");
        assert!(output.contains("id=\"r--my.req\""));
        // Requirement content should be included in the output (wrapped by req handler)
        assert!(output.contains("This is the requirement text."));
    }

    #[tokio::test]
    async fn test_extract_multiple_reqs() {
        let content = "r[first.req]\nText.\n\nr[second.req]\nMore text.\n";
        let (_, reqs) = extract_reqs(content, None).await.unwrap();

        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].id, "first.req");
        assert_eq!(reqs[1].id, "second.req");
    }

    // r[verify markdown.duplicates.same-file]
    #[tokio::test]
    async fn test_duplicate_req_error() {
        let content = "r[dup.req]\nFirst.\n\nr[dup.req]\nSecond.\n";
        let result = extract_reqs(content, None).await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::DuplicateReq(id) if id == "dup.req"));
    }

    // r[verify markdown.syntax.inline-ignored]
    #[tokio::test]
    async fn test_inline_req_ignored() {
        // Requirement marker inline within text should not be extracted
        let content = "This is r[inline.req] in text.\n";
        let (output, reqs) = extract_reqs(content, None).await.unwrap();

        assert!(reqs.is_empty());
        assert!(output.contains("r[inline.req]"));
    }

    #[tokio::test]
    async fn test_req_with_hyphens_underscores() {
        let content = "r[my-req_id.sub-part]\nText.\n";
        let (_, reqs) = extract_reqs(content, None).await.unwrap();

        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].id, "my-req_id.sub-part");
    }

    #[tokio::test]
    async fn test_wbr_insertion() {
        let content = "r[a.b.c]\n";
        let (output, _) = extract_reqs(content, None).await.unwrap();

        assert!(output.contains("[a.<wbr>b.<wbr>c]"));
    }

    #[tokio::test]
    async fn test_invalid_req_id_ignored() {
        // Requirement with invalid characters should be left as-is
        let content = "r[invalid req!]\n";
        let (output, reqs) = extract_reqs(content, None).await.unwrap();

        assert!(reqs.is_empty());
        assert!(output.contains("r[invalid req!]"));
    }

    #[tokio::test]
    async fn test_req_with_metadata() {
        let content = "r[test.req status=stable level=must since=1.0]\nThis MUST be done.\n";
        let result = extract_reqs_with_warnings(content, None, None)
            .await
            .unwrap();

        assert_eq!(result.reqs.len(), 1);
        assert_eq!(result.reqs[0].id, "test.req");
        assert_eq!(result.reqs[0].metadata.status, Some(ReqStatus::Stable));
        assert_eq!(result.reqs[0].metadata.level, Some(ReqLevel::Must));
        assert_eq!(result.reqs[0].metadata.since, Some("1.0".to_string()));
    }

    #[tokio::test]
    async fn test_req_with_tags() {
        let content = "r[test.req tags=foo,bar,baz]\nOptional.\n";
        let result = extract_reqs_with_warnings(content, None, None)
            .await
            .unwrap();

        assert_eq!(result.reqs[0].metadata.tags, vec!["foo", "bar", "baz"]);
    }

    #[tokio::test]
    async fn test_req_text_extraction() {
        let content = "r[multi.line]\nFirst line.\nSecond line.\n\nNext paragraph.\n";
        let result = extract_reqs_with_warnings(content, None, None)
            .await
            .unwrap();

        // Verify HTML rendering of requirement content
        assert_eq!(result.reqs[0].html, "<p>First line. Second line.</p>\n");
        // Verify the "Next paragraph" is still in the output (wasn't consumed)
        assert!(result.output.contains("Next paragraph"));
    }

    #[tokio::test]
    async fn test_req_html_with_formatting() {
        let content =
            "r[formatted.req]\nThis is **bold** and *italic* text with a `code` snippet.\n";
        let result = extract_reqs_with_warnings(content, None, None)
            .await
            .unwrap();

        assert_eq!(result.reqs.len(), 1);
        assert_eq!(result.reqs[0].id, "formatted.req");
        // Verify the HTML contains proper formatting
        assert!(result.reqs[0].html.contains("<strong>bold</strong>"));
        assert!(result.reqs[0].html.contains("<em>italic</em>"));
        assert!(result.reqs[0].html.contains("<code>code</code>"));
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
        let content = "r[missing.keyword]\nThis requirement has no RFC 2119 keyword.\n";
        let result = extract_reqs_with_warnings(content, None, None)
            .await
            .unwrap();

        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].req_id, "missing.keyword");
        assert!(matches!(
            result.warnings[0].kind,
            ReqWarningKind::NoRfc2119Keyword
        ));
    }

    #[tokio::test]
    async fn test_warning_negative_requirement() {
        let content = "r[negative.must.not]\nClients MUST NOT send invalid data.\n";
        let result = extract_reqs_with_warnings(content, None, None)
            .await
            .unwrap();

        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].req_id, "negative.must.not");
        assert!(matches!(
            result.warnings[0].kind,
            ReqWarningKind::NegativeReq(Rfc2119Keyword::MustNot)
        ));
    }

    #[tokio::test]
    async fn test_no_warning_for_positive_must() {
        let content = "r[positive.must]\nClients MUST validate input.\n";
        let result = extract_reqs_with_warnings(content, None, None)
            .await
            .unwrap();

        assert!(result.warnings.is_empty());
    }

    // Metadata coverage tests

    #[test]
    fn test_metadata_counts_for_coverage() {
        let mut meta = ReqMetadata::default();
        assert!(meta.counts_for_coverage()); // default is stable

        meta.status = Some(ReqStatus::Stable);
        assert!(meta.counts_for_coverage());

        meta.status = Some(ReqStatus::Deprecated);
        assert!(meta.counts_for_coverage());

        meta.status = Some(ReqStatus::Draft);
        assert!(!meta.counts_for_coverage());

        meta.status = Some(ReqStatus::Removed);
        assert!(!meta.counts_for_coverage());
    }

    #[test]
    fn test_metadata_is_required() {
        let mut meta = ReqMetadata::default();
        assert!(meta.is_required()); // default level is Must

        meta.level = Some(ReqLevel::Must);
        assert!(meta.is_required());

        meta.level = Some(ReqLevel::Should);
        assert!(!meta.is_required());

        meta.level = Some(ReqLevel::May);
        assert!(!meta.is_required());
    }
}
