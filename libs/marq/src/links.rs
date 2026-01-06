//! Internal link resolution for markdown.
//!
//! Handles `@/path` absolute links and relative `.md` link resolution.

use std::path::Path;

/// Resolve internal links (both `@/` absolute and relative `.md` links).
///
/// # Arguments
/// * `link` - The link URL to resolve
/// * `source_path` - The path of the source markdown file (for relative resolution)
///
/// # Returns
/// The resolved link URL.
pub fn resolve_link(link: &str, source_path: Option<&str>) -> String {
    // Handle absolute @/ links
    if let Some(path) = link.strip_prefix("@/") {
        return resolve_absolute_link(path);
    }

    // Handle relative .md links (only if we have a source path)
    // Check the path part (before fragment) for .md extension
    if let Some(source) = source_path {
        let path_part = link.split('#').next().unwrap_or(link);
        if path_part.ends_with(".md")
            && !link.starts_with("http://")
            && !link.starts_with("https://")
        {
            return resolve_relative_link(link, source);
        }
    }

    // Pass through all other links unchanged (external URLs, fragments, etc.)
    link.to_string()
}

/// Resolve `@/path/to/file.md` links to absolute URLs.
fn resolve_absolute_link(path: &str) -> String {
    // Split off fragment
    let (path_part, fragment) = match path.find('#') {
        Some(idx) => (&path[..idx], Some(&path[idx..])),
        None => (path, None),
    };

    let mut path = path_part.to_string();

    // Remove .md extension
    if path.ends_with(".md") {
        path = path[..path.len() - 3].to_string();
    }

    // Handle _index -> parent directory
    if path.ends_with("/_index") {
        path = path[..path.len() - 7].to_string();
    } else if path == "_index" {
        path = String::new();
    }

    // Ensure leading slash and trailing slash
    let result = if path.is_empty() {
        "/".to_string()
    } else {
        format!("/{}/", path)
    };

    // Append fragment if present
    match fragment {
        Some(f) => format!("{}{}", result, f),
        None => result,
    }
}

/// Resolve relative `.md` links based on current file location.
fn resolve_relative_link(link: &str, source_path: &str) -> String {
    // Split off fragment
    let (link_part, fragment) = match link.find('#') {
        Some(idx) => (&link[..idx], Some(&link[idx..])),
        None => (link, None),
    };

    // Get the directory of the source file
    let source = Path::new(source_path);
    let source_dir = source.parent().unwrap_or(Path::new(""));

    // Resolve the relative link against the source directory
    let resolved = source_dir.join(link_part);

    // Normalize the path (handle .. and .)
    let normalized = normalize_path(&resolved);

    // Convert to string
    let mut path = normalized.replace('\\', "/"); // Normalize Windows paths

    // Remove .md extension
    if path.ends_with(".md") {
        path = path[..path.len() - 3].to_string();
    }

    // Handle _index -> parent directory
    if path.ends_with("/_index") {
        path = path[..path.len() - 7].to_string();
    } else if path == "_index" {
        path = String::new();
    }

    // Ensure leading slash and trailing slash
    let result = if path.is_empty() {
        "/".to_string()
    } else if path.starts_with('/') {
        format!("{}/", path)
    } else {
        format!("/{}/", path)
    };

    // Append fragment if present
    match fragment {
        Some(f) => format!("{}{}", result, f),
        None => result,
    }
}

/// Normalize a path by resolving `.` and `..` components.
fn normalize_path(path: &Path) -> String {
    let mut components: Vec<&str> = Vec::new();

    for component in path.components() {
        match component {
            std::path::Component::Normal(s) => {
                if let Some(s) = s.to_str() {
                    components.push(s);
                }
            }
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {
                // Skip current directory markers
            }
            _ => {}
        }
    }

    components.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_absolute_link_simple() {
        assert_eq!(resolve_link("@/docs/intro.md", None), "/docs/intro/");
    }

    #[test]
    fn test_absolute_link_with_fragment() {
        assert_eq!(
            resolve_link("@/docs/intro.md#section", None),
            "/docs/intro/#section"
        );
    }

    #[test]
    fn test_absolute_link_index() {
        assert_eq!(resolve_link("@/_index.md", None), "/");
        assert_eq!(resolve_link("@/docs/_index.md", None), "/docs/");
    }

    #[test]
    fn test_absolute_link_no_extension() {
        assert_eq!(resolve_link("@/docs/intro", None), "/docs/intro/");
    }

    #[test]
    fn test_relative_link() {
        assert_eq!(
            resolve_link("sibling.md", Some("docs/page.md")),
            "/docs/sibling/"
        );
    }

    #[test]
    fn test_relative_link_with_fragment() {
        assert_eq!(
            resolve_link("sibling.md#section", Some("docs/page.md")),
            "/docs/sibling/#section"
        );
    }

    #[test]
    fn test_relative_link_parent_dir() {
        assert_eq!(
            resolve_link("../other.md", Some("docs/sub/page.md")),
            "/docs/other/"
        );
    }

    #[test]
    fn test_external_link_passthrough() {
        assert_eq!(
            resolve_link("https://example.com", None),
            "https://example.com"
        );
        assert_eq!(
            resolve_link("http://example.com/page.md", None),
            "http://example.com/page.md"
        );
    }

    #[test]
    fn test_fragment_only_passthrough() {
        assert_eq!(resolve_link("#section", None), "#section");
    }

    #[test]
    fn test_non_md_link_passthrough() {
        assert_eq!(resolve_link("image.png", Some("docs/page.md")), "image.png");
    }
}
