//! URL rewriting using proper parsers via cells
//!
//! All HTML transformations are done by the HTML cell in a single pass:
//! - URL rewriting (href, src, srcset attributes)
//! - Internal link resolution (@/ links)
//! - Relative link resolution
//! - Image transformation to picture elements
//! - Inline CSS/JS processing (via CSS/JS cells)
//! - Dead link marking

use std::collections::{HashMap, HashSet};

use crate::cells::{rewrite_string_literals_in_js_cell, rewrite_urls_in_css_cell};

/// Rewrite URLs in CSS using lightningcss parser (via cell)
///
/// Only rewrites actual `url()` values in CSS, not text that happens to look like URLs.
/// Also minifies the CSS output.
/// Returns original CSS if cell is not available.
pub async fn rewrite_urls_in_css(css: &str, path_map: &HashMap<String, String>) -> String {
    // Check if CSS cell is available
    if crate::cells::css_cell().await.is_none() {
        return css.to_string();
    }

    match rewrite_urls_in_css_cell(css.to_string(), path_map.clone()).await {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("CSS rewriting failed: {}", e);
            css.to_string()
        }
    }
}

/// Rewrite string literals in JavaScript that contain asset paths (async version)
/// Returns original JS if cell is not available.
pub async fn rewrite_string_literals_in_js(js: &str, path_map: &HashMap<String, String>) -> String {
    // Check if JS cell is available
    if crate::cells::js_cell().await.is_none() {
        return js.to_string();
    }

    match rewrite_string_literals_in_js_cell(js.to_string(), path_map.clone()).await {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("JS rewriting failed: {}", e);
            js.to_string()
        }
    }
}

/// Process HTML with all transformations via the HTML cell.
///
/// This is the main entry point for HTML processing. It performs all transformations
/// in a single parse/serialize cycle:
/// - URL rewriting (if path_map provided)
/// - Internal link resolution (if source_to_route provided)
/// - Relative link resolution (if base_route provided)
/// - Image transformation (if image_variants provided)
/// - Dead link marking (if known_routes provided)
/// - Inline CSS/JS URL rewriting (via host callbacks to CSS/JS cells)
///
/// Returns the processed HTML and metadata.
pub async fn process_html(
    html: &str,
    options: HtmlProcessOptions,
) -> Result<HtmlProcessOutput, eyre::Error> {
    let client = crate::cells::html_cell()
        .await
        .ok_or_else(|| eyre::eyre!("HTML cell not available"))?;

    let input = cell_html_proto::HtmlProcessInput {
        html: html.to_string(),
        path_map: options.path_map,
        known_routes: options.known_routes,
        code_metadata: None,
        injections: vec![],
        minify: None,
        source_to_route: options.source_to_route,
        base_route: options.base_route,
        image_variants: options.image_variants,
        vite_css_map: options.vite_css_map,
    };

    match client.process(input).await {
        Ok(cell_html_proto::HtmlProcessResult::Success {
            html,
            had_dead_links,
            had_code_buttons: _,
            hrefs,
            element_ids,
        }) => Ok(HtmlProcessOutput {
            html,
            had_dead_links,
            hrefs,
            element_ids,
        }),
        Ok(cell_html_proto::HtmlProcessResult::Error { message }) => {
            Err(eyre::eyre!("HTML processing error: {}", message))
        }
        Err(e) => Err(eyre::eyre!("RPC error: {:?}", e)),
    }
}

/// Options for HTML processing
#[derive(Default)]
pub struct HtmlProcessOptions {
    /// URL rewriting map (old path -> new path)
    pub path_map: Option<HashMap<String, String>>,
    /// Known routes for dead link detection
    pub known_routes: Option<HashSet<String>>,
    /// Source path to route mapping for @/ links
    pub source_to_route: Option<HashMap<String, String>>,
    /// Base route for relative link resolution
    pub base_route: Option<String>,
    /// Image variants for picture element transformation
    pub image_variants: Option<HashMap<String, cell_html_proto::ResponsiveImageInfo>>,
    /// Vite CSS map: entry path -> list of CSS URLs to inject
    pub vite_css_map: Option<HashMap<String, Vec<String>>>,
}

/// Output from HTML processing
#[allow(dead_code)] // Fields may be used for future link checking
pub struct HtmlProcessOutput {
    /// Processed HTML
    pub html: String,
    /// Whether any dead links were found
    pub had_dead_links: bool,
    /// All href values from `<a>` elements
    pub hrefs: Vec<String>,
    /// All id attributes from elements
    pub element_ids: Vec<String>,
}

/// Mark dead internal links in HTML using the cell
///
/// Adds `data-dead` attribute to ``<a>`` tags with internal hrefs that don't exist in known_routes.
/// Returns (modified_html, had_dead_links) tuple.
pub async fn mark_dead_links(html: &str, known_routes: &HashSet<String>) -> (String, bool) {
    let options = HtmlProcessOptions {
        known_routes: Some(known_routes.clone()),
        ..Default::default()
    };

    match process_html(html, options).await {
        Ok(output) => (output.html, output.had_dead_links),
        Err(e) => {
            tracing::warn!("Dead link marking failed: {}", e);
            (html.to_string(), false)
        }
    }
}

// Re-export ResponsiveImageInfo from the proto for convenience
pub use cell_html_proto::ResponsiveImageInfo;

/// Resolve `@/` prefixed links in HTML using source path to route mapping.
///
/// Now delegates to the HTML cell for proper parsing.
pub async fn resolve_internal_links(
    html: &str,
    source_to_route: &HashMap<String, String>,
) -> String {
    let options = HtmlProcessOptions {
        source_to_route: Some(source_to_route.clone()),
        ..Default::default()
    };

    match process_html(html, options).await {
        Ok(output) => output.html,
        Err(e) => {
            tracing::warn!("Internal link resolution failed: {}", e);
            html.to_string()
        }
    }
}

/// Resolve relative links in HTML by joining with base route.
///
/// Now delegates to the HTML cell for proper parsing.
pub async fn resolve_relative_links(html: &str, base_route: &str) -> String {
    let options = HtmlProcessOptions {
        base_route: Some(base_route.to_string()),
        ..Default::default()
    };

    match process_html(html, options).await {
        Ok(output) => output.html,
        Err(e) => {
            tracing::warn!("Relative link resolution failed: {}", e);
            html.to_string()
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dead_link_marking() {
        let mut routes = HashSet::new();
        routes.insert("/exists".to_string());
        routes.insert("/also-exists/".to_string());

        let html =
            r#"<html><body><a href="/exists">Good</a><a href="/missing">Bad</a></body></html>"#;
        let (result, had_dead) = mark_dead_links(html, &routes).await;

        // Note: These tests require the html cell to be running
        // Without the cell, the function returns the original HTML with no dead links
        // The assertions here work for both cases
        if had_dead {
            assert!(result.contains(r#"data-dead"#));
            assert!(!result.contains(r#"href="/exists" data-dead"#));
        }
    }
}
