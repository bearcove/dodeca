use crate::cells::{inject_code_buttons_cell, render_template_cell};
use crate::db::{
    CodeExecutionMetadata, CodeExecutionResult, DependencySourceInfo, Heading, Page, Section,
    SiteTree,
};
use crate::template_host::{RenderContext, RenderContextGuard};
use crate::types::Route;
use crate::url_rewrite::mark_dead_links;
use facet_value::{VArray, VObject, VString, Value};

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// Re-export for backwards compatibility
pub use crate::error_pages::RENDER_ERROR_MARKER;

/// Get base_url from global config, defaulting to "/" for local development
fn get_base_url() -> String {
    crate::config::global_config()
        .map(|c| c.base_url.clone())
        .unwrap_or_else(|| "/".to_string())
}

/// Find the nearest parent section for a route (for page context)
fn find_parent_section<'a>(route: &Route, site_tree: &'a SiteTree) -> Option<&'a Section> {
    let mut current = route.clone();
    loop {
        if let Some(section) = site_tree.sections.get(&current)
            && current != *route
        {
            return Some(section);
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

/// Generate syntax highlighting CSS with media queries for light/dark themes
fn generate_syntax_highlight_css(light_theme_css: &str, dark_theme_css: &str) -> String {
    format!(
        r#"<style>
/* Arborium syntax highlighting - Light theme */
@media (prefers-color-scheme: light) {{
{light_theme_css}
}}

/* Arborium syntax highlighting - Dark theme */
@media (prefers-color-scheme: dark) {{
{dark_theme_css}
}}
</style>"#
    )
}

/// CSS for copy button on code blocks
const COPY_BUTTON_STYLES: &str = r##"<style>
.code-block {
    position: relative;
}
.code-block .copy-btn {
    position: absolute;
    top: 0.5rem;
    right: 0.5rem;
    padding: 0.25rem 0.5rem;
    font-size: 0.75rem;
    background: rgba(80,80,95,0.8);
    border: 1px solid rgba(255,255,255,0.2);
    border-radius: 0.25rem;
    color: #c0caf5;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.15s;
    z-index: 10;
}
.code-block:hover .copy-btn { opacity: 1; }
.code-block .copy-btn:hover { background: rgba(80,80,95,0.95); }
.code-block .copy-btn.copied { background: rgba(50,160,50,0.9); }
</style>"##;

/// JavaScript for copy button functionality - uses event delegation for all copy buttons
const COPY_BUTTON_SCRIPT: &str = r##"<script>
document.addEventListener('click', async (e) => {
    const btn = e.target.closest('.copy-btn');
    if (!btn) return;
    const pre = btn.closest('.code-block') || btn.closest('pre');
    if (!pre) return;
    const code = pre.querySelector('code')?.textContent || pre.textContent;
    await navigator.clipboard.writeText(code);
    btn.textContent = 'Copied!';
    btn.classList.add('copied');
    setTimeout(() => {
        btn.textContent = 'Copy';
        btn.classList.remove('copied');
    }, 2000);
});
</script>"##;

/// CSS and JS for build info icon on code blocks
const BUILD_INFO_STYLES: &str = r##"<style>
.code-block .build-info-btn {
    position: sticky;
    float: right;
    top: 0.5rem;
    right: 3.5rem;
    padding: 0.25rem;
    font-size: 0.75rem;
    background: rgba(255,255,255,0.1);
    border: 1px solid rgba(255,255,255,0.2);
    border-radius: 0.25rem;
    color: inherit;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.15s;
    line-height: 1;
    z-index: 10;
}
.code-block:hover .build-info-btn { opacity: 1; }
.code-block .build-info-btn:hover { background: rgba(255,255,255,0.2); }
.code-block .build-info-btn.verified { border-color: rgba(50,205,50,0.5); }
.build-info-popup {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    background: #1a1a2e;
    border: 1px solid rgba(255,255,255,0.2);
    border-radius: 0.5rem;
    padding: 1.5rem 2rem;
    width: 90vw;
    max-width: 800px;
    max-height: 80vh;
    overflow-y: auto;
    z-index: 10000;
    color: #e0e0e0;
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
    box-shadow: 0 25px 50px -12px rgba(0,0,0,0.5);
}
.build-info-popup h3 {
    margin: 0 0 1rem 0;
    color: #fff;
    font-size: 0.95rem;
    display: flex;
    align-items: center;
    gap: 0.5rem;
}
.build-info-popup .close-btn {
    position: sticky;
    float: right;
    top: 0.75rem;
    right: 0.75rem;
    background: none;
    border: none;
    color: #888;
    font-size: 1.25rem;
    cursor: pointer;
    padding: 0.25rem;
}
.build-info-popup .close-btn:hover { color: #fff; }
.build-info-popup dl {
    margin: 0;
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.4rem 1rem;
}
.build-info-popup dt {
    color: #888;
    font-weight: 500;
}
.build-info-popup dd {
    margin: 0;
    word-break: break-all;
}
.build-info-popup .deps-list {
    max-height: 250px;
    overflow-y: auto;
    background: rgba(0,0,0,0.2);
    padding: 0.5rem 0.75rem;
    border-radius: 0.25rem;
    font-size: 0.7rem;
}
.build-info-popup .deps-list div {
    padding: 0.2rem 0;
    display: flex;
    align-items: center;
    gap: 0.5rem;
}
.build-info-popup .field-icon {
    width: 14px;
    height: 14px;
    vertical-align: -2px;
    margin-right: 0.25rem;
    opacity: 0.7;
}
.build-info-popup .deps-list a,
.build-info-popup .deps-list .dep-local {
    color: #e0e0e0;
    text-decoration: none;
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
}
.build-info-popup .deps-list a:hover {
    color: #fff;
}
.build-info-popup .deps-list a:hover .dep-icon {
    color: #fff;
}
.build-info-popup .deps-list .dep-icon {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    color: #888;
}
.build-info-popup .deps-list .dep-name {
    font-weight: 500;
}
.build-info-popup .deps-list .dep-version {
    color: #888;
    font-weight: normal;
}
.build-info-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0,0,0,0.5);
    z-index: 9999;
}
</style>"##;

/// JavaScript for build info popup (injected when there are build info buttons)
/// The buttons are injected server-side, this JS just handles showing the popup
const BUILD_INFO_POPUP_SCRIPT: &str = r##"<script>
(function() {
    // SVG icons
    var cratesIoIcon = '<svg class="dep-icon" viewBox="0 0 512 512"><path fill="currentColor" d="M239.1 6.3l-208 78c-18.7 7-31.1 25-31.1 45v225.1c0 18.2 10.3 34.8 26.5 42.9l208 104c13.5 6.8 29.4 6.8 42.9 0l208-104c16.3-8.1 26.5-24.8 26.5-42.9V129.3c0-20-12.4-37.9-31.1-44.9l-208-78C262 2.2 250 2.2 239.1 6.3zM256 68.4l192 72v1.1l-192 78-192-78v-1.1l192-72zm32 356V275.5l160-65v160.4l-160 53.5z"/></svg>';
    var gitIcon = '<svg class="dep-icon" viewBox="0 0 16 16"><path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>';
    var pathIcon = '<svg class="dep-icon" viewBox="0 0 16 16"><path fill="currentColor" d="M1.75 1A1.75 1.75 0 000 2.75v10.5C0 14.216.784 15 1.75 15h12.5A1.75 1.75 0 0016 13.25v-8.5A1.75 1.75 0 0014.25 3H7.5a.25.25 0 01-.2-.1l-.9-1.2C6.07 1.26 5.55 1 5 1H1.75z"/></svg>';
    var rustcIcon = '<svg class="field-icon" viewBox="0 0 16 16"><path fill="currentColor" d="M8 0l1.5 2.5L12 1.5l-.5 2.5 2.5.5-1.5 2.5L15 8l-2.5 1.5.5 2.5-2.5-.5-.5 2.5-2.5-1.5L8 16l-1.5-2.5L4 14.5l.5-2.5-2.5-.5 1.5-2.5L1 8l2.5-1.5L3 4l2.5.5.5-2.5 2.5 1.5L8 0zm0 5a3 3 0 100 6 3 3 0 000-6z"/></svg>';
    var targetIcon = '<svg class="field-icon" viewBox="0 0 16 16"><path fill="currentColor" d="M8 0a8 8 0 100 16A8 8 0 008 0zm0 2a6 6 0 110 12A6 6 0 018 2zm0 2a4 4 0 100 8 4 4 0 000-8zm0 2a2 2 0 110 4 2 2 0 010-4z"/></svg>';
    var clockIcon = '<svg class="field-icon" viewBox="0 0 16 16"><path fill="currentColor" d="M8 0a8 8 0 100 16A8 8 0 008 0zm0 2a6 6 0 110 12A6 6 0 018 2zm-.5 2v4.5l3 2 .75-1.125-2.25-1.5V4h-1.5z"/></svg>';
    var depsIcon = '<svg class="field-icon" viewBox="0 0 16 16"><path fill="currentColor" d="M1 2.5A1.5 1.5 0 012.5 1h3a1.5 1.5 0 011.5 1.5v3A1.5 1.5 0 015.5 7h-3A1.5 1.5 0 011 5.5v-3zm8 0A1.5 1.5 0 0110.5 1h3A1.5 1.5 0 0115 2.5v3A1.5 1.5 0 0113.5 7h-3A1.5 1.5 0 019 5.5v-3zm-8 8A1.5 1.5 0 012.5 9h3A1.5 1.5 0 017 10.5v3A1.5 1.5 0 015.5 15h-3A1.5 1.5 0 011 13.5v-3zm8 0A1.5 1.5 0 0110.5 9h3a1.5 1.5 0 011.5 1.5v3a1.5 1.5 0 01-1.5 1.5h-3A1.5 1.5 0 019 13.5v-3z"/></svg>';

    function formatLocalTime(isoString) {
        try {
            var date = new Date(isoString);
            return date.toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' }) + ' at ' + date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
        } catch (e) {
            return isoString;
        }
    }

    function escapeHtml(str) {
        return String(str).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
    }

    window.showBuildInfoPopup = function(info) {
        // Remove existing popup
        var existing = document.querySelector('.build-info-overlay');
        if (existing) existing.remove();

        var overlay = document.createElement('div');
        overlay.className = 'build-info-overlay';

        var popup = document.createElement('div');
        popup.className = 'build-info-popup';

        var depsHtml = '';
        if (info.deps && info.deps.length > 0) {
            depsHtml = '<dt>' + depsIcon + ' Dependencies</dt><dd><div class="deps-list">';
            info.deps.forEach(function(d) {
                var icon, link, versionDisplay;
                var src = d.source;
                if (src.type === 'crates.io') {
                    icon = cratesIoIcon;
                    link = 'https://crates.io/crates/' + encodeURIComponent(d.name) + '/' + encodeURIComponent(d.version);
                    versionDisplay = d.version;
                    depsHtml += '<div><a href="' + link + '" target="_blank" rel="noopener" title="View on crates.io">' + icon + ' <span class="dep-name">' + escapeHtml(d.name) + '</span> <span class="dep-version">' + escapeHtml(versionDisplay) + '</span></a></div>';
                } else if (src.type === 'git') {
                    icon = gitIcon;
                    // Generate proper commit link for GitHub repos
                    var commitShort = src.commit.substring(0, 8);
                    versionDisplay = d.version + ' @ ' + commitShort;
                    if (src.url.indexOf('github.com') !== -1) {
                        // GitHub: link directly to the commit
                        link = src.url.replace(/\.git$/, '') + '/tree/' + src.commit;
                    } else {
                        // Other git hosts: just link to the repo
                        link = src.url;
                    }
                    depsHtml += '<div><a href="' + escapeHtml(link) + '" target="_blank" rel="noopener" title="View commit ' + escapeHtml(src.commit) + '">' + icon + ' <span class="dep-name">' + escapeHtml(d.name) + '</span> <span class="dep-version">' + escapeHtml(versionDisplay) + '</span></a></div>';
                } else {
                    // path dependency
                    icon = pathIcon;
                    versionDisplay = d.version;
                    depsHtml += '<div><span class="dep-local">' + icon + ' <span class="dep-name">' + escapeHtml(d.name) + '</span> <span class="dep-version">' + escapeHtml(versionDisplay) + '</span></span></div>';
                }
            });
            depsHtml += '</div></dd>';
        }

        popup.innerHTML =
            '<button class="close-btn" aria-label="Close">&times;</button>' +
            '<h3>&#x2705; Build Verified</h3>' +
            '<dl>' +
            '<dt>' + rustcIcon + ' Compiler</dt><dd>' + escapeHtml(info.rustc) + '</dd>' +
            '<dt>' + rustcIcon + ' Cargo</dt><dd>' + escapeHtml(info.cargo) + '</dd>' +
            '<dt>' + targetIcon + ' Target</dt><dd>' + escapeHtml(info.target) + '</dd>' +
            '<dt>' + clockIcon + ' Built</dt><dd>' + formatLocalTime(info.timestamp) + (info.cacheHit ? ' (cached)' : '') + '</dd>' +
            depsHtml +
            '</dl>';

        overlay.appendChild(popup);
        document.body.appendChild(overlay);

        function close() {
            overlay.remove();
        }

        overlay.addEventListener('click', function(e) {
            if (e.target === overlay) close();
        });
        popup.querySelector('.close-btn').addEventListener('click', close);
        document.addEventListener('keydown', function handler(e) {
            if (e.key === 'Escape') {
                close();
                document.removeEventListener('keydown', handler);
            }
        });
    };
})();
</script>"##;

/// Convert internal CodeExecutionMetadata to cell protocol type
fn convert_metadata_to_proto(
    meta: &CodeExecutionMetadata,
) -> cell_html_proto::CodeExecutionMetadata {
    cell_html_proto::CodeExecutionMetadata {
        rustc_version: meta.rustc_version.clone(),
        cargo_version: meta.cargo_version.clone(),
        target: meta.target.clone(),
        timestamp: meta.timestamp.clone(),
        cache_hit: meta.cache_hit,
        platform: meta.platform.clone(),
        arch: meta.arch.clone(),
        dependencies: meta
            .dependencies
            .iter()
            .map(|d| cell_html_proto::ResolvedDependency {
                name: d.name.clone(),
                version: d.version.clone(),
                source: match &d.source {
                    DependencySourceInfo::CratesIo => cell_html_proto::DependencySource::CratesIo,
                    DependencySourceInfo::Git { url, commit } => {
                        cell_html_proto::DependencySource::Git {
                            url: url.clone(),
                            commit: commit.clone(),
                        }
                    }
                    DependencySourceInfo::Path { path } => {
                        cell_html_proto::DependencySource::Path { path: path.clone() }
                    }
                },
            })
            .collect(),
    }
}

/// Build a map from normalized code text to metadata for code blocks with execution results
fn build_code_metadata_map(
    results: &[CodeExecutionResult],
) -> HashMap<String, cell_html_proto::CodeExecutionMetadata> {
    let mut map = HashMap::new();
    for result in results {
        if let Some(ref metadata) = result.metadata {
            let normalized = result.code.trim().to_string();
            map.insert(normalized, convert_metadata_to_proto(metadata));
        }
    }
    map
}

/// Inject copy buttons and build info buttons into code blocks using the html cell.
/// This is a single-pass operation that also sets position:relative inline on pre elements.
async fn inject_code_buttons(
    html: &str,
    code_metadata: &HashMap<String, cell_html_proto::CodeExecutionMetadata>,
) -> (String, bool) {
    match inject_code_buttons_cell(html.to_string(), code_metadata.clone()).await {
        Ok((result, had_buttons)) => (result, had_buttons),
        Err(e) => {
            tracing::warn!("Code button injection failed: {}", e);
            (html.to_string(), false)
        }
    }
}

/// Inject livereload script, copy buttons, and optionally mark dead links
#[allow(dead_code)]
pub async fn inject_livereload(
    html: &str,
    options: RenderOptions,
    known_routes: Option<&HashSet<String>>,
) -> String {
    inject_livereload_with_build_info(html, options, known_routes, &[], &[]).await
}

/// Inject livereload script, copy buttons, build info, head injections, and optionally mark dead links
pub async fn inject_livereload_with_build_info(
    html: &str,
    options: RenderOptions,
    known_routes: Option<&HashSet<String>>,
    code_execution_results: &[CodeExecutionResult],
    head_injections: &[String],
) -> String {
    let mut result = html.to_string();
    let mut has_dead_links = false;

    // Mark dead links if we have known routes (dev mode)
    if let Some(routes) = known_routes {
        let (marked, had_dead) = mark_dead_links(&result, routes).await;
        result = marked;
        has_dead_links = had_dead;
    }

    // Build the code metadata map and inject buttons (copy + build info) into code blocks
    let code_metadata = build_code_metadata_map(code_execution_results);
    let (with_buttons, _) = inject_code_buttons(&result, &code_metadata).await;
    result = with_buttons;

    // Only include build info popup script if we have code execution results
    let build_info_assets = if !code_execution_results.is_empty() {
        format!("{BUILD_INFO_STYLES}{BUILD_INFO_POPUP_SCRIPT}")
    } else {
        String::new()
    };

    // Always inject copy button script and syntax highlighting styles for code blocks
    // Inject after opening <head> tag so content is properly inside <head>
    let config = crate::config::global_config().expect("Config not initialized");
    let syntax_css = generate_syntax_highlight_css(&config.light_theme_css, &config.dark_theme_css);
    let term_css = format!("<style>\n{}</style>", cell_term_proto::generate_css());
    let head_injection_html = head_injections.join("");
    let scripts_to_inject = format!(
        "{syntax_css}{term_css}{COPY_BUTTON_STYLES}{COPY_BUTTON_SCRIPT}{build_info_assets}{head_injection_html}"
    );
    result = hotmeal_server::inject_into_head(&result, &scripts_to_inject);

    if options.livereload {
        // Only inject dead link styles if there are actually dead links
        let styles = if has_dead_links { DEAD_LINK_STYLES } else { "" };

        // Get cache-busted URLs for devtools assets
        let (js_url, wasm_url) = crate::serve::devtools_urls();

        // Load dodeca-devtools WASM module which handles:
        // - WebSocket connection to /__dodeca
        // - DOM patching for live updates
        // - CSS hot reload
        // - Error overlay with source context
        // - Scope explorer and REPL (future)
        let devtools_script = format!(
            r##"<script type="module">
(async function() {{
    try {{
        const {{ default: init, mount_devtools }} = await import('{js_url}');
        await init('{wasm_url}');
        mount_devtools();
        console.log('[dodeca] devtools loaded');
    }} catch (e) {{
        console.error('[dodeca] failed to load devtools:', e);
    }}
}})();
</script>"##
        );
        // Inject styles and script into <head>
        hotmeal_server::inject_into_head(&result, &format!("{styles}{devtools_script}"))
    } else {
        result
    }
}

// ============================================================================
// Cell-based rendering (uses gingembre cell for template processing)
// ============================================================================

/// Something that can be rendered (page or section)
pub enum Renderable<'a> {
    Page(&'a Page),
    Section(&'a Section),
}

impl<'a> Renderable<'a> {
    /// Get the template name for this renderable
    fn template_name(&self) -> &str {
        match self {
            Renderable::Page(page) => page.template.as_deref().unwrap_or("page.html"),
            Renderable::Section(section) => section.template.as_deref().unwrap_or_else(|| {
                if section.route.as_str() == "/" {
                    "index.html"
                } else {
                    "section.html"
                }
            }),
        }
    }

    /// Get the route for this renderable
    fn route(&self) -> &Route {
        match self {
            Renderable::Page(page) => &page.route,
            Renderable::Section(section) => &section.route,
        }
    }

    /// Build the initial context value for template rendering
    fn build_context(&self, site_tree: &SiteTree) -> Value {
        let base_url = get_base_url();
        let mut obj = VObject::new();

        // Add config
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
        config_map.insert(
            VString::from("description"),
            Value::from(site_description.as_str()),
        );
        config_map.insert(VString::from("base_url"), Value::from(base_url.as_str()));
        obj.insert(VString::from("config"), Value::from(config_map));

        // Add page and section based on what we're rendering
        match self {
            Renderable::Page(page) => {
                obj.insert(VString::from("page"), page_to_value(page, site_tree));
                // Find and add parent section
                if let Some(section) = find_parent_section(&page.route, site_tree) {
                    obj.insert(
                        VString::from("section"),
                        section_to_value(section, site_tree, &base_url),
                    );
                }
            }
            Renderable::Section(section) => {
                obj.insert(VString::from("page"), Value::NULL);
                obj.insert(
                    VString::from("section"),
                    section_to_value(section, site_tree, &base_url),
                );
            }
        }

        // Add current_path
        obj.insert(
            VString::from("current_path"),
            Value::from(self.route().as_str()),
        );

        // Add root section if available
        if let Some(root) = site_tree.sections.get(&Route::root()) {
            obj.insert(
                VString::from("root"),
                section_to_value(root, site_tree, &base_url),
            );
        }

        obj.into()
    }
}

/// Render a page or section via the gingembre cell.
pub async fn try_render_via_cell(
    renderable: Renderable<'_>,
    site_tree: &SiteTree,
    templates: HashMap<String, String>,
) -> std::result::Result<String, cell_gingembre_proto::TemplateRenderError> {
    let template_name = renderable.template_name();
    let route = renderable.route().clone();

    // Get database from task-local
    let db = crate::db::TASK_DB.try_with(|db| db.clone()).map_err(|_| {
        let bt = std::backtrace::Backtrace::force_capture();
        cell_gingembre_proto::TemplateRenderError {
            message: format!("Database not available in task-local context\n\nBacktrace:\n{bt}"),
            location: None,
            help: None,
        }
    })?;

    // Create render context with templates and site_tree
    let context = RenderContext::new(templates, db, Arc::new(site_tree.clone()));
    let guard = RenderContextGuard::new(context);

    // Build initial context
    let initial_context = renderable.build_context(site_tree);

    // Render via cell
    let result = match render_template_cell(guard.id(), template_name, initial_context).await {
        Ok(cell_gingembre_proto::RenderResult::Success { html }) => Ok(html),
        Ok(cell_gingembre_proto::RenderResult::Error { error }) => Err(error),
        Err(e) => Err(cell_gingembre_proto::TemplateRenderError {
            message: format!("Gingembre cell error: {}", e),
            location: None,
            help: None,
        }),
    };

    // Debug check for missing doctype
    if let Ok(ref html) = result {
        if !html.contains("<!DOCTYPE") {
            tracing::error!(
                route = %route.as_str(),
                html_len = html.len(),
                html_preview = %html.chars().take(300).collect::<String>(),
                html_tail = %html.chars().rev().take(100).collect::<String>().chars().rev().collect::<String>(),
                "CELL template rendered WITHOUT doctype!"
            );
        }
    }

    result
}

/// Render page via cell - returns Result with structured error
pub async fn render_page_via_cell(
    page: &Page,
    site_tree: &SiteTree,
    templates: HashMap<String, String>,
) -> Result<String, cell_gingembre_proto::TemplateRenderError> {
    try_render_via_cell(Renderable::Page(page), site_tree, templates).await
}

/// Render section via cell - returns Result with structured error
pub async fn render_section_via_cell(
    section: &Section,
    site_tree: &SiteTree,
    templates: HashMap<String, String>,
) -> Result<String, cell_gingembre_proto::TemplateRenderError> {
    try_render_via_cell(Renderable::Section(section), site_tree, templates).await
}

/// Convert a heading to a Value dict with children field
fn heading_to_value(h: &Heading, children: Vec<Value>) -> Value {
    let mut map = VObject::new();
    map.insert(VString::from("title"), Value::from(h.title.as_str()));
    map.insert(VString::from("id"), Value::from(h.id.as_str()));
    map.insert(VString::from("level"), Value::from(h.level as i64));
    map.insert(
        VString::from("permalink"),
        Value::from(format!("#{}", h.id).as_str()),
    );
    map.insert(VString::from("children"), VArray::from_iter(children));
    map.into()
}

/// Convert headings to a hierarchical TOC Value (Zola-style nested structure)
pub fn headings_to_toc(headings: &[Heading]) -> Value {
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
    let base_url = get_base_url();

    // Walk up the route hierarchy, collecting all ancestor sections
    loop {
        if let Some(section) = site_tree.sections.get(&current) {
            // Skip the content root ("/") - it's not useful in breadcrumbs
            if section.route.as_str() != "/" {
                let mut ancestor_map = VObject::new();
                ancestor_map.insert(VString::from("title"), Value::from(section.title.as_str()));
                ancestor_map.insert(
                    VString::from("permalink"),
                    Value::from(make_permalink(&base_url, section.route.as_str()).as_str()),
                );
                ancestor_map.insert(VString::from("path"), Value::from(section.route.as_str()));
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
pub fn page_to_value(page: &Page, site_tree: &SiteTree) -> Value {
    use facet_value::DestructuredRef;

    let base_url = get_base_url();
    let mut map = VObject::new();
    map.insert(VString::from("title"), Value::from(page.title.as_str()));
    let body_html = page.body_html.as_str();
    if body_html.is_empty() {
        tracing::warn!(
            route = %page.route.as_str(),
            title = %page.title,
            "page_to_value: body_html is empty!"
        );
    } else {
        tracing::debug!(
            route = %page.route.as_str(),
            body_html_len = body_html.len(),
            body_html_preview = %body_html.chars().take(100).collect::<String>(),
            "page_to_value: body_html content"
        );
    }
    map.insert(VString::from("content"), Value::from(body_html));
    map.insert(
        VString::from("permalink"),
        Value::from(make_permalink(&base_url, page.route.as_str()).as_str()),
    );
    map.insert(VString::from("path"), Value::from(page.route.as_str()));
    map.insert(VString::from("weight"), Value::from(page.weight as i64));
    map.insert(VString::from("toc"), headings_to_value(&page.headings));
    map.insert(
        VString::from("ancestors"),
        VArray::from_iter(build_ancestors(&page.section_route, site_tree)),
    );
    map.insert(
        VString::from("last_updated"),
        Value::from(page.last_updated),
    );

    // Extract description from extra.description for Zola compatibility
    let description = match page.extra.destructure_ref() {
        DestructuredRef::Object(obj) => obj.get("description").cloned(),
        _ => None,
    };
    if let Some(desc) = description {
        map.insert(VString::from("description"), desc);
    }

    map.insert(VString::from("extra"), page.extra.clone());
    map.into()
}

/// Convert a Section to a Value for template context
pub fn section_to_value(section: &Section, site_tree: &SiteTree, base_url: &str) -> Value {
    let mut map = VObject::new();
    map.insert(VString::from("title"), Value::from(section.title.as_str()));
    map.insert(
        VString::from("content"),
        Value::from(section.body_html.as_str()),
    );
    map.insert(
        VString::from("permalink"),
        Value::from(make_permalink(base_url, section.route.as_str()).as_str()),
    );
    map.insert(VString::from("path"), Value::from(section.route.as_str()));
    map.insert(VString::from("weight"), Value::from(section.weight as i64));
    map.insert(
        VString::from("last_updated"),
        Value::from(section.last_updated),
    );
    map.insert(
        VString::from("ancestors"),
        VArray::from_iter(build_ancestors(&section.route, site_tree)),
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
            use facet_value::DestructuredRef;
            let mut page_map = VObject::new();
            page_map.insert(VString::from("title"), Value::from(p.title.as_str()));
            page_map.insert(
                VString::from("permalink"),
                Value::from(make_permalink(base_url, p.route.as_str()).as_str()),
            );
            page_map.insert(VString::from("path"), Value::from(p.route.as_str()));
            page_map.insert(VString::from("weight"), Value::from(p.weight as i64));
            page_map.insert(VString::from("toc"), headings_to_value(&p.headings));
            // Extract description from extra.description for Zola compatibility
            if let DestructuredRef::Object(obj) = p.extra.destructure_ref() {
                if let Some(desc) = obj.get("description") {
                    page_map.insert(VString::from("description"), desc.clone());
                }
            }
            page_map.insert(VString::from("extra"), p.extra.clone());
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
        .map(|s| subsection_to_value(s, site_tree, base_url))
        .collect();
    map.insert(VString::from("subsections"), VArray::from_iter(subsections));
    map.insert(VString::from("toc"), headings_to_value(&section.headings));
    map.insert(VString::from("extra"), section.extra.clone());

    map.into()
}

/// Convert a subsection to a value (includes pages but not recursive subsections)
fn subsection_to_value(section: &Section, site_tree: &SiteTree, base_url: &str) -> Value {
    let mut map = VObject::new();
    map.insert(VString::from("title"), Value::from(section.title.as_str()));
    map.insert(
        VString::from("permalink"),
        Value::from(make_permalink(base_url, section.route.as_str()).as_str()),
    );
    map.insert(VString::from("path"), Value::from(section.route.as_str()));
    map.insert(VString::from("weight"), Value::from(section.weight as i64));
    map.insert(VString::from("extra"), section.extra.clone());

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
            page_map.insert(VString::from("path"), Value::from(p.route.as_str()));
            page_map.insert(
                VString::from("permalink"),
                Value::from(make_permalink(base_url, p.route.as_str()).as_str()),
            );
            page_map.insert(VString::from("weight"), Value::from(p.weight as i64));
            page_map.insert(VString::from("extra"), p.extra.clone());
            page_map.into()
        })
        .collect();
    map.insert(VString::from("pages"), VArray::from_iter(pages));

    map.into()
}

/// Convert a source path like "learn/_index.md" to a route like "/learn"
/// Also accepts routes directly (starting with "/") for convenience.
pub fn path_to_route(path: &str) -> Route {
    let mut p = path.to_string();

    // Remove .md extension
    if p.ends_with(".md") {
        p = p[..p.len() - 3].to_string();
    }

    // Handle _index (with or without leading slash)
    if p.ends_with("/_index") {
        p = p[..p.len() - 7].to_string();
    } else if p == "_index" || p == "/_index" {
        p = String::new();
    }

    // Normalize: remove trailing slashes
    p = p.trim_end_matches('/').to_string();

    // Ensure leading slash, no trailing slash (except for root)
    if p.is_empty() {
        Route::root()
    } else if p.starts_with('/') {
        Route::new(p)
    } else {
        Route::new(format!("/{p}"))
    }
}

/// Convert a route like "/learn/" back to a path like "learn/_index.md"
pub fn route_to_path(route: &str) -> String {
    let r = route.trim_matches('/');
    if r.is_empty() {
        "_index.md".to_string()
    } else {
        format!("{r}/_index.md")
    }
}

/// Create a permalink from base_url and route
/// e.g., `("https://example.com", "/spec/core/")` -> `"https://example.com/spec/core/"`
/// e.g., `("/", "/spec/core/")` -> `"/spec/core/"`
fn make_permalink(base_url: &str, route: &str) -> String {
    if base_url == "/" {
        route.to_string()
    } else {
        // Remove trailing slash from base_url to avoid double slashes
        let base = base_url.trim_end_matches('/');
        format!("{base}{route}")
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::db::{
        CodeExecutionMetadata, CodeExecutionResult, DependencySourceInfo, ResolvedDependencyInfo,
    };

    fn make_test_result(
        code: &str,
        metadata: Option<CodeExecutionMetadata>,
    ) -> CodeExecutionResult {
        CodeExecutionResult {
            source_path: "test.md".to_string(),
            line: 1,
            language: "rust".to_string(),
            code: code.to_string(),
            status: crate::db::CodeExecutionStatus::Success,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: 100,
            error: None,
            metadata,
        }
    }

    #[tokio::test]
    async fn test_inject_code_buttons_with_build_info() {
        // Note: This test requires the html cell to be running
        // Without the cell, the function returns the original HTML with no buttons
        let html = r#"<html><body><pre><code>fn main() {}</code></pre></body></html>"#;

        let metadata = CodeExecutionMetadata {
            rustc_version: "rustc 1.83.0-nightly".to_string(),
            cargo_version: "cargo 1.83.0".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            cache_hit: false,
            platform: "linux".to_string(),
            arch: "x86_64".to_string(),
            dependencies: vec![ResolvedDependencyInfo {
                name: "serde".to_string(),
                version: "1.0.0".to_string(),
                source: DependencySourceInfo::CratesIo,
            }],
        };

        let results = vec![make_test_result("fn main() {}", Some(metadata))];

        let code_metadata = build_code_metadata_map(&results);
        let (result, had_buttons) = inject_code_buttons(html, &code_metadata).await;

        // With cell: buttons are injected (copy + build info)
        // Without cell: returns original HTML
        if had_buttons {
            assert!(
                result.contains(r#"class="copy-btn""#),
                "Should contain copy button"
            );
            assert!(
                result.contains(r#"class="build-info-btn verified""#),
                "Should contain build info button"
            );
            assert!(
                result.contains("showBuildInfoPopup"),
                "Should have onclick handler"
            );
            assert!(
                result.contains("rustc 1.83.0-nightly"),
                "Should contain rustc version in title"
            );
            assert!(
                result.contains(r#"style="position:relative""#),
                "Should have inline position:relative"
            );
        } else {
            assert_eq!(result, html, "Without cell, HTML should be unchanged");
        }
    }

    #[tokio::test]
    async fn test_inject_code_buttons_no_build_info_match() {
        // Note: This test requires the html cell to be running
        let html = r#"<html><body><pre><code>fn other() {}</code></pre></body></html>"#;

        let metadata = CodeExecutionMetadata {
            rustc_version: "rustc 1.83.0-nightly".to_string(),
            cargo_version: "cargo 1.83.0".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            cache_hit: false,
            platform: "linux".to_string(),
            arch: "x86_64".to_string(),
            dependencies: vec![],
        };

        // Different code than what's in the HTML
        let results = vec![make_test_result("fn main() {}", Some(metadata))];

        let code_metadata = build_code_metadata_map(&results);
        let (result, had_buttons) = inject_code_buttons(html, &code_metadata).await;

        // Copy button should still be added, but no build-info button
        if had_buttons {
            assert!(
                result.contains(r#"class="copy-btn""#),
                "Should contain copy button"
            );
            assert!(
                !result.contains("build-info-btn"),
                "Should not contain build info button"
            );
        }
    }

    #[tokio::test]
    async fn test_inject_code_buttons_empty_metadata() {
        let html = r#"<html><body><pre><code>fn main() {}</code></pre></body></html>"#;

        let code_metadata: HashMap<String, cell_html_proto::CodeExecutionMetadata> = HashMap::new();
        let (result, had_buttons) = inject_code_buttons(html, &code_metadata).await;

        // Copy button should still be added even with no build info
        if had_buttons {
            assert!(
                result.contains(r#"class="copy-btn""#),
                "Should contain copy button"
            );
            assert!(
                !result.contains("build-info-btn"),
                "Should not contain build info button"
            );
        } else {
            assert_eq!(result, html, "Without cell, HTML should be unchanged");
        }
    }
}
