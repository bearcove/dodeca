//! Requirement definition extraction for specification traceability.
//!
//! Supports the req id syntax used by tracey, see <https://github.com/bearcove/tracey>

use std::path::PathBuf;

use facet::Facet;

use crate::{Error, Result};

/// Byte offset and length in source content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
pub struct SourceSpan {
    /// Byte offset from start of content
    pub offset: usize,
    /// Length in bytes
    pub length: usize,
}

/// Structured rule identifier with optional version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
pub struct RuleId {
    /// Base identifier without version suffix.
    pub base: String,
    /// Version number (unversioned IDs are version 1).
    pub version: u32,
}

impl std::fmt::Display for RuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.version == 1 {
            f.write_str(&self.base)
        } else {
            write!(f, "{}+{}", self.base, self.version)
        }
    }
}

impl PartialEq<&str> for RuleId {
    fn eq(&self, other: &&str) -> bool {
        self.to_string() == *other
    }
}

impl PartialEq<RuleId> for &str {
    fn eq(&self, other: &RuleId) -> bool {
        *self == other.to_string()
    }
}

/// Parse a rule ID with an optional `+N` version suffix.
pub fn parse_rule_id(id: &str) -> Option<RuleId> {
    if id.is_empty() {
        return None;
    }

    if let Some((base, version_str)) = id.rsplit_once('+') {
        if base.is_empty() || base.contains('+') || version_str.is_empty() {
            return None;
        }
        let version = version_str.parse::<u32>().ok()?;
        if version == 0 {
            return None;
        }
        Some(RuleId {
            base: base.to_string(),
            version,
        })
    } else if id.contains('+') {
        None
    } else {
        Some(RuleId {
            base: id.to_string(),
            version: 1,
        })
    }
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
    pub id: RuleId,
    /// The anchor ID for HTML linking (e.g., "r--channel.id.allocation")
    pub anchor_id: String,
    /// Source span of just the requirement marker (e.g., `r[` to `]`)
    /// Use this for inlay hints and diagnostics that should only highlight the marker.
    pub marker_span: SourceSpan,
    /// Source span of the entire requirement (marker + all content paragraphs).
    /// Use this for hover highlight ranges that should cover the full requirement.
    pub span: SourceSpan,
    /// Line number where this requirement is defined (1-indexed)
    pub line: usize,
    /// Requirement metadata (status, level, since, until, tags)
    pub metadata: ReqMetadata,
    /// Raw markdown source of the requirement content (without the `r[...]` marker).
    /// For blockquote rules, this includes the `> ` prefixes.
    /// Can be rendered with marq to get HTML.
    pub raw: String,
    /// The rendered HTML of the content following the requirement marker
    pub html: String,
}

/// Warning about requirement quality.
#[derive(Debug, Clone, Facet)]
pub struct ReqWarning {
    /// File where the warning occurred
    pub file: PathBuf,
    /// Requirement ID this warning relates to
    pub req_id: RuleId,
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

/// Parse a requirement marker content (inside r[...]).
///
/// Supports formats:
/// - `req.id` - simple requirement ID
/// - `req.id status=stable level=must` - requirement ID with attributes
pub fn parse_req_marker(inner: &str) -> Result<(RuleId, ReqMetadata)> {
    let inner = inner.trim();

    // Find where the requirement ID ends (at first space or end of string)
    let (req_id, attrs_str) = match inner.find(' ') {
        Some(idx) => (&inner[..idx], inner[idx + 1..].trim()),
        None => (inner, ""),
    };

    let req_id = parse_rule_id(req_id).ok_or_else(|| {
        Error::DuplicateReq("empty or invalid requirement identifier".to_string())
    })?;

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

#[cfg(test)]
mod tests {
    use super::*;

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
