//! Strongly-typed string wrappers using strid
//!
//! Instead of passing `String` and `&str` everywhere (stringly-typed code),
//! we use distinct types that make the code self-documenting and prevent
//! mixing up different kinds of strings at compile time.
//!
//! # Types provided
//!
//! - [`SourcePath`] / [`SourcePathRef`] - relative path to source file (e.g., "learn/_index.md")
//! - [`Route`] / [`RouteRef`] - URL route (e.g., "/learn/")
//! - [`Title`] / [`TitleRef`] - page/section title
//! - [`HtmlBody`] / [`HtmlBodyRef`] - rendered HTML content
//! - [`SourceContent`] / [`SourceContentRef`] - raw markdown file content

use strid::braid;

/// Relative path to a source file from the content directory.
/// Example: "learn/_index.md", "learn/showcases/json.md"
#[braid]
pub struct SourcePath {
    inner: String,
}

/// URL route path for a page or section.
/// Always starts with `/`, no trailing slash (except for root "/").
/// Example: "/learn", "/learn/showcases/json"
#[braid]
pub struct Route {
    inner: String,
}

/// Title of a page or section from frontmatter.
#[braid]
pub struct Title {
    inner: String,
}

/// Rendered HTML body content.
#[braid]
pub struct HtmlBody {
    inner: String,
}

/// Raw source file content (markdown with frontmatter).
#[braid]
pub struct SourceContent {
    inner: String,
}

/// Relative path to a template file from the templates directory.
/// Example: "base.html", "page.html"
#[braid]
pub struct TemplatePath {
    inner: String,
}

/// Raw template file content.
#[braid]
pub struct TemplateContent {
    inner: String,
}

/// Relative path to a Sass/SCSS file from the sass directory.
/// Example: "main.scss", "_variables.scss"
#[braid]
pub struct SassPath {
    inner: String,
}

/// Raw Sass/SCSS file content.
#[braid]
pub struct SassContent {
    inner: String,
}

/// Relative path to a static file from the static directory.
/// Example: "favicon.ico", "images/logo.png"
#[braid(serde)]
pub struct StaticPath {
    inner: String,
}

/// Relative path to a data file from the data directory.
/// Example: "versions.toml", "config.json", "meta.kdl"
#[braid]
pub struct DataPath {
    inner: String,
}

/// Raw data file content.
#[braid]
pub struct DataContent {
    inner: String,
}

impl Route {
    /// Create the root route "/"
    pub fn root() -> Self {
        Self::from_static("/")
    }

    /// Check if this route is within a section (contains the section name)
    pub fn is_in_section(&self, section: &str) -> bool {
        RouteRef::from_str(self.as_str()).is_in_section(section)
    }

    /// Get the parent route (e.g., "/learn/showcases/" -> "/learn/")
    pub fn parent(&self) -> Option<Route> {
        RouteRef::from_str(self.as_str()).parent()
    }
}

impl RouteRef {
    /// Check if this route is within a section (contains the section name)
    pub fn is_in_section(&self, section: &str) -> bool {
        // Match "/section" or "/section/" to handle both with and without trailing slash
        let s = self.as_str();
        s.contains(&format!("/{section}/")) || s.ends_with(&format!("/{section}"))
    }

    /// Check if this is an ancestor of another route
    pub fn is_ancestor_of(&self, other: &RouteRef) -> bool {
        other.as_str().starts_with(self.as_str())
    }

    /// Get the parent route (e.g., "/learn/showcases" -> "/learn")
    pub fn parent(&self) -> Option<Route> {
        let s = self.as_str().trim_end_matches('/');
        if s.is_empty() || s == "/" {
            return None;
        }
        match s.rfind('/') {
            Some(0) => Some(Route::root()),
            Some(idx) => Some(Route::new(s[..idx].to_string())),
            None => Some(Route::root()),
        }
    }
}

impl SourcePath {
    /// Check if this is a section index file (_index.md)
    pub fn is_section_index(&self) -> bool {
        self.as_str().ends_with("_index.md")
    }

    /// Convert source path to URL route.
    /// - "learn/_index.md" -> "/learn"
    /// - "learn/page.md" -> "/learn/page"
    /// - "_index.md" -> "/"
    pub fn to_route(&self) -> Route {
        let mut path = self.as_str().to_string();

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

        // Ensure leading slash, no trailing slash (except for root)
        if path.is_empty() {
            Route::root()
        } else {
            Route::new(format!("/{path}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_path_to_route() {
        assert_eq!(
            SourcePath::from_static("learn/_index.md").to_route(),
            Route::from_static("/learn")
        );
        assert_eq!(
            SourcePath::from_static("learn/showcases/_index.md").to_route(),
            Route::from_static("/learn/showcases")
        );
        assert_eq!(
            SourcePath::from_static("_index.md").to_route(),
            Route::root()
        );
        assert_eq!(
            SourcePath::from_static("learn/page.md").to_route(),
            Route::from_static("/learn/page")
        );
    }

    #[test]
    fn test_route_parent() {
        assert_eq!(
            Route::from_static("/learn/showcases").parent(),
            Some(Route::from_static("/learn"))
        );
        assert_eq!(Route::from_static("/learn").parent(), Some(Route::root()));
        assert_eq!(Route::root().parent(), None);
    }

    #[test]
    fn test_route_is_in_section() {
        let route = Route::from_static("/learn/showcases/json");
        assert!(route.is_in_section("learn"));
        assert!(route.is_in_section("showcases"));
        assert!(!route.is_in_section("extend"));
    }
}
