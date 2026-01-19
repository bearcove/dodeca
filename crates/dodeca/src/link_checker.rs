//! Link checking for generated HTML
//!
//! Query-based: works directly with HTML content from SiteOutput,
//! no disk I/O needed. External links are cached by (url, date) in picante.
//!
//! External link checking is done via the linkcheck cell, which handles
//! per-domain rate limiting internally.

use crate::db::{Database, ExternalLinkStatus, HttpErrorDiagnostics};
use crate::queries::check_external_url;
use crate::types::Route;
use chrono::{Datelike, NaiveDate};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::Duration;
use tracing::info;

/// A broken link found during checking
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenLink {
    /// The route of the page containing the broken link
    pub source_route: Route,
    /// The href value that's broken
    pub href: String,
    /// Why it's broken
    pub reason: String,
    /// Is this an external link?
    pub is_external: bool,
    /// HTTP error diagnostics (for external links with HTTP errors)
    pub diagnostics: Option<HttpErrorDiagnostics>,
}

/// Results from link checking
#[derive(Debug, Default, Clone)]
pub struct LinkCheckResult {
    pub total_links: usize,
    pub internal_links: usize,
    pub external_links: usize,
    pub external_checked: usize,
    pub broken_links: Vec<BrokenLink>,
}

impl LinkCheckResult {
    pub fn is_ok(&self) -> bool {
        self.broken_links.is_empty()
    }

    pub fn internal_broken(&self) -> usize {
        self.broken_links.iter().filter(|l| !l.is_external).count()
    }

    pub fn external_broken(&self) -> usize {
        self.broken_links.iter().filter(|l| l.is_external).count()
    }
}

/// Regex to extract href attributes from anchor tags
static HREF_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a\s[^>]*href=["']([^"']+)["']"#).unwrap());

/// Regex to extract id attributes from any element
static ELEMENT_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\sid=["']([^"']+)["']"#).unwrap());

/// A page with its route and HTML content
pub struct Page<'a> {
    pub route: &'a Route,
    pub html: &'a str,
}

/// An extracted link with its source
#[derive(Debug, Clone)]
pub struct ExtractedLink {
    pub source_route: Route,
    pub href: String,
}

/// Extract all links from pages, categorized
pub fn extract_links<'a>(pages: impl Iterator<Item = Page<'a>>) -> ExtractedLinks {
    let mut result = ExtractedLinks::default();

    let pages: Vec<_> = pages.collect();
    result.known_routes = pages.iter().map(|p| p.route.as_str().to_string()).collect();

    for page in &pages {
        // Extract element IDs for fragment validation
        let mut element_ids = HashSet::new();
        for cap in ELEMENT_ID_REGEX.captures_iter(page.html) {
            element_ids.insert(cap[1].to_string());
        }
        result
            .element_ids
            .insert(page.route.as_str().to_string(), element_ids);

        // Extract links
        for cap in HREF_REGEX.captures_iter(page.html) {
            let href = &cap[1];
            result.total += 1;

            if href.starts_with("http://") || href.starts_with("https://") {
                result.external.push(ExtractedLink {
                    source_route: page.route.clone(),
                    href: href.to_string(),
                });
            } else if href.starts_with('#') {
                // Same-page anchor - validate against current page's headings
                result.internal.push(ExtractedLink {
                    source_route: page.route.clone(),
                    href: href.to_string(),
                });
            } else if href.starts_with("mailto:")
                || href.starts_with("tel:")
                || href.starts_with("javascript:")
            {
                // Skip special links
            } else {
                result.internal.push(ExtractedLink {
                    source_route: page.route.clone(),
                    href: href.to_string(),
                });
            }
        }
    }

    result
}

/// Extracted links from all pages
#[derive(Debug, Default)]
pub struct ExtractedLinks {
    pub total: usize,
    pub internal: Vec<ExtractedLink>,
    pub external: Vec<ExtractedLink>,
    pub known_routes: HashSet<String>,
    /// Heading IDs per route (for fragment validation)
    pub element_ids: HashMap<String, HashSet<String>>,
}

/// Check internal links only (fast, no network)
pub fn check_internal_links(extracted: &ExtractedLinks) -> LinkCheckResult {
    let mut result = LinkCheckResult {
        total_links: extracted.total,
        internal_links: extracted.internal.len(),
        external_links: extracted.external.len(),
        ..Default::default()
    };

    for link in &extracted.internal {
        if let Some(reason) = check_internal_link(
            &link.source_route,
            &link.href,
            &extracted.known_routes,
            &extracted.element_ids,
        ) {
            result.broken_links.push(BrokenLink {
                source_route: link.source_route.clone(),
                href: link.href.clone(),
                reason,
                is_external: false,
                diagnostics: None,
            });
        }
    }

    result
}

/// Default delay between requests to the same domain (in milliseconds)
const DEFAULT_RATE_LIMIT_MS: u64 = 1000;

/// Options for external link checking
#[derive(Debug, Clone)]
pub struct ExternalLinkOptions {
    /// Domains to skip checking (anti-bot policies, known flaky, etc.)
    pub skip_domains: HashSet<String>,
    /// Minimum delay between requests to the same domain
    pub rate_limit: Duration,
}

impl Default for ExternalLinkOptions {
    fn default() -> Self {
        Self {
            skip_domains: HashSet::new(),
            rate_limit: Duration::from_millis(DEFAULT_RATE_LIMIT_MS),
        }
    }
}

impl ExternalLinkOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn skip_domains(mut self, domains: impl IntoIterator<Item = impl Into<String>>) -> Self {
        for domain in domains {
            self.skip_domains.insert(domain.into());
        }
        self
    }

    /// Set the rate limit in milliseconds
    pub fn rate_limit_ms(mut self, ms: u64) -> Self {
        self.rate_limit = Duration::from_millis(ms);
        self
    }
}

/// Check external links using picante-cached queries.
/// Each URL is checked via a tracked query keyed by (url, day_bucket).
/// Same URL on same day = cache hit (instant). New day = cache miss = re-check.
/// URLs are checked in parallel for speed.
pub async fn check_external_links(
    db: &Database,
    extracted: &ExtractedLinks,
    date: NaiveDate,
    _options: &ExternalLinkOptions,
) -> (Vec<BrokenLink>, usize) {
    use futures_util::future::join_all;

    // Deduplicate URLs and track which links use each URL
    let mut unique_urls: HashMap<&str, Vec<&ExtractedLink>> = HashMap::new();
    for link in &extracted.external {
        unique_urls.entry(&link.href).or_default().push(link);
    }

    // Compute day bucket (YYYYMMDD as u32)
    let day_bucket = date.year() as u32 * 10000 + date.month() * 100 + date.day();

    info!(
        urls = unique_urls.len(),
        day_bucket, "Checking external links"
    );

    // Check all URLs in parallel via picante queries
    let urls: Vec<_> = unique_urls.keys().map(|s| (*s).to_string()).collect();
    let futures: Vec<_> = urls
        .iter()
        .map(|url| check_external_url(db, url.clone(), day_bucket))
        .collect();

    let results = join_all(futures).await;

    // Process results
    let mut broken = Vec::new();
    let checked_count = results.len();

    for (url, result) in urls.iter().zip(results.into_iter()) {
        let status = match result {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(url, error = ?e, "Failed to check URL");
                ExternalLinkStatus::Failed(format!("query error: {e}"))
            }
        };

        // Report if broken
        if !matches!(status, ExternalLinkStatus::Ok) {
            let (reason, diagnostics) = match &status {
                ExternalLinkStatus::Ok => unreachable!(),
                ExternalLinkStatus::HttpError { code, diagnostics } => {
                    (format!("HTTP {code}"), Some(diagnostics.clone()))
                }
                ExternalLinkStatus::Failed(msg) => (msg.clone(), None),
            };

            if let Some(links) = unique_urls.get(url.as_str()) {
                for link in links {
                    broken.push(BrokenLink {
                        source_route: link.source_route.clone(),
                        href: link.href.clone(),
                        reason: reason.clone(),
                        is_external: true,
                        diagnostics: diagnostics.clone(),
                    });
                }
            }
        }
    }

    (broken, checked_count)
}

/// Check all links (internal only, for backwards compatibility)
pub fn check_links<'a>(pages: impl Iterator<Item = Page<'a>>) -> LinkCheckResult {
    let extracted = extract_links(pages);
    check_internal_links(&extracted)
}

/// Check if an internal link is valid
/// Returns None if valid, Some(reason) if broken
fn check_internal_link(
    source_route: &Route,
    href: &str,
    known_routes: &HashSet<String>,
    element_ids: &HashMap<String, HashSet<String>>,
) -> Option<String> {
    // Split href into path and fragment
    let (path, fragment) = match href.find('#') {
        Some(idx) => (&href[..idx], Some(&href[idx + 1..])),
        None => (href, None),
    };

    // Empty path means same-page anchor
    if path.is_empty() {
        // Validate fragment against current page's headings
        if let Some(frag) = fragment
            && !frag.is_empty()
        {
            let source_key = source_route.as_str().to_string();
            if let Some(ids) = element_ids.get(&source_key)
                && !ids.contains(frag)
            {
                return Some(format!("anchor '#{frag}' not found on page"));
            }
        }
        return None;
    }

    // Resolve the target route
    let target_route = if path.starts_with('/') {
        // Absolute path
        normalize_route(path)
    } else {
        // Relative path - resolve from source route (add / before relative path)
        let base = source_route.as_str();
        normalize_route(&format!("{base}/{path}"))
    };

    // Check if route exists (try various forms)
    let route_exists = known_routes.contains(&target_route)
        || {
            let without_slash = target_route.trim_end_matches('/');
            !without_slash.is_empty()
                && without_slash != target_route
                && known_routes.contains(without_slash)
        }
        || {
            let with_slash = format!("{}/", target_route.trim_end_matches('/'));
            known_routes.contains(&with_slash)
        };

    if !route_exists {
        // Check for static files (e.g., /main.css, /favicon.ico)
        // These won't be in known_routes but are valid
        if is_likely_static_file(path) {
            return None;
        }
        return Some(format!("target '{target_route}' not found"));
    }

    // Route exists - now validate fragment if present
    if let Some(frag) = fragment
        && !frag.is_empty()
    {
        // Find the target route's element IDs (try with/without trailing slash)
        let target_ids = element_ids
            .get(&target_route)
            .or_else(|| element_ids.get(target_route.trim_end_matches('/')))
            .or_else(|| {
                let with_slash = format!("{}/", target_route.trim_end_matches('/'));
                element_ids.get(&with_slash)
            });

        if let Some(ids) = target_ids
            && !ids.contains(frag)
        {
            return Some(format!("anchor '#{frag}' not found on target page"));
        }
        // If we can't find heading IDs for the target, don't fail
        // (could be a static file or external page)
    }

    None
}

/// Check if a path looks like a static file
fn is_likely_static_file(path: &str) -> bool {
    let extensions = [
        ".css", ".js", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".woff", ".woff2", ".ttf",
        ".eot", ".pdf", ".zip", ".tar", ".gz",
    ];
    extensions.iter().any(|ext| path.ends_with(ext))
}

/// Normalize a route path (handle .. and ., ensure leading slash, no trailing slash except root)
fn normalize_route(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();

    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }

    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_route() {
        assert_eq!(normalize_route("/learn/page/"), "/learn/page");
        assert_eq!(normalize_route("/learn/page"), "/learn/page");
        assert_eq!(normalize_route("/learn/page/../"), "/learn");
        assert_eq!(normalize_route("/learn/./page/"), "/learn/page");
        assert_eq!(normalize_route("/learn/../extend/"), "/extend");
        assert_eq!(normalize_route("/"), "/");
    }

    #[test]
    fn test_check_links_finds_broken() {
        let root = Route::from_static("/");
        let exists = Route::from_static("/exists");
        let pages = vec![
            Page {
                route: &root,
                html: r#"<a href="/exists">ok</a> <a href="/missing">broken</a>"#,
            },
            Page {
                route: &exists,
                html: r#"<a href="/">back</a>"#,
            },
        ];

        let result = check_links(pages.into_iter());
        assert_eq!(result.total_links, 3);
        assert_eq!(result.broken_links.len(), 1);
        assert_eq!(result.broken_links[0].href, "/missing");
    }

    #[test]
    fn test_relative_links() {
        let learn = Route::from_static("/learn");
        let learn_page = Route::from_static("/learn/page");
        let extend = Route::from_static("/extend");
        let pages = vec![
            Page {
                route: &learn,
                html: r#"<a href="page">relative</a> <a href="../extend">up</a>"#,
            },
            Page {
                route: &learn_page,
                html: "",
            },
            Page {
                route: &extend,
                html: "",
            },
        ];

        let result = check_links(pages.into_iter());
        assert!(result.is_ok(), "broken: {:?}", result.broken_links);
    }

    #[test]
    fn test_skips_external_and_special() {
        let root = Route::from_static("/");
        let pages = vec![Page {
            route: &root,
            html: "<h2 id=\"anchor\">Anchor Section</h2>\
                   <a href=\"https://example.com\">external</a>\
                   <a href=\"#anchor\">anchor</a>\
                   <a href=\"mailto:x@y.z\">email</a>\
                   <a href=\"/style.css\">static</a>",
        }];

        let result = check_links(pages.into_iter());
        assert!(result.is_ok(), "broken: {:?}", result.broken_links);
        assert_eq!(result.external_links, 1);
    }

    #[test]
    fn test_extract_links() {
        let root = Route::from_static("/");
        let pages = vec![Page {
            route: &root,
            html: "<a href=\"https://example.com\">ext</a>\
                   <a href=\"/page/\">int</a>\
                   <a href=\"#anchor\">anchor</a>",
        }];

        let extracted = extract_links(pages.into_iter());
        assert_eq!(extracted.total, 3);
        assert_eq!(extracted.external.len(), 1);
        // Now same-page anchors are also internal (for validation)
        assert_eq!(extracted.internal.len(), 2);
    }

    #[test]
    fn test_hash_fragment_valid() {
        let page = Route::from_static("/page");
        let pages = vec![Page {
            route: &page,
            html: "<h2 id=\"section\">Section One</h2>\
                   <a href=\"#section\">link to section</a>",
        }];

        let result = check_links(pages.into_iter());
        assert!(result.is_ok(), "broken: {:?}", result.broken_links);
    }

    #[test]
    fn test_hash_fragment_invalid() {
        let page = Route::from_static("/page");
        let pages = vec![Page {
            route: &page,
            html: "<h2 id=\"section\">Section One</h2>\
                   <a href=\"#nonexistent\">link to missing section</a>",
        }];

        let result = check_links(pages.into_iter());
        assert_eq!(result.broken_links.len(), 1);
        assert!(result.broken_links[0].reason.contains("#nonexistent"));
    }

    #[test]
    fn test_cross_page_hash_fragment_valid() {
        let page1 = Route::from_static("/page1");
        let page2 = Route::from_static("/page2");
        let pages = vec![
            Page {
                route: &page1,
                html: "<a href=\"/page2#section\">link to page2 section</a>",
            },
            Page {
                route: &page2,
                html: "<h2 id=\"section\">Section</h2>",
            },
        ];

        let result = check_links(pages.into_iter());
        assert!(result.is_ok(), "broken: {:?}", result.broken_links);
    }

    #[test]
    fn test_cross_page_hash_fragment_invalid() {
        let page1 = Route::from_static("/page1");
        let page2 = Route::from_static("/page2");
        let pages = vec![
            Page {
                route: &page1,
                html: "<a href=\"/page2#missing\">link to missing section</a>",
            },
            Page {
                route: &page2,
                html: "<h2 id=\"section\">Section</h2>",
            },
        ];

        let result = check_links(pages.into_iter());
        assert_eq!(result.broken_links.len(), 1);
        assert!(result.broken_links[0].reason.contains("#missing"));
    }

    #[test]
    fn test_extract_element_ids() {
        let page = Route::from_static("/page");
        let pages = vec![Page {
            route: &page,
            html: "<h1 id=\"title\">Title</h1>\
                   <h2 id=\"intro\">Intro</h2>\
                   <h3 id=\"details\">Details</h3>\
                   <div id=\"r-rule.one\">Rule</div>\
                   <span id=\"custom-anchor\">Anchor</span>",
        }];

        let extracted = extract_links(pages.into_iter());
        let ids = extracted.element_ids.get("/page").unwrap();
        // Heading IDs
        assert!(ids.contains("title"));
        assert!(ids.contains("intro"));
        assert!(ids.contains("details"));
        // Non-heading element IDs
        assert!(ids.contains("r-rule.one"));
        assert!(ids.contains("custom-anchor"));
    }
}
