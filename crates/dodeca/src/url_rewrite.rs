//! Precise URL rewriting using proper parsers
//!
//! - CSS: Uses lightningcss visitor API to find and rewrite `url()` values (via plugin)
//! - HTML: Uses lol_html to rewrite attributes and inline style/script content
//! - JS: Uses OXC parser to find string literals and rewrite asset paths (via plugin)

use std::collections::{HashMap, HashSet};

use crate::plugins::{rewrite_string_literals_in_js_plugin, rewrite_urls_in_css_plugin};

/// Rewrite URLs in CSS using lightningcss parser (via plugin)
///
/// Only rewrites actual `url()` values in CSS, not text that happens to look like URLs.
/// Also minifies the CSS output.
pub fn rewrite_urls_in_css(css: &str, path_map: &HashMap<String, String>) -> String {
    match rewrite_urls_in_css_plugin(css, path_map) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("CSS rewriting failed: {}", e);
            css.to_string()
        }
    }
}

/// Rewrite URLs in HTML using lol_html parser
///
/// Rewrites:
/// - `href` and `src` attributes
/// - `srcset` attribute values
/// - Inline `<style>` tag content (via lightningcss)
/// - String literals in `<script>` tags that match known asset paths
pub fn rewrite_urls_in_html(html: &str, path_map: &HashMap<String, String>) -> String {
    use lol_html::{RewriteStrSettings, element, rewrite_str, text};

    // Clone path_map for the closures
    let href_map = path_map.clone();
    let src_map = path_map.clone();
    let srcset_map = path_map.clone();
    let style_map = path_map.clone();

    let result = rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                // Rewrite href attributes (links, stylesheets)
                element!("[href]", |el| {
                    if let Some(href) = el.get_attribute("href")
                        && let Some(new_href) = href_map.get(&href)
                    {
                        el.set_attribute("href", new_href).ok();
                    }
                    Ok(())
                }),
                // Rewrite src attributes (images, scripts)
                element!("[src]", |el| {
                    if let Some(src) = el.get_attribute("src")
                        && let Some(new_src) = src_map.get(&src)
                    {
                        el.set_attribute("src", new_src).ok();
                    }
                    Ok(())
                }),
                // Rewrite srcset attributes (responsive images)
                element!("[srcset]", |el| {
                    if let Some(srcset) = el.get_attribute("srcset") {
                        let new_srcset = rewrite_srcset(&srcset, &srcset_map);
                        el.set_attribute("srcset", &new_srcset).ok();
                    }
                    Ok(())
                }),
                // Rewrite inline <style> content
                text!("style", |text| {
                    let css = text.as_str();
                    if !css.trim().is_empty() {
                        let rewritten = rewrite_urls_in_css(css, &style_map);
                        text.replace(&rewritten, lol_html::html_content::ContentType::Text);
                    }
                    Ok(())
                }),
            ],
            ..Default::default()
        },
    );

    // First pass: HTML attributes and style tags via lol_html
    let html_rewritten = match result {
        Ok(rewritten) => rewritten,
        Err(e) => {
            tracing::warn!("Failed to rewrite HTML URLs: {:?}", e);
            html.to_string()
        }
    };

    // Second pass: rewrite script content via regex (lol_html doesn't allow script text replacement)
    rewrite_script_tags(&html_rewritten, path_map)
}

/// Mark dead internal links in HTML using lol_html
///
/// Adds `data-dead` attribute to `<a>` tags with internal hrefs that don't exist in known_routes.
/// Returns (modified_html, had_dead_links) tuple.
pub fn mark_dead_links(html: &str, known_routes: &HashSet<String>) -> (String, bool) {
    use lol_html::{RewriteStrSettings, element, rewrite_str};
    use std::cell::Cell;

    let had_dead = Cell::new(false);
    let routes = known_routes.clone();

    let result = rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![element!("a[href]", |el| {
                if let Some(href) = el.get_attribute("href") {
                    // Skip external links, anchors, special protocols, and static files
                    if href.starts_with("http://")
                        || href.starts_with("https://")
                        || href.starts_with('#')
                        || href.starts_with("mailto:")
                        || href.starts_with("tel:")
                        || href.starts_with("javascript:")
                        || href.starts_with("/__")
                    {
                        return Ok(());
                    }

                    // Skip static file extensions
                    let static_extensions = [
                        ".css", ".js", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".woff",
                        ".woff2", ".ttf", ".eot", ".pdf", ".zip", ".tar", ".gz", ".webp", ".jxl",
                        ".xml", ".txt", ".md", ".wasm",
                    ];
                    if static_extensions.iter().any(|ext| href.ends_with(ext)) {
                        return Ok(());
                    }

                    // Only check absolute internal links (starting with /)
                    if !href.starts_with('/') {
                        return Ok(());
                    }

                    // Split off fragment
                    let path = href.split('#').next().unwrap_or(&href);
                    if path.is_empty() {
                        return Ok(());
                    }

                    // Normalize the route
                    let target = normalize_route_for_check(path);

                    // Check if route exists (with various trailing slash combinations)
                    let exists = routes.contains(&target)
                        || routes.contains(&format!("{}/", target.trim_end_matches('/')))
                        || routes.contains(target.trim_end_matches('/'));

                    if !exists {
                        el.set_attribute("data-dead", &target).ok();
                        had_dead.set(true);
                    }
                }
                Ok(())
            })],
            ..Default::default()
        },
    );

    match result {
        Ok(rewritten) => (rewritten, had_dead.get()),
        Err(e) => {
            tracing::warn!("Failed to mark dead links: {:?}", e);
            (html.to_string(), false)
        }
    }
}

/// Normalize a route path for dead link checking
fn normalize_route_for_check(path: &str) -> String {
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

/// Rewrite script tag content in HTML
///
/// Since lol_html doesn't allow direct modification of `<script>` text content,
/// we use regex to find and rewrite script tags.
fn rewrite_script_tags(html: &str, path_map: &HashMap<String, String>) -> String {
    use regex::Regex;

    // Match <script>...</script> tags, capturing the content
    // Using (?s) for DOTALL mode so . matches newlines
    let script_re = Regex::new(r"(?s)(<script[^>]*>)(.*?)(</script>)").unwrap();

    let result = script_re.replace_all(html, |caps: &regex::Captures| {
        let open_tag = &caps[1];
        let content = &caps[2];
        let close_tag = &caps[3];

        let rewritten_content = rewrite_string_literals_in_js(content, path_map);
        format!("{open_tag}{rewritten_content}{close_tag}")
    });

    result.into_owned()
}

/// Rewrite string literals in JavaScript that contain asset paths
///
/// Uses OXC parser (via plugin) to properly parse JavaScript and find string literals,
/// then replaces paths with cache-busted versions.
fn rewrite_string_literals_in_js(js: &str, path_map: &HashMap<String, String>) -> String {
    match rewrite_string_literals_in_js_plugin(js, path_map) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("JS rewriting failed: {}", e);
            js.to_string()
        }
    }
}

/// Rewrite URLs in a srcset attribute value
///
/// srcset format: "url1 1x, url2 2x" or "url1 100w, url2 200w"
fn rewrite_srcset(srcset: &str, path_map: &HashMap<String, String>) -> String {
    srcset
        .split(',')
        .map(|entry| {
            let entry = entry.trim();
            // Split into URL and descriptor (e.g., "1x", "100w")
            let parts: Vec<&str> = entry.split_whitespace().collect();
            if parts.is_empty() {
                return entry.to_string();
            }

            let url = parts[0];
            let descriptor = parts.get(1).copied();

            let new_url = path_map.get(url).map(|s| s.as_str()).unwrap_or(url);

            match descriptor {
                Some(d) => format!("{new_url} {d}"),
                None => new_url.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Information about responsive image variants for HTML transformation
#[derive(Debug, Clone)]
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
///
/// Transforms:
/// ```html
/// <img src="/images/photo.png" alt="A photo">
/// ```
/// Into:
/// ```html
/// <picture>
///   <source srcset="/images/photo-320w.jxl 320w, /images/photo.jxl 1920w" type="image/jxl">
///   <source srcset="/images/photo-320w.webp 320w, /images/photo.webp 1920w" type="image/webp">
///   <img src="/images/photo.webp" alt="A photo" width="1920" height="1080"
///        loading="lazy" decoding="async"
///        style="background: url(data:image/png;base64,...) center/cover no-repeat">
/// </picture>
/// ```
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
    // Captures: 1=before src, 2=src value, 3=after src before closing, 4=self-closing or not
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

            // Extract existing width/height/loading/decoding attributes to avoid duplicates
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
                // Add thumbhash placeholder as background, cleared on load
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
            // Not a processable image, return unchanged
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

    #[test]
    fn test_rewrite_css_urls() {
        let css = r#"
            @font-face {
                font-family: "Inter";
                src: url("/fonts/Inter.woff2") format("woff2");
            }
            body {
                background: url("/images/bg.png");
            }
        "#;

        let mut path_map = HashMap::new();
        path_map.insert(
            "/fonts/Inter.woff2".to_string(),
            "/fonts/Inter.abc123.woff2".to_string(),
        );
        path_map.insert(
            "/images/bg.png".to_string(),
            "/images/bg.def456.png".to_string(),
        );

        let result = rewrite_urls_in_css(css, &path_map);

        assert!(result.contains("/fonts/Inter.abc123.woff2"));
        assert!(result.contains("/images/bg.def456.png"));
        assert!(!result.contains("\"/fonts/Inter.woff2\""));
        assert!(!result.contains("\"/images/bg.png\""));
    }

    #[test]
    fn test_rewrite_html_urls() {
        let html = r#"<!DOCTYPE html>
<html>
<head>
    <link rel="stylesheet" href="/main.css">
</head>
<body>
    <img src="/images/logo.png" alt="Logo">
    <a href="/about/">About</a>
    <p>Check out /main.css in your browser</p>
</body>
</html>"#;

        let mut path_map = HashMap::new();
        path_map.insert("/main.css".to_string(), "/main.abc123.css".to_string());
        path_map.insert(
            "/images/logo.png".to_string(),
            "/images/logo.def456.png".to_string(),
        );

        let result = rewrite_urls_in_html(html, &path_map);

        // Attributes should be rewritten
        assert!(result.contains("href=\"/main.abc123.css\""));
        assert!(result.contains("src=\"/images/logo.def456.png\""));

        // Text content should NOT be rewritten
        assert!(result.contains("Check out /main.css in your browser"));

        // Non-matching href should be unchanged
        assert!(result.contains("href=\"/about/\""));
    }

    #[test]
    fn test_rewrite_inline_style() {
        let html = r#"<!DOCTYPE html>
<html>
<head>
    <style>
        body {
            background: url("/images/bg.png");
        }
    </style>
</head>
<body></body>
</html>"#;

        let mut path_map = HashMap::new();
        path_map.insert(
            "/images/bg.png".to_string(),
            "/images/bg.abc123.png".to_string(),
        );

        let result = rewrite_urls_in_html(html, &path_map);

        assert!(result.contains("/images/bg.abc123.png"));
        assert!(!result.contains("\"/images/bg.png\""));
    }

    #[test]
    fn test_rewrite_inline_script() {
        let html = r#"<!DOCTYPE html>
<html>
<head>
    <script>
        const logo = "/images/logo.png";
        const other = '/images/icon.svg';
        const template = `/images/hero.jpg`;
        const notAnAsset = "/about/";
    </script>
</head>
<body></body>
</html>"#;

        let mut path_map = HashMap::new();
        path_map.insert(
            "/images/logo.png".to_string(),
            "/images/logo.abc123.png".to_string(),
        );
        path_map.insert(
            "/images/icon.svg".to_string(),
            "/images/icon.def456.svg".to_string(),
        );
        path_map.insert(
            "/images/hero.jpg".to_string(),
            "/images/hero.789xyz.jpg".to_string(),
        );

        let result = rewrite_urls_in_html(html, &path_map);

        // Known assets should be rewritten
        assert!(result.contains("\"/images/logo.abc123.png\""));
        assert!(result.contains("'/images/icon.def456.svg'"));
        assert!(result.contains("`/images/hero.789xyz.jpg`"));

        // Unknown paths should NOT be rewritten
        assert!(result.contains("\"/about/\""));
    }

    #[test]
    fn test_script_text_not_rewritten() {
        let html = r#"<!DOCTYPE html>
<html>
<head>
    <script>
        // Comment: /images/logo.png
        console.log("Loading /images/logo.png");
    </script>
</head>
<body></body>
</html>"#;

        let mut path_map = HashMap::new();
        path_map.insert(
            "/images/logo.png".to_string(),
            "/images/logo.abc123.png".to_string(),
        );

        let result = rewrite_urls_in_html(html, &path_map);

        // The string literal should be rewritten
        assert!(result.contains("\"Loading /images/logo.abc123.png\""));

        // The comment should NOT be rewritten (it's not a string literal)
        assert!(result.contains("// Comment: /images/logo.png"));
    }

    #[test]
    fn test_css_url_in_text_not_rewritten() {
        let css = r#"
            /* This comment mentions /fonts/Inter.woff2 */
            body { font-family: sans-serif; }
        "#;

        let mut path_map = HashMap::new();
        path_map.insert(
            "/fonts/Inter.woff2".to_string(),
            "/fonts/Inter.abc123.woff2".to_string(),
        );

        let result = rewrite_urls_in_css(css, &path_map);

        // Comment should not be rewritten (lightningcss strips comments by default)
        // The important thing is we don't crash or produce invalid CSS
        assert!(!result.contains("Inter.abc123"));
    }

    #[test]
    fn test_rewrite_srcset() {
        let mut path_map = HashMap::new();
        path_map.insert(
            "/img/hero.png".to_string(),
            "/img/hero.abc123.png".to_string(),
        );
        path_map.insert(
            "/img/hero-2x.png".to_string(),
            "/img/hero-2x.def456.png".to_string(),
        );

        let srcset = "/img/hero.png 1x, /img/hero-2x.png 2x";
        let result = rewrite_srcset(srcset, &path_map);

        assert_eq!(
            result,
            "/img/hero.abc123.png 1x, /img/hero-2x.def456.png 2x"
        );
    }

    #[test]
    fn test_js_string_literal_rewriting() {
        let js = r#"
            const a = "/images/logo.png";
            const b = '/images/icon.svg';
            const c = `/images/hero.jpg`;
            const d = "/not/in/map.png";
        "#;

        let mut path_map = HashMap::new();
        path_map.insert(
            "/images/logo.png".to_string(),
            "/images/logo.abc.png".to_string(),
        );
        path_map.insert(
            "/images/icon.svg".to_string(),
            "/images/icon.def.svg".to_string(),
        );
        path_map.insert(
            "/images/hero.jpg".to_string(),
            "/images/hero.ghi.jpg".to_string(),
        );

        let result = rewrite_string_literals_in_js(js, &path_map);

        assert!(result.contains("\"/images/logo.abc.png\""));
        assert!(result.contains("'/images/icon.def.svg'"));
        assert!(result.contains("`/images/hero.ghi.jpg`"));
        assert!(result.contains("\"/not/in/map.png\"")); // unchanged
    }

    #[test]
    fn test_transform_images_to_picture() {
        let html = r#"<img src="/images/photo.png" alt="A photo">"#;

        let mut image_variants = HashMap::new();
        image_variants.insert(
            "/images/photo.png".to_string(),
            ResponsiveImageInfo {
                jxl_srcset: vec![
                    ("/images/photo-320w.jxl".to_string(), 320),
                    ("/images/photo.jxl".to_string(), 800),
                ],
                webp_srcset: vec![
                    ("/images/photo-320w.webp".to_string(), 320),
                    ("/images/photo.webp".to_string(), 800),
                ],
                original_width: 800,
                original_height: 600,
                thumbhash_data_url: "data:image/png;base64,ABC123".to_string(),
            },
        );

        let result = transform_images_to_picture(html, &image_variants);

        // Should be wrapped in <picture>
        assert!(result.contains("<picture>"));
        assert!(result.contains("</picture>"));

        // Should have JXL and WebP sources with srcset
        assert!(result.contains("type=\"image/jxl\""));
        assert!(result.contains("type=\"image/webp\""));
        assert!(result.contains("320w"));
        assert!(result.contains("800w"));

        // Fallback img should use largest WebP
        assert!(result.contains(r#"src="/images/photo.webp""#));

        // Should have width and height
        assert!(result.contains(r#"width="800""#));
        assert!(result.contains(r#"height="600""#));

        // Should have lazy loading and decoding async
        assert!(result.contains(r#"loading="lazy""#));
        assert!(result.contains(r#"decoding="async""#));

        // Should have thumbhash background
        assert!(result.contains("background:url(data:image/png;base64,ABC123)"));

        // Should preserve alt attribute
        assert!(result.contains(r#"alt="A photo""#));
    }

    #[test]
    fn test_transform_images_to_picture_preserves_existing_dimensions() {
        let html = r#"<img src="/images/photo.png" width="400" height="300" alt="Resized">"#;

        let mut image_variants = HashMap::new();
        image_variants.insert(
            "/images/photo.png".to_string(),
            ResponsiveImageInfo {
                jxl_srcset: vec![("/images/photo.jxl".to_string(), 800)],
                webp_srcset: vec![("/images/photo.webp".to_string(), 800)],
                original_width: 800,
                original_height: 600,
                thumbhash_data_url: "data:image/png;base64,XYZ".to_string(),
            },
        );

        let result = transform_images_to_picture(html, &image_variants);

        // Should NOT add width/height (already present)
        // Should keep existing width/height
        assert!(result.contains(r#"width="400""#));
        assert!(result.contains(r#"height="300""#));
        // Should NOT have the intrinsic dimensions added
        assert!(!result.contains(r#"width="800""#));
    }

    #[test]
    fn test_transform_images_to_picture_non_matching() {
        let html = r#"<img src="/images/external.png" alt="External">"#;

        let image_variants: HashMap<String, ResponsiveImageInfo> = HashMap::new(); // Empty - no matches

        let result = transform_images_to_picture(html, &image_variants);

        // Should be unchanged
        assert_eq!(result, html);
    }
}
