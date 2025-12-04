use crate::db::{Heading, Page, Section, SiteTree};
use crate::error_pages::render_error_page;
use crate::template::{
    Context, DataResolver, Engine, InMemoryLoader, TemplateLoader, VArray, VObject, VString,
    Value, ValueExt,
};
use crate::types::Route;
use crate::url_rewrite::mark_dead_links;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// Re-export for backwards compatibility
pub use crate::error_pages::RENDER_ERROR_MARKER;

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
        // - Text "css:/path" message = CSS hot reload
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

    // Hot-reload CSS by swapping stylesheet links
    function hotReloadCss(newPath) {
        // Find all stylesheet links that match /main.*.css pattern
        const links = document.querySelectorAll('link[rel="stylesheet"]');
        let updated = 0;
        for (const link of links) {
            const href = link.getAttribute('href');
            // Match /main.*.css or /css/style.*.css patterns
            if (href && (href.match(/^\/main\.[^/]+\.css/) || href.match(/^\/css\/style\.[^/]+\.css/) || href === '/main.css' || href === '/css/style.css')) {
                // Create new link element
                const newLink = document.createElement('link');
                newLink.rel = 'stylesheet';
                newLink.href = newPath;
                // Insert new link after old one, then remove old after load
                link.parentNode.insertBefore(newLink, link.nextSibling);
                newLink.onload = () => {
                    link.remove();
                    console.log('[livereload] CSS updated:', newPath);
                };
                newLink.onerror = () => {
                    console.error('[livereload] CSS load failed:', newPath);
                    newLink.remove();
                };
                updated++;
            }
        }
        if (updated === 0) {
            console.warn('[livereload] No matching stylesheets found for CSS update');
        }
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
                } else if (event.data.startsWith('css:')) {
                    // CSS hot reload
                    const newPath = event.data.substring(4);
                    console.log('[livereload] CSS changed:', newPath);
                    hotReloadCss(newPath);
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
// Render functions for Salsa tracked queries
// ============================================================================

/// Render a page to HTML using a template loader.
/// Returns Result - caller decides whether to show error page (dev) or fail (prod)
pub fn try_render_page_with_loader<L: TemplateLoader>(
    page: &Page,
    site_tree: &SiteTree,
    loader: L,
    data: Option<Value>,
) -> std::result::Result<String, String> {
    let mut engine = Engine::new(loader);

    let mut ctx = build_render_context(site_tree, data);
    ctx.set("page", page_to_value(page, site_tree));
    ctx.set("current_path", Value::from(page.route.as_str()));

    // Find the parent section for sidebar navigation
    if let Some(section) = find_parent_section(&page.route, site_tree) {
        ctx.set("section", section_to_value(section, site_tree));
    }

    engine
        .render("page.html", &ctx)
        .map_err(|e| format!("{e:?}"))
}

/// Render page with a loader - development mode (shows error page on failure)
pub fn render_page_with_loader<L: TemplateLoader>(
    page: &Page,
    site_tree: &SiteTree,
    loader: L,
    data: Option<Value>,
) -> String {
    try_render_page_with_loader(page, site_tree, loader, data)
        .unwrap_or_else(|e| render_error_page(&e))
}

/// Render a section to HTML using a template loader.
/// Returns Result - caller decides whether to show error page (dev) or fail (prod)
pub fn try_render_section_with_loader<L: TemplateLoader>(
    section: &Section,
    site_tree: &SiteTree,
    loader: L,
    data: Option<Value>,
) -> std::result::Result<String, String> {
    let mut engine = Engine::new(loader);

    let mut ctx = build_render_context(site_tree, data);
    ctx.set("section", section_to_value(section, site_tree));
    ctx.set("current_path", Value::from(section.route.as_str()));
    // Set page to NULL so templates can use `{% if page %}` without error
    ctx.set("page", Value::NULL);

    let template_name = if section.route.as_str() == "/" {
        "index.html"
    } else {
        "section.html"
    };

    engine
        .render(template_name, &ctx)
        .map_err(|e| format!("{e:?}"))
}

/// Render section with a loader - development mode (shows error page on failure)
pub fn render_section_with_loader<L: TemplateLoader>(
    section: &Section,
    site_tree: &SiteTree,
    loader: L,
    data: Option<Value>,
) -> String {
    try_render_section_with_loader(section, site_tree, loader, data)
        .unwrap_or_else(|e| render_error_page(&e))
}

// ============================================================================
// Lazy data resolver variants (for fine-grained Salsa tracking)
// ============================================================================

/// Render a page to HTML with lazy data resolver.
/// Each data path access becomes a tracked Salsa dependency.
pub fn try_render_page_with_resolver<L: TemplateLoader>(
    page: &Page,
    site_tree: &SiteTree,
    loader: L,
    resolver: Arc<dyn DataResolver>,
) -> std::result::Result<String, String> {
    let mut engine = Engine::new(loader);

    let mut ctx = build_render_context_with_resolver(site_tree, resolver);
    ctx.set("page", page_to_value(page, site_tree));
    ctx.set("current_path", Value::from(page.route.as_str()));

    // Find the parent section for sidebar navigation
    if let Some(section) = find_parent_section(&page.route, site_tree) {
        ctx.set("section", section_to_value(section, site_tree));
    }

    engine
        .render("page.html", &ctx)
        .map_err(|e| format!("{e:?}"))
}

/// Render page with lazy resolver - development mode (shows error page on failure)
pub fn render_page_with_resolver<L: TemplateLoader>(
    page: &Page,
    site_tree: &SiteTree,
    loader: L,
    resolver: Arc<dyn DataResolver>,
) -> String {
    try_render_page_with_resolver(page, site_tree, loader, resolver)
        .unwrap_or_else(|e| render_error_page(&e))
}

/// Render a section to HTML with lazy data resolver.
/// Each data path access becomes a tracked Salsa dependency.
pub fn try_render_section_with_resolver<L: TemplateLoader>(
    section: &Section,
    site_tree: &SiteTree,
    loader: L,
    resolver: Arc<dyn DataResolver>,
) -> std::result::Result<String, String> {
    let mut engine = Engine::new(loader);

    let mut ctx = build_render_context_with_resolver(site_tree, resolver);
    ctx.set("section", section_to_value(section, site_tree));
    ctx.set("current_path", Value::from(section.route.as_str()));
    // Set page to NULL so templates can use `{% if page %}` without error
    ctx.set("page", Value::NULL);

    let template_name = if section.route.as_str() == "/" {
        "index.html"
    } else {
        "section.html"
    };

    engine
        .render(template_name, &ctx)
        .map_err(|e| format!("{e:?}"))
}

/// Render section with lazy resolver - development mode (shows error page on failure)
pub fn render_section_with_resolver<L: TemplateLoader>(
    section: &Section,
    site_tree: &SiteTree,
    loader: L,
    resolver: Arc<dyn DataResolver>,
) -> String {
    try_render_section_with_resolver(section, site_tree, loader, resolver)
        .unwrap_or_else(|e| render_error_page(&e))
}

// ============================================================================
// Convenience functions using HashMap (backward compatibility)
// ============================================================================

/// Helper to create an InMemoryLoader from a HashMap
fn loader_from_map(templates: &HashMap<String, String>) -> InMemoryLoader {
    let mut loader = InMemoryLoader::new();
    for (path, content) in templates {
        loader.add(path.clone(), content.clone());
    }
    loader
}

/// Pure function to render a page to HTML (for Salsa tracking)
/// Returns Result - caller decides whether to show error page (dev) or fail (prod)
pub fn try_render_page_to_html(
    page: &Page,
    site_tree: &SiteTree,
    templates: &HashMap<String, String>,
    data: Option<Value>,
) -> std::result::Result<String, String> {
    try_render_page_with_loader(page, site_tree, loader_from_map(templates), data)
}

/// Render page - development mode (shows error page on failure)
pub fn render_page_to_html(
    page: &Page,
    site_tree: &SiteTree,
    templates: &HashMap<String, String>,
    data: Option<Value>,
) -> String {
    render_page_with_loader(page, site_tree, loader_from_map(templates), data)
}

/// Pure function to render a section to HTML (for Salsa tracking)
/// Returns Result - caller decides whether to show error page (dev) or fail (prod)
pub fn try_render_section_to_html(
    section: &Section,
    site_tree: &SiteTree,
    templates: &HashMap<String, String>,
    data: Option<Value>,
) -> std::result::Result<String, String> {
    try_render_section_with_loader(section, site_tree, loader_from_map(templates), data)
}

/// Render section - development mode (shows error page on failure)
pub fn render_section_to_html(
    section: &Section,
    site_tree: &SiteTree,
    templates: &HashMap<String, String>,
    data: Option<Value>,
) -> String {
    render_section_with_loader(section, site_tree, loader_from_map(templates), data)
}

/// Build the render context with config and global functions
fn build_render_context(site_tree: &SiteTree, data: Option<Value>) -> Context {
    let mut ctx = build_render_context_base(site_tree);

    // Add data files (if any)
    if let Some(data_value) = data {
        ctx.set("data", data_value);
    } else {
        ctx.set("data", Value::from(VObject::new()));
    }

    ctx
}

/// Build the render context with a lazy data resolver
///
/// Instead of loading all data upfront, this uses a resolver that will
/// fetch data on-demand. Each data path access becomes a tracked Salsa dependency.
fn build_render_context_with_resolver(
    site_tree: &SiteTree,
    resolver: Arc<dyn DataResolver>,
) -> Context {
    let mut ctx = build_render_context_base(site_tree);

    // Set the data resolver for lazy data loading
    ctx.set_data_resolver(resolver);

    ctx
}

/// Base context builder with config and global functions (no data)
fn build_render_context_base(site_tree: &SiteTree) -> Context {
    let mut ctx = Context::new();

    // Add config - derive title/description from root section's frontmatter
    let mut config_map = VObject::new();
    let (site_title, site_description) = site_tree
        .sections
        .get(&Route::root())
        .map(|root| {
            (
                root.title.to_string(),
                root.description.clone().unwrap_or_default(),
            )
        })
        .unwrap_or_else(|| ("Untitled".to_string(), String::new()));
    config_map.insert(VString::from("title"), Value::from(site_title.as_str()));
    config_map.insert(VString::from("description"), Value::from(site_description.as_str()));
    config_map.insert(VString::from("base_url"), Value::from("/"));
    ctx.set("config", Value::from(config_map));

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
            Ok(Value::from(url.as_str()))
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
                let mut section_map = VObject::new();
                section_map.insert(
                    VString::from("title"),
                    Value::from(section.title.as_str()),
                );
                section_map.insert(
                    VString::from("permalink"),
                    Value::from(section.route.as_str()),
                );
                section_map.insert(VString::from("path"), Value::from(path.as_str()));
                section_map.insert(
                    VString::from("content"),
                    Value::from(section.body_html.as_str()),
                );
                section_map.insert(VString::from("toc"), headings_to_toc(&section.headings));

                let section_pages: Vec<Value> = pages
                    .values()
                    .filter(|p| p.section_route == section.route)
                    .map(|p| {
                        let mut page_map = VObject::new();
                        page_map.insert(
                            VString::from("title"),
                            Value::from(p.title.as_str()),
                        );
                        page_map.insert(
                            VString::from("permalink"),
                            Value::from(p.route.as_str()),
                        );
                        page_map.insert(
                            VString::from("path"),
                            Value::from(route_to_path(p.route.as_str()).as_str()),
                        );
                        page_map.insert(VString::from("weight"), Value::from(p.weight as i64));
                        page_map.insert(VString::from("toc"), headings_to_toc(&p.headings));
                        page_map.into()
                    })
                    .collect();
                section_map.insert(VString::from("pages"), VArray::from_iter(section_pages));

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
                    .map(|s| Value::from(route_to_path(s.route.as_str()).as_str()))
                    .collect();
                section_map.insert(VString::from("subsections"), VArray::from_iter(subsections));

                Ok(section_map.into())
            } else {
                Ok(Value::NULL)
            }
        }),
    );

    ctx
}

/// Convert a heading to a Value dict with children field
fn heading_to_value(h: &Heading, children: Vec<Value>) -> Value {
    let mut map = VObject::new();
    map.insert(VString::from("title"), Value::from(h.title.as_str()));
    map.insert(VString::from("id"), Value::from(h.id.as_str()));
    map.insert(VString::from("level"), Value::from(h.level as i64));
    map.insert(VString::from("permalink"), Value::from(format!("#{}", h.id).as_str()));
    map.insert(VString::from("children"), VArray::from_iter(children));
    map.into()
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
        return VArray::new().into();
    }

    // Find the minimum level to use as the "top level"
    let min_level = headings.iter().map(|h| h.level).min().unwrap_or(1);

    // Build tree recursively
    let (result, _) = build_toc_subtree(headings, 0, min_level);
    VArray::from_iter(result).into()
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
/// Note: The content root ("/") is excluded from ancestors to avoid noisy breadcrumbs.
fn build_ancestors(section_route: &Route, site_tree: &SiteTree) -> Vec<Value> {
    let mut ancestors = Vec::new();
    let mut current = section_route.clone();

    // Walk up the route hierarchy, collecting all ancestor sections
    loop {
        if let Some(section) = site_tree.sections.get(&current) {
            // Skip the content root ("/") - it's not useful in breadcrumbs
            if section.route.as_str() != "/" {
                let mut ancestor_map = VObject::new();
                ancestor_map.insert(
                    VString::from("title"),
                    Value::from(section.title.as_str()),
                );
                ancestor_map.insert(
                    VString::from("permalink"),
                    Value::from(section.route.as_str()),
                );
                ancestor_map.insert(
                    VString::from("path"),
                    Value::from(route_to_path(section.route.as_str()).as_str()),
                );
                ancestor_map.insert(VString::from("weight"), Value::from(section.weight as i64));
                ancestors.push(ancestor_map.into());
            }
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
    let mut map = VObject::new();
    map.insert(VString::from("title"), Value::from(page.title.as_str()));
    map.insert(VString::from("content"), Value::from(page.body_html.as_str()));
    map.insert(VString::from("permalink"), Value::from(page.route.as_str()));
    map.insert(
        VString::from("path"),
        Value::from(route_to_path(page.route.as_str()).as_str()),
    );
    map.insert(VString::from("weight"), Value::from(page.weight as i64));
    map.insert(VString::from("toc"), headings_to_value(&page.headings));
    map.insert(
        VString::from("ancestors"),
        VArray::from_iter(build_ancestors(&page.section_route, site_tree)),
    );
    map.insert(VString::from("last_updated"), Value::from(page.last_updated));
    map.into()
}

/// Convert a Section to a Value for template context
fn section_to_value(section: &Section, site_tree: &SiteTree) -> Value {
    let mut map = VObject::new();
    map.insert(VString::from("title"), Value::from(section.title.as_str()));
    map.insert(VString::from("content"), Value::from(section.body_html.as_str()));
    map.insert(VString::from("permalink"), Value::from(section.route.as_str()));
    map.insert(
        VString::from("path"),
        Value::from(route_to_path(section.route.as_str()).as_str()),
    );
    map.insert(VString::from("weight"), Value::from(section.weight as i64));
    map.insert(VString::from("last_updated"), Value::from(section.last_updated));

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
            let mut page_map = VObject::new();
            page_map.insert(VString::from("title"), Value::from(p.title.as_str()));
            page_map.insert(VString::from("permalink"), Value::from(p.route.as_str()));
            page_map.insert(
                VString::from("path"),
                Value::from(route_to_path(p.route.as_str()).as_str()),
            );
            page_map.insert(VString::from("weight"), Value::from(p.weight as i64));
            page_map.insert(VString::from("toc"), headings_to_value(&p.headings));
            page_map.into()
        })
        .collect();
    map.insert(VString::from("pages"), VArray::from_iter(section_pages));

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
    map.insert(VString::from("subsections"), VArray::from_iter(subsections));
    map.insert(VString::from("toc"), headings_to_value(&section.headings));

    map.into()
}

/// Convert a subsection to a value (includes pages but not recursive subsections)
fn subsection_to_value(section: &Section, site_tree: &SiteTree) -> Value {
    let mut map = VObject::new();
    map.insert(VString::from("title"), Value::from(section.title.as_str()));
    map.insert(VString::from("permalink"), Value::from(section.route.as_str()));
    map.insert(VString::from("weight"), Value::from(section.weight as i64));

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
            let mut page_map = VObject::new();
            page_map.insert(VString::from("title"), Value::from(p.title.as_str()));
            page_map.insert(VString::from("permalink"), Value::from(p.route.as_str()));
            page_map.insert(VString::from("weight"), Value::from(p.weight as i64));
            page_map.into()
        })
        .collect();
    map.insert(VString::from("pages"), VArray::from_iter(pages));

    map.into()
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
