//! Heading extraction and slug generation.

/// A heading extracted from the markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// The heading text (without the `#` prefix)
    pub title: String,
    /// The slug ID for linking (e.g., "my-heading")
    pub id: String,
    /// The heading level (1-6)
    pub level: u8,
    /// Line number where this heading appears (1-indexed)
    pub line: usize,
}

/// Generate a URL-safe slug from text.
///
/// Converts to lowercase, replaces non-alphanumeric characters (except dots)
/// with hyphens, and removes leading/trailing hyphens.
///
/// # Example
///
/// ```
/// use bearmark::slugify;
///
/// assert_eq!(slugify("Hello World"), "hello-world");
/// assert_eq!(slugify("My API (v2)"), "my-api-v2");
/// assert_eq!(slugify("  Spaced  Out  "), "spaced-out");
/// assert_eq!(slugify("i.have.dots"), "i.have.dots");
/// ```
pub fn slugify(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_was_hyphen = true; // Start true to skip leading hyphens

    for c in text.chars() {
        if c.is_alphanumeric() || c == '.' {
            result.push(c.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            result.push('-');
            last_was_hyphen = true;
        }
    }

    // Remove trailing hyphen
    if result.ends_with('-') {
        result.pop();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_simple() {
        assert_eq!(slugify("Hello World"), "hello-world");
    }

    #[test]
    fn test_slugify_special_chars() {
        assert_eq!(slugify("My API (v2)"), "my-api-v2");
    }

    #[test]
    fn test_slugify_numbers() {
        assert_eq!(slugify("Chapter 1: Introduction"), "chapter-1-introduction");
    }

    #[test]
    fn test_slugify_consecutive_spaces() {
        assert_eq!(slugify("a   b"), "a-b");
    }

    #[test]
    fn test_slugify_leading_trailing() {
        assert_eq!(slugify("  Hello  "), "hello");
    }

    #[test]
    fn test_slugify_unicode() {
        // Unicode alphanumeric chars should be preserved (lowercased)
        assert_eq!(slugify("Héllo Wörld"), "héllo-wörld");
    }

    #[test]
    fn test_slugify_empty() {
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("   "), "");
    }

    #[test]
    fn test_slugify_dots() {
        assert_eq!(slugify("i.have.dots"), "i.have.dots");
        assert_eq!(slugify("config.path.default"), "config.path.default");
    }
}
