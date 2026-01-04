//! Frontmatter parsing for markdown documents.
//!
//! Supports both TOML (`+++`) and YAML (`---`) frontmatter formats.

use facet::Facet;
use facet_value::Value;

use crate::{Error, Result};

/// Parsed frontmatter from a markdown document.
#[derive(Debug, Clone, Default, Facet)]
pub struct Frontmatter {
    /// Document title
    #[facet(default)]
    pub title: String,

    /// Sort weight for ordering documents
    #[facet(default)]
    pub weight: i32,

    /// Document description
    #[facet(default)]
    pub description: Option<String>,

    /// Template to use for rendering (dodeca-specific, but included for compatibility)
    #[facet(default)]
    pub template: Option<String>,

    /// Additional custom fields
    #[facet(default)]
    pub extra: Value,
}

/// Type of frontmatter delimiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrontmatterFormat {
    /// TOML frontmatter delimited by `+++`
    #[default]
    Toml,
    /// YAML frontmatter delimited by `---`
    Yaml,
}

/// Result of stripping frontmatter from a document.
#[derive(Debug, Clone)]
pub struct StrippedFrontmatter<'a> {
    /// The raw frontmatter content (without delimiters)
    pub raw: Option<&'a str>,
    /// The markdown body after frontmatter
    pub body: &'a str,
    /// The detected format
    pub format: Option<FrontmatterFormat>,
}

/// Strip frontmatter from a markdown document without parsing it.
///
/// This is useful when you want to handle the raw frontmatter yourself.
///
/// # Example
///
/// ```
/// use bearmark::{strip_frontmatter, FrontmatterFormat};
///
/// let md = "+++\ntitle = \"Hello\"\n+++\n# Content";
/// let result = strip_frontmatter(md);
///
/// assert_eq!(result.raw, Some("title = \"Hello\""));
/// assert_eq!(result.body, "# Content");
/// assert_eq!(result.format, Some(FrontmatterFormat::Toml));
/// ```
pub fn strip_frontmatter(markdown: &str) -> StrippedFrontmatter<'_> {
    // Detect format from opening delimiter
    let (delimiter, format) = if markdown.starts_with("+++\n") || markdown.starts_with("+++\r\n") {
        ("+++", FrontmatterFormat::Toml)
    } else if markdown.starts_with("---\n") || markdown.starts_with("---\r\n") {
        ("---", FrontmatterFormat::Yaml)
    } else {
        // No frontmatter
        return StrippedFrontmatter {
            raw: None,
            body: markdown,
            format: None,
        };
    };

    // Calculate start offset (skip opening delimiter + newline)
    let start_offset = if markdown.starts_with(&format!("{}\r\n", delimiter)) {
        5 // delimiter (3) + \r\n (2)
    } else {
        4 // delimiter (3) + \n (1)
    };

    let search_area = &markdown[start_offset..];

    // Find closing delimiter on its own line
    let closing_patterns = [
        format!("\n{}\n", delimiter),
        format!("\n{}\r\n", delimiter),
        format!("\r\n{}\n", delimiter),
        format!("\r\n{}\r\n", delimiter),
    ];

    let mut best_match: Option<(usize, usize)> = None;

    for pattern in &closing_patterns {
        if let Some(pos) = search_area.find(pattern)
            && (best_match.is_none() || pos < best_match.unwrap().0)
        {
            best_match = Some((pos, pattern.len()));
        }
    }

    if let Some((pos, pattern_len)) = best_match {
        let raw = search_area[..pos].trim();
        let content_start = start_offset + pos + pattern_len;
        let body = &markdown[content_start..];

        StrippedFrontmatter {
            raw: Some(raw),
            body,
            format: Some(format),
        }
    } else {
        // Opening delimiter but no closing - treat as no frontmatter
        StrippedFrontmatter {
            raw: None,
            body: markdown,
            format: None,
        }
    }
}

/// Parse frontmatter from a markdown document.
///
/// Returns the parsed frontmatter and the remaining body.
///
/// # Example
///
/// ```
/// use bearmark::parse_frontmatter;
///
/// let md = "+++\ntitle = \"Hello\"\nweight = 10\n+++\n# Content";
/// let (frontmatter, body) = parse_frontmatter(md).unwrap();
///
/// assert_eq!(frontmatter.title, "Hello");
/// assert_eq!(frontmatter.weight, 10);
/// assert_eq!(body, "# Content");
/// ```
pub fn parse_frontmatter(markdown: &str) -> Result<(Frontmatter, &str)> {
    let stripped = strip_frontmatter(markdown);

    let frontmatter = match (stripped.raw, stripped.format) {
        (Some(raw), Some(FrontmatterFormat::Toml)) => facet_toml::from_str::<Frontmatter>(raw)
            .map_err(|e| Error::FrontmatterParse(format!("TOML: {}", e)))?,
        (Some(raw), Some(FrontmatterFormat::Yaml)) => facet_yaml::from_str::<Frontmatter>(raw)
            .map_err(|e| Error::FrontmatterParse(format!("YAML: {}", e)))?,
        _ => Frontmatter::default(),
    };

    Ok((frontmatter, stripped.body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_toml_frontmatter() {
        let md = "+++\ntitle = \"Hello\"\n+++\n# Content";
        let result = strip_frontmatter(md);

        assert_eq!(result.raw, Some("title = \"Hello\""));
        assert_eq!(result.body, "# Content");
        assert_eq!(result.format, Some(FrontmatterFormat::Toml));
    }

    #[test]
    fn test_strip_yaml_frontmatter() {
        let md = "---\ntitle: Hello\n---\n# Content";
        let result = strip_frontmatter(md);

        assert_eq!(result.raw, Some("title: Hello"));
        assert_eq!(result.body, "# Content");
        assert_eq!(result.format, Some(FrontmatterFormat::Yaml));
    }

    #[test]
    fn test_strip_no_frontmatter() {
        let md = "# Just Content";
        let result = strip_frontmatter(md);

        assert_eq!(result.raw, None);
        assert_eq!(result.body, "# Just Content");
        assert_eq!(result.format, None);
    }

    #[test]
    fn test_parse_toml_frontmatter() {
        let md = "+++\ntitle = \"My Doc\"\nweight = 42\n+++\n# Content";
        let (fm, body) = parse_frontmatter(md).unwrap();

        assert_eq!(fm.title, "My Doc");
        assert_eq!(fm.weight, 42);
        assert_eq!(body, "# Content");
    }

    #[test]
    fn test_parse_yaml_frontmatter() {
        let md = "---\ntitle: My Doc\nweight: 42\n---\n# Content";
        let (fm, body) = parse_frontmatter(md).unwrap();

        assert_eq!(fm.title, "My Doc");
        assert_eq!(fm.weight, 42);
        assert_eq!(body, "# Content");
    }

    #[test]
    fn test_parse_no_frontmatter() {
        let md = "# Just Content";
        let (fm, body) = parse_frontmatter(md).unwrap();

        assert_eq!(fm.title, "");
        assert_eq!(fm.weight, 0);
        assert_eq!(body, "# Just Content");
    }

    #[test]
    fn test_frontmatter_with_extra_fields() {
        let md = "+++\ntitle = \"Test\"\n\n[extra]\ncustom_field = \"value\"\n+++\n# Content";
        let (fm, _) = parse_frontmatter(md).unwrap();

        assert_eq!(fm.title, "Test");

        // extra field should be captured in the Value
        use facet_value::DestructuredRef;
        match fm.extra.destructure_ref() {
            DestructuredRef::Object(obj) => {
                let custom_field = obj.get("custom_field").expect("custom_field should exist");
                assert_eq!(custom_field.as_string().unwrap().as_str(), "value");
            }
            other => panic!("expected object, got {:?}", other),
        }
    }

    #[test]
    fn test_frontmatter_extra_with_integration_test_content() {
        // This is the exact content from the integration test
        let md = r#"+++
title = "Guide"
[extra]
sidebar = true
icon = "book"
custom_value = 42
+++

# Guide

This is the guide section.
"#;
        let (fm, body) = parse_frontmatter(md).unwrap();

        assert_eq!(fm.title, "Guide");
        assert!(body.contains("# Guide"));

        // extra field should contain all the [extra] fields
        use facet_value::DestructuredRef;
        match fm.extra.destructure_ref() {
            DestructuredRef::Object(obj) => {
                eprintln!(
                    "Extra fields: {:?}",
                    obj.iter().map(|(k, _)| k.to_string()).collect::<Vec<_>>()
                );

                let sidebar = obj.get("sidebar").expect("sidebar should exist");
                assert_eq!(sidebar.as_bool(), Some(true), "sidebar should be true");

                let icon = obj.get("icon").expect("icon should exist");
                assert_eq!(
                    icon.as_string().unwrap().as_str(),
                    "book",
                    "icon should be 'book'"
                );

                let custom_value = obj.get("custom_value").expect("custom_value should exist");
                assert_eq!(
                    custom_value.as_number().and_then(|n| n.to_i64()),
                    Some(42),
                    "custom_value should be 42"
                );
            }
            other => panic!("expected object for extra, got {:?}", other),
        }
    }

    #[test]
    fn test_frontmatter_extra_survives_postcard_roundtrip() {
        use facet_postcard;
        use facet_value::DestructuredRef;

        // Parse the same content as the integration test
        let md = r#"+++
title = "Guide"
[extra]
sidebar = true
icon = "book"
custom_value = 42
+++

# Guide
"#;
        let (fm, _) = parse_frontmatter(md).unwrap();

        // Print what we got before serialization
        eprintln!("Before postcard: extra = {:?}", fm.extra);

        // Serialize just the Frontmatter
        let bytes = facet_postcard::to_vec(&fm).expect("serialize Frontmatter");
        eprintln!("Serialized {} bytes", bytes.len());

        // Deserialize
        let fm2: Frontmatter = facet_postcard::from_slice(&bytes).expect("deserialize Frontmatter");

        // Print what we got after deserialization
        eprintln!("After postcard: extra = {:?}", fm2.extra);

        // Verify extra survived
        match fm2.extra.destructure_ref() {
            DestructuredRef::Object(obj) => {
                let keys: Vec<_> = obj.iter().map(|(k, _)| k.to_string()).collect();
                eprintln!("After roundtrip, extra has keys: {:?}", keys);

                let sidebar = obj.get("sidebar").expect("sidebar should exist");
                assert_eq!(sidebar.as_bool(), Some(true), "sidebar should be true");

                let icon = obj.get("icon").expect("icon should exist");
                assert_eq!(icon.as_string().unwrap().as_str(), "book");
            }
            other => panic!("expected object after roundtrip, got {:?}", other),
        }
    }
}
