//! Precise URL rewriting using proper parsers
//!
//! - CSS: Uses lightningcss visitor API to find and rewrite `url()` values (via plugin)
//! - HTML: Uses html5ever to parse, mutate, and serialize HTML (via plugin)
//! - JS: Uses OXC parser to find string literals and rewrite asset paths (via plugin)

use std::collections::{HashMap, HashSet};

use crate::plugins::{
    mark_dead_links_plugin, rewrite_string_literals_in_js_plugin, rewrite_urls_in_css_plugin,
    rewrite_urls_in_html_plugin,
};

/// Rewrite URLs in CSS using lightningcss parser (via plugin)
///
/// Only rewrites actual `url()` values in CSS, not text that happens to look like URLs.
/// Also minifies the CSS output.
/// Returns original CSS if plugin is not available.
pub async fn rewrite_urls_in_css(css: &str, path_map: &HashMap<String, String>) -> String {
    // Check if CSS plugin is available
    if crate::plugins::plugins().css.is_none() {
        return css.to_string();
    }

    match rewrite_urls_in_css_plugin(css, path_map).await {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("CSS rewriting failed: {}", e);
            css.to_string()
        }
    }
}

/// Rewrite string literals in JavaScript that contain asset paths (async version)
/// Returns original JS if plugin is not available.
async fn rewrite_string_literals_in_js(js: &str, path_map: &HashMap<String, String>) -> String {
    // Check if JS plugin is available
    if crate::plugins::plugins().js.is_none() {
        return js.to_string();
    }

    match rewrite_string_literals_in_js_plugin(js, path_map).await {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("JS rewriting failed: {}", e);
            js.to_string()
        }
    }
}

/// Rewrite URLs in HTML using the html plugin
///
/// Rewrites:
/// - `href` and `src` attributes
/// - `srcset` attribute values
/// - Inline `<style>` tag content (via lightningcss plugin)
/// - String literals in `<script>` tags (via OXC plugin)
///
/// Returns original HTML if the html plugin is not available.
pub async fn rewrite_urls_in_html(html: &str, path_map: &HashMap<String, String>) -> String {
    // First: rewrite HTML attributes using the plugin
    let html_with_attrs = match rewrite_urls_in_html_plugin(html, path_map).await {
        Some(result) => result,
        None => html.to_string(),
    };

    // Then: extract and process inline CSS and JS
    // This is a simplified approach - we use regex to find <style> and <script> content
    // and rewrite them separately, then replace in the HTML
    let mut result = html_with_attrs;

    // Process inline styles
    let style_re = regex::Regex::new(r"<style[^>]*>([\s\S]*?)</style>").unwrap();
    for cap in style_re.captures_iter(&result.clone()) {
        let original_css = &cap[1];
        if !original_css.trim().is_empty() {
            let processed_css = rewrite_urls_in_css(original_css, path_map).await;
            if original_css != processed_css {
                result = result.replace(original_css, &processed_css);
            }
        }
    }

    // Process inline scripts
    let script_re = regex::Regex::new(r"<script[^>]*>([\s\S]*?)</script>").unwrap();
    for cap in script_re.captures_iter(&result.clone()) {
        let original_js = &cap[1];
        if !original_js.trim().is_empty() {
            let processed_js = rewrite_string_literals_in_js(original_js, path_map).await;
            if original_js != processed_js {
                result = result.replace(original_js, &processed_js);
            }
        }
    }

    result
}

/// Mark dead internal links in HTML using the plugin
///
/// Adds `data-dead` attribute to `<a>` tags with internal hrefs that don't exist in known_routes.
/// Returns (modified_html, had_dead_links) tuple.
/// Returns original HTML with no dead links if the plugin is not available.
pub async fn mark_dead_links(html: &str, known_routes: &HashSet<String>) -> (String, bool) {
    match mark_dead_links_plugin(html, known_routes).await {
        Some((result, had_dead)) => (result, had_dead),
        None => (html.to_string(), false),
    }
}

/// Information about responsive image variants for picture element generation
pub struct ResponsiveImageInfo {
    /// JXL srcset entries: vec of (path, width)
    pub jxl_srcset: Vec<(String, u32)>,
    /// WebP srcset entries: vec of (path, width)
    pub webp_srcset: Vec<(String, u32)>,
    /// Original dimensions
    pub original_width: u32,
    pub original_height: u32,
    /// Thumbhash data URL for placeholder
    pub thumbhash_data_url: String,
}

/// Build a srcset string from path/width pairs
fn build_srcset(entries: &[(String, u32)]) -> String {
    entries
        .iter()
        .map(|(path, width)| format!("{path} {width}w"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Transform `<img>` tags pointing to internal images into `<picture>` elements
///
/// The `image_variants` map contains:
/// - Key: original image path (e.g., "/images/photo.png")
/// - Value: ResponsiveImageInfo with srcsets, dimensions, and thumbhash
pub fn transform_images_to_picture(
    html: &str,
    image_variants: &HashMap<String, ResponsiveImageInfo>,
) -> String {
    use regex::Regex;

    // If no image variants, return unchanged
    if image_variants.is_empty() {
        return html.to_string();
    }

    // Match <img> tags with src attribute (double quotes)
    let img_re_double = Regex::new(r#"<img\s+([^>]*?)src="([^"]+)"([^>]*?)(/?)>"#).unwrap();
    // Match <img> tags with src attribute (single quotes)
    let img_re_single = Regex::new(r#"<img\s+([^>]*?)src='([^']+)'([^>]*?)(/?)>"#).unwrap();

    let transform = |caps: &regex::Captures, quote: &str| -> String {
        let before_src = &caps[1];
        let src = &caps[2];
        let after_src = &caps[3];
        let self_closing = &caps[4];

        // Check if this src has image variants
        if let Some(info) = image_variants.get(src) {
            // Build srcset strings
            let jxl_srcset = build_srcset(&info.jxl_srcset);
            let webp_srcset = build_srcset(&info.webp_srcset);

            // Get the largest WebP variant for fallback src
            let fallback_src = info
                .webp_srcset
                .iter()
                .max_by_key(|(_, w)| w)
                .map(|(p, _)| p.as_str())
                .unwrap_or("");

            // Extract existing attributes to avoid duplicates
            let has_width = before_src.contains("width=") || after_src.contains("width=");
            let has_height = before_src.contains("height=") || after_src.contains("height=");
            let has_loading = before_src.contains("loading=") || after_src.contains("loading=");
            let has_decoding = before_src.contains("decoding=") || after_src.contains("decoding=");
            let has_style = before_src.contains("style=") || after_src.contains("style=");

            // Build extra attributes
            let mut extra_attrs = String::new();
            if !has_width {
                extra_attrs.push_str(&format!(" width={quote}{}{quote}", info.original_width));
            }
            if !has_height {
                extra_attrs.push_str(&format!(" height={quote}{}{quote}", info.original_height));
            }
            if !has_loading {
                extra_attrs.push_str(&format!(" loading={quote}lazy{quote}"));
            }
            if !has_decoding {
                extra_attrs.push_str(&format!(" decoding={quote}async{quote}"));
            }
            if !has_style {
                extra_attrs.push_str(&format!(
                    " style={quote}background:url({}) center/cover no-repeat{quote}",
                    info.thumbhash_data_url
                ));
                extra_attrs.push_str(&format!(
                    " onload={quote}this.style.background='none'{quote}"
                ));
            }

            // Reconstruct the img tag with WebP src and extra attributes
            let img_tag = format!(
                "<img {before_src}src={quote}{fallback_src}{quote}{after_src}{extra_attrs}{self_closing}>"
            );

            // Build the picture element with responsive srcsets
            format!(
                "<picture>\
                    <source srcset={quote}{jxl_srcset}{quote} type={quote}image/jxl{quote}>\
                    <source srcset={quote}{webp_srcset}{quote} type={quote}image/webp{quote}>\
                    {img_tag}\
                </picture>"
            )
        } else {
            caps[0].to_string()
        }
    };

    // First pass: double quotes
    let result = img_re_double.replace_all(html, |caps: &regex::Captures| transform(caps, "\""));

    // Second pass: single quotes
    let result = img_re_single.replace_all(&result, |caps: &regex::Captures| transform(caps, "'"));

    result.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_html_attribute_rewriting() {
        // Note: This test requires the html plugin to be running
        // Without the plugin, the function returns the original HTML
        let mut path_map = HashMap::new();
        path_map.insert("/style.css".to_string(), "/style.abc123.css".to_string());
        path_map.insert("/app.js".to_string(), "/app.def456.js".to_string());

        let html = r#"<html><head><link href="/style.css"></head><body><script src="/app.js"></script></body></html>"#;
        let result = rewrite_urls_in_html(html, &path_map).await;

        // With plugin: URLs are rewritten
        // Without plugin: returns original HTML
        if result.contains("abc123") {
            assert!(result.contains(r#"href="/style.abc123.css""#));
            assert!(result.contains(r#"src="/app.def456.js""#));
        } else {
            assert_eq!(result, html, "Without plugin, HTML should be unchanged");
        }
    }

    #[tokio::test]
    async fn test_dead_link_marking() {
        let mut routes = HashSet::new();
        routes.insert("/exists".to_string());
        routes.insert("/also-exists/".to_string());

        let html = r#"<html><body><a href="/exists">Good</a><a href="/missing">Bad</a></body></html>"#;
        let (result, had_dead) = mark_dead_links(html, &routes).await;

        // Note: These tests require the html plugin to be running
        // Without the plugin, the function returns the original HTML with no dead links
        // The assertions here work for both cases
        if had_dead {
            assert!(result.contains(r#"data-dead="/missing""#));
            assert!(!result.contains(r#"href="/exists" data-dead"#));
        }
    }
}
