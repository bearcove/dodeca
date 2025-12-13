//! Precise URL rewriting using proper parsers
//!
//! - CSS: Uses lightningcss visitor API to find and rewrite `url()` values (via plugin)
//! - HTML: Uses html5ever to parse, mutate, and serialize HTML
//! - JS: Uses OXC parser to find string literals and rewrite asset paths (via plugin)

use std::collections::{HashMap, HashSet};

use crate::plugins::{rewrite_string_literals_in_js_plugin, rewrite_urls_in_css_plugin};
use html5ever::serialize::{SerializeOpts, serialize};
use html5ever::tendril::TendrilSink;
use html5ever::{parse_document, LocalName, QualName, local_name, ns};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};
use std::cell::RefCell;
use std::default::Default;
use std::rc::Rc;

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

/// Collected content that needs async processing
#[derive(Debug)]
struct CollectedContent {
    /// (node_handle, original_css)
    styles: Vec<(Handle, String)>,
    /// (node_handle, original_js)
    scripts: Vec<(Handle, String)>,
}

/// Rewrite URLs in HTML using html5ever parser
///
/// Rewrites:
/// - `href` and `src` attributes
/// - `srcset` attribute values
/// - Inline `<style>` tag content (via lightningcss plugin)
/// - String literals in `<script>` tags (via OXC plugin)
pub async fn rewrite_urls_in_html(html: &str, path_map: &HashMap<String, String>) -> String {
    // First pass: parse HTML and collect content for async processing (sync, no await)
    let (html_with_attrs_rewritten, styles, scripts) = {
        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())
            .unwrap();

        let collected = Rc::new(RefCell::new(CollectedContent {
            styles: Vec::new(),
            scripts: Vec::new(),
        }));

        walk_and_collect(&dom.document, path_map, &collected);

        // Serialize with attribute rewrites
        let mut output = Vec::new();
        let document: SerializableHandle = dom.document.clone().into();
        serialize(
            &mut output,
            &document,
            SerializeOpts {
                scripting_enabled: true,
                traversal_scope: html5ever::serialize::TraversalScope::ChildrenOnly(None),
                create_missing_parent: false,
            },
        )
        .unwrap();

        let html_str = String::from_utf8(output).unwrap_or_else(|_| html.to_string());
        let collected = Rc::try_unwrap(collected).unwrap().into_inner();

        // Extract just the strings (CSS/JS content), drop the Handles
        let styles: Vec<String> = collected.styles.into_iter().map(|(_, s)| s).collect();
        let scripts: Vec<String> = collected.scripts.into_iter().map(|(_, s)| s).collect();

        (html_str, styles, scripts)
    };
    // At this point, all Rc/RcDom is dropped, so we can await safely

    // Process all CSS and JS in parallel
    let css_futures: Vec<_> = styles
        .iter()
        .map(|css| rewrite_urls_in_css(css, path_map))
        .collect();

    let js_futures: Vec<_> = scripts
        .iter()
        .map(|js| rewrite_string_literals_in_js(js, path_map))
        .collect();

    let (css_results, js_results) = futures::future::join(
        futures::future::join_all(css_futures),
        futures::future::join_all(js_futures),
    )
    .await;

    // Replace original content with processed content using string replacement
    let mut result = html_with_attrs_rewritten;
    for (original, processed) in styles.iter().zip(css_results.iter()) {
        if original != processed {
            result = result.replace(original, processed);
        }
    }
    for (original, processed) in scripts.iter().zip(js_results.iter()) {
        if original != processed {
            result = result.replace(original, processed);
        }
    }

    result
}

/// Walk the DOM tree, collecting style/script content and rewriting attributes
fn walk_and_collect(
    handle: &Handle,
    path_map: &HashMap<String, String>,
    collected: &Rc<RefCell<CollectedContent>>,
) {
    let node = handle;

    match &node.data {
        NodeData::Element { name, attrs, .. } => {
            let mut attrs = attrs.borrow_mut();

            // Rewrite href attribute
            if let Some(attr) = attrs.iter_mut().find(|a| a.name.local == local_name!("href")) {
                if let Some(new_val) = path_map.get(attr.value.as_ref()) {
                    attr.value = new_val.clone().into();
                }
            }

            // Rewrite src attribute
            if let Some(attr) = attrs.iter_mut().find(|a| a.name.local == local_name!("src")) {
                if let Some(new_val) = path_map.get(attr.value.as_ref()) {
                    attr.value = new_val.clone().into();
                }
            }

            // Rewrite srcset attribute
            if let Some(attr) = attrs.iter_mut().find(|a| a.name.local == local_name!("srcset")) {
                let new_srcset = rewrite_srcset(&attr.value, path_map);
                attr.value = new_srcset.into();
            }

            // Collect style tag content
            if name.local == local_name!("style") {
                let text = get_text_content(handle);
                if !text.trim().is_empty() {
                    collected.borrow_mut().styles.push((handle.clone(), text));
                }
            }

            // Collect script tag content
            if name.local == local_name!("script") {
                let text = get_text_content(handle);
                if !text.trim().is_empty() {
                    collected.borrow_mut().scripts.push((handle.clone(), text));
                }
            }
        }
        _ => {}
    }

    // Recurse into children
    for child in node.children.borrow().iter() {
        walk_and_collect(child, path_map, collected);
    }
}

/// Get text content of an element (concatenating all text nodes)
fn get_text_content(handle: &Handle) -> String {
    let mut text = String::new();
    for child in handle.children.borrow().iter() {
        if let NodeData::Text { contents } = &child.data {
            text.push_str(&contents.borrow());
        }
    }
    text
}

/// Mark dead internal links in HTML
///
/// Adds `data-dead` attribute to `<a>` tags with internal hrefs that don't exist in known_routes.
/// Returns (modified_html, had_dead_links) tuple.
pub fn mark_dead_links(html: &str, known_routes: &HashSet<String>) -> (String, bool) {
    // Parse HTML into DOM
    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();

    let had_dead = Rc::new(RefCell::new(false));
    walk_and_mark_dead(&dom.document, known_routes, &had_dead);

    // Serialize back to HTML
    let mut output = Vec::new();
    let document: SerializableHandle = dom.document.clone().into();
    serialize(
        &mut output,
        &document,
        SerializeOpts {
            scripting_enabled: true,
            traversal_scope: html5ever::serialize::TraversalScope::ChildrenOnly(None),
            create_missing_parent: false,
        },
    )
    .unwrap();

    let result = String::from_utf8(output).unwrap_or_else(|_| html.to_string());
    (result, *had_dead.borrow())
}

/// Walk DOM and mark dead links
fn walk_and_mark_dead(
    handle: &Handle,
    known_routes: &HashSet<String>,
    had_dead: &Rc<RefCell<bool>>,
) {
    let node = handle;

    if let NodeData::Element { name, attrs, .. } = &node.data {
        // Only check <a> elements with href
        if name.local == local_name!("a") {
            let mut attrs = attrs.borrow_mut();
            if let Some(href_attr) = attrs.iter().find(|a| a.name.local == local_name!("href")) {
                let href = href_attr.value.as_ref();

                // Skip external links, anchors, special protocols, and static files
                if !href.starts_with("http://")
                    && !href.starts_with("https://")
                    && !href.starts_with('#')
                    && !href.starts_with("mailto:")
                    && !href.starts_with("tel:")
                    && !href.starts_with("javascript:")
                    && !href.starts_with("/__")
                    && href.starts_with('/')
                {
                    // Skip static file extensions
                    let static_extensions = [
                        ".css", ".js", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".woff",
                        ".woff2", ".ttf", ".eot", ".pdf", ".zip", ".tar", ".gz", ".webp", ".jxl",
                        ".xml", ".txt", ".md", ".wasm",
                    ];

                    if !static_extensions.iter().any(|ext| href.ends_with(ext)) {
                        // Split off fragment
                        let path = href.split('#').next().unwrap_or(href);
                        if !path.is_empty() {
                            let target = normalize_route_for_check(path);

                            // Check if route exists
                            let exists = known_routes.contains(&target)
                                || known_routes
                                    .contains(&format!("{}/", target.trim_end_matches('/')))
                                || known_routes.contains(target.trim_end_matches('/'));

                            if !exists {
                                // Add data-dead attribute
                                attrs.push(html5ever::Attribute {
                                    name: QualName::new(
                                        None,
                                        ns!(),
                                        LocalName::from("data-dead"),
                                    ),
                                    value: target.into(),
                                });
                                *had_dead.borrow_mut() = true;
                            }
                        }
                    }
                }
            }
        }
    }

    // Recurse into children
    for child in node.children.borrow().iter() {
        walk_and_mark_dead(child, known_routes, had_dead);
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

/// Rewrite URLs in a srcset attribute value
///
/// srcset format: "url1 1x, url2 2x" or "url1 100w, url2 200w"
fn rewrite_srcset(srcset: &str, path_map: &HashMap<String, String>) -> String {
    srcset
        .split(',')
        .map(|entry| {
            let entry = entry.trim();
            // Split on whitespace: "url 2x" -> ["url", "2x"]
            let parts: Vec<&str> = entry.split_whitespace().collect();
            if parts.is_empty() {
                return entry.to_string();
            }

            let url = parts[0];
            let descriptor = parts.get(1).copied().unwrap_or("");

            // Try to rewrite the URL
            let new_url = path_map.get(url).map(|s| s.as_str()).unwrap_or(url);

            if descriptor.is_empty() {
                new_url.to_string()
            } else {
                format!("{} {}", new_url, descriptor)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
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

    #[test]
    fn test_srcset_rewriting() {
        let mut path_map = HashMap::new();
        path_map.insert("/img/a.png".to_string(), "/img/a.abc123.png".to_string());
        path_map.insert("/img/b.png".to_string(), "/img/b.def456.png".to_string());

        let srcset = "/img/a.png 1x, /img/b.png 2x";
        let result = rewrite_srcset(srcset, &path_map);
        assert_eq!(result, "/img/a.abc123.png 1x, /img/b.def456.png 2x");
    }

    #[test]
    fn test_normalize_route() {
        assert_eq!(normalize_route_for_check("/foo/bar"), "/foo/bar");
        assert_eq!(normalize_route_for_check("/foo/../bar"), "/bar");
        assert_eq!(normalize_route_for_check("/foo/./bar"), "/foo/bar");
        assert_eq!(normalize_route_for_check("/"), "/");
    }

    #[tokio::test]
    async fn test_html_attribute_rewriting() {
        let mut path_map = HashMap::new();
        path_map.insert("/style.css".to_string(), "/style.abc123.css".to_string());
        path_map.insert("/app.js".to_string(), "/app.def456.js".to_string());

        let html = r#"<html><head><link href="/style.css"></head><body><script src="/app.js"></script></body></html>"#;
        let result = rewrite_urls_in_html(html, &path_map).await;

        assert!(result.contains(r#"href="/style.abc123.css""#));
        assert!(result.contains(r#"src="/app.def456.js""#));
    }

    #[test]
    fn test_dead_link_marking() {
        let mut routes = HashSet::new();
        routes.insert("/exists".to_string());
        routes.insert("/also-exists/".to_string());

        let html = r#"<html><body><a href="/exists">Good</a><a href="/missing">Bad</a></body></html>"#;
        let (result, had_dead) = mark_dead_links(html, &routes);

        assert!(had_dead);
        assert!(result.contains(r#"data-dead="/missing""#));
        assert!(!result.contains(r#"href="/exists" data-dead"#));
    }
}
