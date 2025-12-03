use crate::db::{Heading, Page, Section, SiteTree};
use crate::template::{Context, Engine, InMemoryLoader, Value};
use crate::types::Route;
use crate::url_rewrite::mark_dead_links;
use std::collections::{HashMap, HashSet};

/// Find the nearest parent section for a route (for page context)
fn find_parent_section<'a>(route: &Route, site_tree: &'a SiteTree) -> Option<&'a Section> {
    let mut current = route.clone();
    loop {
        if let Some(section) = site_tree.sections.get(&current) {
            if current != *route {
                return Some(section);
            }
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => {
                // Return root section if it exists
                return site_tree.sections.get(&Route::root());
            }
        }
    }
}

/// Convert ANSI escape codes to HTML spans with inline styles.
fn ansi_to_html(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars().peekable();
    let mut in_span = false;

    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['

            // Parse the escape sequence
            let mut seq = String::new();
            while let Some(&ch) = chars.peek() {
                if ch.is_ascii_digit() || ch == ';' {
                    seq.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            // Consume the final character (usually 'm')
            let final_char = chars.next();

            if final_char == Some('m') {
                // Close any existing span
                if in_span {
                    output.push_str("</span>");
                    in_span = false;
                }

                // Parse the style
                if let Some(style) = parse_ansi_style(&seq) {
                    if !style.is_empty() {
                        output.push_str(&format!("<span style=\"{style}\">"));
                        in_span = true;
                    }
                }
            }
        } else if c == '<' {
            output.push_str("&lt;");
        } else if c == '>' {
            output.push_str("&gt;");
        } else if c == '&' {
            output.push_str("&amp;");
        } else {
            output.push(c);
        }
    }

    if in_span {
        output.push_str("</span>");
    }

    output
}

/// Parse ANSI style codes and return CSS style string.
fn parse_ansi_style(seq: &str) -> Option<String> {
    if seq.is_empty() || seq == "0" {
        return Some(String::new()); // Reset
    }

    let parts: Vec<&str> = seq.split(';').collect();
    let mut styles = Vec::new();

    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "0" => return Some(String::new()), // Reset
            "1" => styles.push("font-weight:bold".to_string()),
            "2" => styles.push("opacity:0.7".to_string()), // Dim
            "3" => styles.push("font-style:italic".to_string()),
            "4" => styles.push("text-decoration:underline".to_string()),
            "30" => styles.push("color:#000".to_string()),
            "31" => styles.push("color:#e06c75".to_string()), // Red
            "32" => styles.push("color:#98c379".to_string()), // Green
            "33" => styles.push("color:#e5c07b".to_string()), // Yellow
            "34" => styles.push("color:#61afef".to_string()), // Blue
            "35" => styles.push("color:#c678dd".to_string()), // Magenta
            "36" => styles.push("color:#56b6c2".to_string()), // Cyan
            "37" => styles.push("color:#abb2bf".to_string()), // White
            "38" => {
                // Extended color (24-bit RGB)
                if i + 1 < parts.len() && parts[i + 1] == "2" && i + 4 < parts.len() {
                    let r = parts[i + 2];
                    let g = parts[i + 3];
                    let b = parts[i + 4];
                    styles.push(format!("color:rgb({r},{g},{b})"));
                    i += 4;
                }
            }
            "90" => styles.push("color:#5c6370".to_string()), // Bright black (gray)
            "91" => styles.push("color:#e06c75".to_string()), // Bright red
            "92" => styles.push("color:#98c379".to_string()), // Bright green
            "93" => styles.push("color:#e5c07b".to_string()), // Bright yellow
            "94" => styles.push("color:#61afef".to_string()), // Bright blue
            "95" => styles.push("color:#c678dd".to_string()), // Bright magenta
            "96" => styles.push("color:#56b6c2".to_string()), // Bright cyan
            "97" => styles.push("color:#fff".to_string()),    // Bright white
            _ => {}
        }
        i += 1;
    }

    Some(styles.join(";"))
}

/// Options for rendering
#[derive(Default, Clone, Copy)]
pub struct RenderOptions {
    /// Whether to inject live reload script
    pub livereload: bool,
    /// Development mode - show error pages instead of failing
    pub dev_mode: bool,
}

/// CSS for dead link highlighting in dev mode (subtle overline)
const DEAD_LINK_STYLES: &str = r#"<style>
a[data-dead] {
    text-decoration: overline !important;
    text-decoration-color: rgba(255, 107, 107, 0.6) !important;
}
</style>"#;

/// Inject livereload script and optionally mark dead links
pub fn inject_livereload(html: &str, options: RenderOptions, known_routes: Option<&HashSet<String>>) -> String {
    let mut result = html.to_string();
    let mut has_dead_links = false;

    // Mark dead links if we have known routes (dev mode)
    if let Some(routes) = known_routes {
        let (marked, had_dead) = mark_dead_links(&result, routes);
        result = marked;
        has_dead_links = had_dead;
    }

    if options.livereload {
        // Only inject dead link styles if there are actually dead links
        let styles = if has_dead_links { DEAD_LINK_STYLES } else { "" };

        // WASM-powered livereload client:
        // - Loads WASM module for DOM patching
        // - Binary WebSocket messages = patches (apply via WASM)
        // - Text "reload" message = full page reload (fallback)
        let livereload_script = r##"<script type="module">
(async function() {
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = protocol + '//' + window.location.host + '/__livereload';
    let ws;
    let reconnectTimer;
    let applyPatches = null;

    // Load WASM module
    try {
        const { default: init, apply_patches } = await import('/__livereload.js');
        await init('/__livereload.wasm');
        applyPatches = apply_patches;
        console.log('[livereload] WASM loaded, smart reload enabled');
    } catch (e) {
        console.warn('[livereload] WASM not available, using full reload:', e);
    }

    function connect() {
        ws = new WebSocket(wsUrl);
        ws.binaryType = 'arraybuffer';
        ws.onopen = function() {
            console.log('[livereload] connected');
            // Tell server which route we're viewing
            ws.send('route:' + window.location.pathname);
        };
        ws.onmessage = async function(event) {
            if (typeof event.data === 'string') {
                if (event.data === 'reload') {
                    console.log('[livereload] full reload');
                    window.location.reload();
                }
            } else if (event.data instanceof ArrayBuffer && applyPatches) {
                // Binary message = patches
                try {
                    const bytes = new Uint8Array(event.data);
                    const count = applyPatches(bytes);
                    console.log('[livereload] applied', count, 'patches');
                } catch (e) {
                    console.error('[livereload] patch error, falling back to reload:', e);
                    window.location.reload();
                }
            }
        };
        ws.onclose = function() {
            console.log('[livereload] disconnected, reconnecting...');
            clearTimeout(reconnectTimer);
            reconnectTimer = setTimeout(connect, 1000);
        };
    }
    connect();
})();
</script>"##;
        // Inject styles and script after <html> - always present even after minification
        result.replacen("<html", &format!("{styles}{livereload_script}<html"), 1)
    } else {
        result
    }
}

// ============================================================================
// Pure render functions for Salsa tracked queries
// ============================================================================

/// Pure function to render a page to HTML (for Salsa tracking)
/// Returns Result - caller decides whether to show error page (dev) or fail (prod)
pub fn try_render_page_to_html(
    page: &Page,
    site_tree: &SiteTree,
    templates: &HashMap<String, String>,
    data: Option<Value>,
) -> std::result::Result<String, String> {
    let mut loader = InMemoryLoader::new();
    for (path, content) in templates {
        loader.add(path.clone(), content.clone());
    }
    let mut engine = Engine::new(loader);

    let mut ctx = build_render_context(site_tree, data);
    ctx.set("page", page_to_value(page, site_tree));
    ctx.set(
        "current_path",
        Value::String(page.route.as_str().to_string()),
    );

    // Find the parent section for sidebar navigation
    if let Some(section) = find_parent_section(&page.route, site_tree) {
        ctx.set("section", section_to_value(section, site_tree));
    }

    engine
        .render("page.html", &ctx)
        .map_err(|e| format!("{e:?}"))
}

/// Render page - development mode (shows error page on failure)
pub fn render_page_to_html(
    page: &Page,
    site_tree: &SiteTree,
    templates: &HashMap<String, String>,
    data: Option<Value>,
) -> String {
    try_render_page_to_html(page, site_tree, templates, data)
        .unwrap_or_else(|e| render_error_page(&e))
}

/// Pure function to render a section to HTML (for Salsa tracking)
/// Returns Result - caller decides whether to show error page (dev) or fail (prod)
pub fn try_render_section_to_html(
    section: &Section,
    site_tree: &SiteTree,
    templates: &HashMap<String, String>,
    data: Option<Value>,
) -> std::result::Result<String, String> {
    let mut loader = InMemoryLoader::new();
    for (path, content) in templates {
        loader.add(path.clone(), content.clone());
    }
    let mut engine = Engine::new(loader);

    let mut ctx = build_render_context(site_tree, data);
    ctx.set("section", section_to_value(section, site_tree));
    ctx.set(
        "current_path",
        Value::String(section.route.as_str().to_string()),
    );
    // Set page to None so templates can use `{% if page %}` without error
    ctx.set("page", Value::None);

    let template_name = if section.route.as_str() == "/" {
        "index.html"
    } else {
        "section.html"
    };

    engine
        .render(template_name, &ctx)
        .map_err(|e| format!("{e:?}"))
}

/// Render section - development mode (shows error page on failure)
pub fn render_section_to_html(
    section: &Section,
    site_tree: &SiteTree,
    templates: &HashMap<String, String>,
    data: Option<Value>,
) -> String {
    try_render_section_to_html(section, site_tree, templates, data)
        .unwrap_or_else(|e| render_error_page(&e))
}

/// Marker that indicates a page contains a render error (for build mode detection)
pub const RENDER_ERROR_MARKER: &str = "<!-- DODECA_RENDER_ERROR -->";

/// Render a visible error page for development
///
/// TODO: This inline HTML is revolting. The error page styling should be
/// extracted into its own crate or at minimum a proper template file that
/// can be shared with the 404 page. The ANSI-to-HTML conversion should also
/// be a separate utility.
fn render_error_page(error: &str) -> String {
    let error_html = ansi_to_html(error);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
{RENDER_ERROR_MARKER}
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Template Error - dodeca</title>
    <style>
        body {{
            font-family: system-ui, -apple-system, sans-serif;
            background: #1a1a1a;
            color: #e5e5e5;
            margin: 0;
            padding: 2rem;
            min-height: 100vh;
        }}
        .error-container {{
            max-width: 900px;
            margin: 0 auto;
        }}
        h1 {{
            color: #e5e5e5;
            border-bottom: 1px solid #333;
            padding-bottom: 0.5rem;
            font-weight: 400;
        }}
        pre {{
            background: #0d0d0d;
            border: 1px solid #333;
            border-radius: 8px;
            padding: 1rem;
            overflow-x: auto;
            white-space: pre-wrap;
            word-wrap: break-word;
            font-size: 14px;
            line-height: 1.6;
            color: #ccc;
        }}
        .hint {{
            background: #252525;
            border-left: 3px solid #666;
            padding: 1rem;
            margin-top: 1.5rem;
            color: #aaa;
        }}
        .hint strong {{
            color: #e5e5e5;
        }}
    </style>
</head>
<body>
    <div class="error-container">
        <h1>Template Render Error</h1>
        <pre>{error_html}</pre>
        <div class="hint">
            <strong>Hint:</strong> Check your template syntax and ensure all referenced variables exist.
        </div>
    </div>
</body>
</html>"#
    )
}

/// Build the render context with config and global functions
fn build_render_context(site_tree: &SiteTree, data: Option<Value>) -> Context {
    let mut ctx = Context::new();

    // Add config
    let mut config_map = HashMap::new();
    config_map.insert("title".to_string(), Value::String("dodeca".to_string()));
    config_map.insert(
        "description".to_string(),
        Value::String("A fully incremental static site generator".to_string()),
    );
    config_map.insert("base_url".to_string(), Value::String("/".to_string()));
    ctx.set("config", Value::Dict(config_map));

    // Add data files (if any)
    if let Some(data_value) = data {
        ctx.set("data", data_value);
    } else {
        ctx.set("data", Value::Dict(HashMap::new()));
    }

    // Add root section for sidebar navigation
    if let Some(root) = site_tree.sections.get(&Route::root()) {
        ctx.set("root", section_to_value(root, site_tree));
    }

    // Register get_url function
    ctx.register_fn(
        "get_url",
        Box::new(move |_args, kwargs| {
            let path = kwargs
                .iter()
                .find(|(k, _)| k == "path")
                .map(|(_, v)| v.render_to_string())
                .unwrap_or_default();

            let url = if path.starts_with('/') {
                path
            } else if path.is_empty() {
                "/".to_string()
            } else {
                format!("/{path}")
            };
            Ok(Value::String(url))
        }),
    );

    // Register get_section function
    let sections = site_tree.sections.clone();
    let pages = site_tree.pages.clone();
    ctx.register_fn(
        "get_section",
        Box::new(move |_args, kwargs| {
            let path = kwargs
                .iter()
                .find(|(k, _)| k == "path")
                .map(|(_, v)| v.render_to_string())
                .unwrap_or_default();

            let route = path_to_route(&path);

            if let Some(section) = sections.get(&route) {
                let mut section_map = HashMap::new();
                section_map.insert(
                    "title".to_string(),
                    Value::String(section.title.as_str().to_string()),
                );
                section_map.insert(
                    "permalink".to_string(),
                    Value::String(section.route.as_str().to_string()),
                );
                section_map.insert("path".to_string(), Value::String(path.clone()));
                section_map.insert(
                    "content".to_string(),
                    Value::String(section.body_html.as_str().to_string()),
                );
                section_map.insert("toc".to_string(), headings_to_toc(&section.headings));

                let section_pages: Vec<Value> = pages
                    .values()
                    .filter(|p| p.section_route == section.route)
                    .map(|p| {
                        let mut page_map = HashMap::new();
                        page_map.insert(
                            "title".to_string(),
                            Value::String(p.title.as_str().to_string()),
                        );
                        page_map.insert(
                            "permalink".to_string(),
                            Value::String(p.route.as_str().to_string()),
                        );
                        page_map.insert(
                            "path".to_string(),
                            Value::String(route_to_path(p.route.as_str())),
                        );
                        page_map.insert("weight".to_string(), Value::Int(p.weight as i64));
                        page_map.insert("toc".to_string(), headings_to_toc(&p.headings));
                        Value::Dict(page_map)
                    })
                    .collect();
                section_map.insert("pages".to_string(), Value::List(section_pages));

                let subsections: Vec<Value> = sections
                    .values()
                    .filter(|s| {
                        s.route != section.route
                            && s.route.as_str().starts_with(section.route.as_str())
                            && s.route.as_str()[section.route.as_str().len()..]
                                .trim_matches('/')
                                .chars()
                                .filter(|c| *c == '/')
                                .count()
                                == 0
                    })
                    .map(|s| Value::String(route_to_path(s.route.as_str())))
                    .collect();
                section_map.insert("subsections".to_string(), Value::List(subsections));

                Ok(Value::Dict(section_map))
            } else {
                Ok(Value::None)
            }
        }),
    );

    ctx
}

/// Convert a heading to a Value dict with children field
fn heading_to_value(h: &Heading, children: Vec<Value>) -> Value {
    let mut map = HashMap::new();
    map.insert("title".to_string(), Value::String(h.title.clone()));
    map.insert("id".to_string(), Value::String(h.id.clone()));
    map.insert("level".to_string(), Value::Int(h.level as i64));
    map.insert("permalink".to_string(), Value::String(format!("#{}", h.id)));
    map.insert("children".to_string(), Value::List(children));
    Value::Dict(map)
}

/// Convert headings to a hierarchical TOC Value (Zola-style nested structure)
fn headings_to_toc(headings: &[Heading]) -> Value {
    build_toc_tree(headings)
}

/// Convert headings to hierarchical Value list for template context
fn headings_to_value(headings: &[Heading]) -> Value {
    build_toc_tree(headings)
}

/// Build a hierarchical tree from a flat list of headings
fn build_toc_tree(headings: &[Heading]) -> Value {
    if headings.is_empty() {
        return Value::List(vec![]);
    }

    // Find the minimum level to use as the "top level"
    let min_level = headings.iter().map(|h| h.level).min().unwrap_or(1);

    // Build tree recursively
    let (result, _) = build_toc_subtree(headings, 0, min_level);
    Value::List(result)
}

/// Recursively build TOC subtree, returns (list of Value nodes, next index to process)
fn build_toc_subtree(headings: &[Heading], start: usize, parent_level: u8) -> (Vec<Value>, usize) {
    let mut result = Vec::new();
    let mut i = start;

    while i < headings.len() {
        let h = &headings[i];

        // If we hit a heading at or above parent level (lower number), we're done with this subtree
        if h.level < parent_level {
            break;
        }

        // If this heading is at the expected level, add it with its children
        if h.level == parent_level {
            // Collect children (headings with level > parent_level until we hit another at parent_level)
            let (children, next_i) = build_toc_subtree(headings, i + 1, parent_level + 1);
            result.push(heading_to_value(h, children));
            i = next_i;
        } else {
            // Heading is deeper than expected - just move on
            i += 1;
        }
    }

    (result, i)
}

/// Build the ancestor chain for a page (ordered from root to immediate parent)
fn build_ancestors(section_route: &Route, site_tree: &SiteTree) -> Vec<Value> {
    let mut ancestors = Vec::new();
    let mut current = section_route.clone();

    // Walk up the route hierarchy, collecting all ancestor sections
    loop {
        if let Some(section) = site_tree.sections.get(&current) {
            let mut ancestor_map = HashMap::new();
            ancestor_map.insert(
                "title".to_string(),
                Value::String(section.title.as_str().to_string()),
            );
            ancestor_map.insert(
                "permalink".to_string(),
                Value::String(section.route.as_str().to_string()),
            );
            ancestor_map.insert(
                "path".to_string(),
                Value::String(route_to_path(section.route.as_str())),
            );
            ancestor_map.insert("weight".to_string(), Value::Int(section.weight as i64));
            ancestors.push(Value::Dict(ancestor_map));
        }

        match current.parent() {
            Some(parent) => current = parent,
            None => break,
        }
    }

    // Reverse so it's root -> ... -> immediate parent
    ancestors.reverse();
    ancestors
}

/// Convert a Page to a Value for template context
fn page_to_value(page: &Page, site_tree: &SiteTree) -> Value {
    let mut map = HashMap::new();
    map.insert(
        "title".to_string(),
        Value::String(page.title.as_str().to_string()),
    );
    map.insert(
        "content".to_string(),
        Value::String(page.body_html.as_str().to_string()),
    );
    map.insert(
        "permalink".to_string(),
        Value::String(page.route.as_str().to_string()),
    );
    map.insert(
        "path".to_string(),
        Value::String(route_to_path(page.route.as_str())),
    );
    map.insert("weight".to_string(), Value::Int(page.weight as i64));
    map.insert("toc".to_string(), headings_to_value(&page.headings));
    map.insert(
        "ancestors".to_string(),
        Value::List(build_ancestors(&page.section_route, site_tree)),
    );
    map.insert(
        "last_updated".to_string(),
        Value::Int(page.last_updated),
    );
    Value::Dict(map)
}

/// Convert a Section to a Value for template context
fn section_to_value(section: &Section, site_tree: &SiteTree) -> Value {
    let mut map = HashMap::new();
    map.insert(
        "title".to_string(),
        Value::String(section.title.as_str().to_string()),
    );
    map.insert(
        "content".to_string(),
        Value::String(section.body_html.as_str().to_string()),
    );
    map.insert(
        "permalink".to_string(),
        Value::String(section.route.as_str().to_string()),
    );
    map.insert(
        "path".to_string(),
        Value::String(route_to_path(section.route.as_str())),
    );
    map.insert("weight".to_string(), Value::Int(section.weight as i64));
    map.insert(
        "last_updated".to_string(),
        Value::Int(section.last_updated),
    );

    // Add pages in this section (sorted by weight, including their headings)
    let mut pages: Vec<&Page> = site_tree
        .pages
        .values()
        .filter(|p| p.section_route == section.route)
        .collect();
    pages.sort_by_key(|p| p.weight);
    let section_pages: Vec<Value> = pages
        .into_iter()
        .map(|p| {
            let mut page_map = HashMap::new();
            page_map.insert(
                "title".to_string(),
                Value::String(p.title.as_str().to_string()),
            );
            page_map.insert(
                "permalink".to_string(),
                Value::String(p.route.as_str().to_string()),
            );
            page_map.insert(
                "path".to_string(),
                Value::String(route_to_path(p.route.as_str())),
            );
            page_map.insert("weight".to_string(), Value::Int(p.weight as i64));
            page_map.insert("toc".to_string(), headings_to_value(&p.headings));
            Value::Dict(page_map)
        })
        .collect();
    map.insert("pages".to_string(), Value::List(section_pages));

    // Add subsections (full objects, sorted by weight)
    let mut child_sections: Vec<&Section> = site_tree
        .sections
        .values()
        .filter(|s| {
            s.route != section.route
                && s.route.as_str().starts_with(section.route.as_str())
                && s.route.as_str()[section.route.as_str().len()..]
                    .trim_matches('/')
                    .chars()
                    .filter(|c| *c == '/')
                    .count()
                    == 0
        })
        .collect();
    child_sections.sort_by_key(|s| s.weight);
    let subsections: Vec<Value> = child_sections
        .into_iter()
        .map(|s| subsection_to_value(s, site_tree))
        .collect();
    map.insert("subsections".to_string(), Value::List(subsections));
    map.insert("toc".to_string(), headings_to_value(&section.headings));

    Value::Dict(map)
}

/// Convert a subsection to a value (includes pages but not recursive subsections)
fn subsection_to_value(section: &Section, site_tree: &SiteTree) -> Value {
    let mut map = HashMap::new();
    map.insert(
        "title".to_string(),
        Value::String(section.title.as_str().to_string()),
    );
    map.insert(
        "permalink".to_string(),
        Value::String(section.route.as_str().to_string()),
    );
    map.insert("weight".to_string(), Value::Int(section.weight as i64));

    // Add pages in this section, sorted by weight
    let mut section_pages: Vec<&Page> = site_tree
        .pages
        .values()
        .filter(|p| p.section_route == section.route)
        .collect();
    section_pages.sort_by_key(|p| p.weight);

    let pages: Vec<Value> = section_pages
        .into_iter()
        .map(|p| {
            let mut page_map = HashMap::new();
            page_map.insert(
                "title".to_string(),
                Value::String(p.title.as_str().to_string()),
            );
            page_map.insert(
                "permalink".to_string(),
                Value::String(p.route.as_str().to_string()),
            );
            page_map.insert("weight".to_string(), Value::Int(p.weight as i64));
            Value::Dict(page_map)
        })
        .collect();
    map.insert("pages".to_string(), Value::List(pages));

    Value::Dict(map)
}

/// Convert a source path like "learn/_index.md" to a route like "/learn"
fn path_to_route(path: &str) -> Route {
    let mut p = path.to_string();

    // Remove .md extension
    if p.ends_with(".md") {
        p = p[..p.len() - 3].to_string();
    }

    // Handle _index
    if p.ends_with("/_index") {
        p = p[..p.len() - 7].to_string();
    } else if p == "_index" {
        p = String::new();
    }

    // Ensure leading slash, no trailing slash (except for root)
    if p.is_empty() {
        Route::root()
    } else {
        Route::new(format!("/{p}"))
    }
}

/// Convert a route like "/learn/" back to a path like "learn/_index.md"
fn route_to_path(route: &str) -> String {
    let r = route.trim_matches('/');
    if r.is_empty() {
        "_index.md".to_string()
    } else {
        format!("{r}/_index.md")
    }
}
