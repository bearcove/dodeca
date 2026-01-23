//! Dodeca HTML processing cell (cell-html)
//!
//! This cell handles all HTML transformations using hotmeal:
//! - Parsing and serialization
//! - URL rewriting (href, src, srcset attributes)
//! - Dead link marking
//! - Code button injection (copy + build info)
//! - Script/style injection
//! - Inline CSS/JS minification (via callbacks to host)
//! - HTML structural minification

use std::collections::{HashMap, HashSet};

use color_eyre::Result;
use hotmeal::{Document, LocalName, NodeId, NodeKind, QualName, Stem, StrTendril, ns};

use cell_html_proto::{
    CodeExecutionMetadata, HtmlHostClient, HtmlProcessInput, HtmlProcessResult, HtmlProcessor,
    HtmlProcessorDispatcher, HtmlResult, Injection,
};
use dodeca_cell_runtime::{ConnectionHandle, run_cell};

/// HTML processor implementation
#[derive(Clone)]
pub struct HtmlProcessorImpl {
    handle_cell: std::sync::Arc<std::sync::OnceLock<ConnectionHandle>>,
}

impl HtmlProcessorImpl {
    fn new(handle_cell: std::sync::Arc<std::sync::OnceLock<ConnectionHandle>>) -> Self {
        Self { handle_cell }
    }

    fn handle(&self) -> &ConnectionHandle {
        self.handle_cell.get().expect("handle not initialized yet")
    }

    /// Get a client for calling back to the host
    fn host_client(&self) -> HtmlHostClient {
        HtmlHostClient::new(self.handle().clone())
    }
}

impl HtmlProcessor for HtmlProcessorImpl {
    async fn process(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        input: HtmlProcessInput,
    ) -> HtmlProcessResult {
        let mut had_dead_links = false;
        let mut had_code_buttons = false;

        // Phase 1: All sync DOM work (before any await points)
        let html = {
            let tendril = StrTendril::from(input.html.as_str());
            let mut doc = hotmeal::parse(&tendril);

            // 1. URL rewriting
            if let Some(path_map) = &input.path_map {
                rewrite_urls_in_doc(&mut doc, path_map);
            }

            // 2. Dead link marking
            if let Some(known_routes) = &input.known_routes {
                had_dead_links = mark_dead_links_in_doc(&mut doc, known_routes);
            }

            // 3. Code button injection
            if let Some(code_metadata) = &input.code_metadata {
                had_code_buttons = inject_code_buttons_in_doc(&mut doc, code_metadata);
            }

            // 4. Content injections (on the tree)
            for injection in &input.injections {
                apply_injection(&mut doc, injection);
            }

            // Serialize - this produces an owned String
            doc.to_html()
        };
        // tendril and doc are dropped here, before any await

        // Phase 2: Async minification (if requested)
        let html = if let Some(ref minify_opts) = input.minify {
            let host = self.host_client();
            let mut current_html = html;

            if minify_opts.minify_inline_css {
                match minify_inline_css_string(&host, &current_html).await {
                    Ok(minified) => current_html = minified,
                    Err(e) => tracing::warn!("CSS minification failed: {}", e),
                }
            }

            if minify_opts.minify_inline_js {
                match minify_inline_js_string(&host, &current_html).await {
                    Ok(minified) => current_html = minified,
                    Err(e) => tracing::warn!("JS minification failed: {}", e),
                }
            }

            current_html
        } else {
            html
        };

        HtmlProcessResult::Success {
            html,
            had_dead_links,
            had_code_buttons,
        }
    }

    // === Legacy methods ===

    async fn rewrite_urls(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        html: String,
        path_map: HashMap<String, String>,
    ) -> HtmlResult {
        let tendril = StrTendril::from(html.as_str());
        let mut doc = hotmeal::parse(&tendril);
        rewrite_urls_in_doc(&mut doc, &path_map);
        HtmlResult::Success {
            html: doc.to_html(),
        }
    }

    async fn mark_dead_links(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        html: String,
        known_routes: HashSet<String>,
    ) -> HtmlResult {
        let tendril = StrTendril::from(html.as_str());
        let mut doc = hotmeal::parse(&tendril);
        let had_dead = mark_dead_links_in_doc(&mut doc, &known_routes);
        HtmlResult::SuccessWithFlag {
            html: doc.to_html(),
            flag: had_dead,
        }
    }

    async fn inject_code_buttons(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        html: String,
        code_metadata: HashMap<String, CodeExecutionMetadata>,
    ) -> HtmlResult {
        let tendril = StrTendril::from(html.as_str());
        let mut doc = hotmeal::parse(&tendril);
        let had_buttons = inject_code_buttons_in_doc(&mut doc, &code_metadata);
        HtmlResult::SuccessWithFlag {
            html: doc.to_html(),
            flag: had_buttons,
        }
    }
}

// ============================================================================
// Helper functions for attribute access
// ============================================================================

/// Get an attribute value from an element
fn get_attr(doc: &Document, node_id: NodeId, attr_name: &str) -> Option<String> {
    if let NodeKind::Element(elem) = &doc.get(node_id).kind {
        for (name, value) in &elem.attrs {
            if name.local.as_ref() == attr_name {
                return Some(value.as_ref().to_string());
            }
        }
    }
    None
}

/// Set an attribute on an element
fn set_attr(doc: &mut Document, node_id: NodeId, attr_name: &str, value: &str) {
    if let NodeKind::Element(elem) = &mut doc.get_mut(node_id).kind {
        let qname = QualName::new(None, ns!(), LocalName::from(attr_name));
        // Find existing and update, or add new
        if let Some((_, existing)) = elem
            .attrs
            .iter_mut()
            .find(|(n, _)| n.local.as_ref() == attr_name)
        {
            *existing = Stem::from(value.to_string());
        } else {
            elem.attrs.push((qname, Stem::from(value.to_string())));
        }
    }
}

/// Check if node is an element with the given tag name
fn is_element(doc: &Document, node_id: NodeId, tag: &str) -> bool {
    if let NodeKind::Element(elem) = &doc.get(node_id).kind {
        elem.tag.as_ref() == tag
    } else {
        false
    }
}

/// Get the tag name of an element (or None if not an element)
fn tag_name<'a>(doc: &'a Document, node_id: NodeId) -> Option<&'a str> {
    if let NodeKind::Element(elem) = &doc.get(node_id).kind {
        Some(elem.tag.as_ref())
    } else {
        None
    }
}

/// Get text content from a node (recursively)
fn get_text_content(doc: &Document, node_id: NodeId) -> String {
    let mut text = String::new();
    collect_text(doc, node_id, &mut text);
    text
}

fn collect_text(doc: &Document, node_id: NodeId, out: &mut String) {
    match &doc.get(node_id).kind {
        NodeKind::Text(t) => out.push_str(t.as_ref()),
        NodeKind::Element(_) => {
            for child_id in doc.children(node_id) {
                collect_text(doc, child_id, out);
            }
        }
        _ => {}
    }
}

// ============================================================================
// Inline CSS/JS minification (string-based, for use across await points)
// ============================================================================

/// Minify inline `<style>` content via host callback (string-based)
async fn minify_inline_css_string(host: &HtmlHostClient, html: &str) -> Result<String> {
    // Phase 1: Extract CSS content (sync, no await)
    let css_to_minify: Vec<(usize, String)> = {
        let tendril = StrTendril::from(html);
        let doc = hotmeal::parse(&tendril);

        let mut results = Vec::new();
        if let Some(head_id) = doc.head() {
            for (idx, node_id) in doc
                .children(head_id)
                .filter(|&id| is_element(&doc, id, "style"))
                .enumerate()
            {
                let text = get_text_content(&doc, node_id);
                if !text.trim().is_empty() {
                    results.push((idx, text));
                }
            }
        }
        results
    };
    // tendril dropped here

    if css_to_minify.is_empty() {
        return Ok(html.to_string());
    }

    // Phase 2: Minify CSS (async)
    let mut minified: HashMap<usize, String> = HashMap::new();
    for (idx, css) in css_to_minify {
        match host.minify_css(css.clone()).await {
            Ok(cell_html_proto::MinifyCssResult::Success { css: min_css }) => {
                minified.insert(idx, min_css);
            }
            Ok(cell_html_proto::MinifyCssResult::Error { message }) => {
                tracing::warn!("CSS minification error: {}", message);
            }
            Err(e) => {
                tracing::warn!("CSS minification RPC error: {}", e);
            }
        }
    }

    if minified.is_empty() {
        return Ok(html.to_string());
    }

    // Phase 3: Apply minified CSS (sync)
    let tendril = StrTendril::from(html);
    let mut doc = hotmeal::parse(&tendril);

    if let Some(head_id) = doc.head() {
        let style_nodes: Vec<NodeId> = doc
            .children(head_id)
            .filter(|&id| is_element(&doc, id, "style"))
            .collect();

        for (idx, node_id) in style_nodes.into_iter().enumerate() {
            if let Some(min_css) = minified.get(&idx) {
                replace_text_content(&mut doc, node_id, min_css);
            }
        }
    }

    Ok(doc.to_html())
}

/// Minify inline `<script>` content via host callback (string-based)
async fn minify_inline_js_string(host: &HtmlHostClient, html: &str) -> Result<String> {
    // Phase 1: Extract JS content (sync, no await)
    let js_to_minify: Vec<(usize, String)> = {
        let tendril = StrTendril::from(html);
        let doc = hotmeal::parse(&tendril);

        let mut results = Vec::new();
        if let Some(head_id) = doc.head() {
            for (idx, node_id) in doc
                .children(head_id)
                .filter(|&id| is_element(&doc, id, "script") && get_attr(&doc, id, "src").is_none())
                .enumerate()
            {
                let text = get_text_content(&doc, node_id);
                if !text.trim().is_empty() {
                    results.push((idx, text));
                }
            }
        }
        results
    };
    // tendril dropped here

    if js_to_minify.is_empty() {
        return Ok(html.to_string());
    }

    // Phase 2: Minify JS (async)
    let mut minified: HashMap<usize, String> = HashMap::new();
    for (idx, js) in js_to_minify {
        match host.minify_js(js.clone()).await {
            Ok(cell_html_proto::MinifyJsResult::Success { js: min_js }) => {
                minified.insert(idx, min_js);
            }
            Ok(cell_html_proto::MinifyJsResult::Error { message }) => {
                tracing::warn!("JS minification error: {}", message);
            }
            Err(e) => {
                tracing::warn!("JS minification RPC error: {}", e);
            }
        }
    }

    if minified.is_empty() {
        return Ok(html.to_string());
    }

    // Phase 3: Apply minified JS (sync)
    let tendril = StrTendril::from(html);
    let mut doc = hotmeal::parse(&tendril);

    if let Some(head_id) = doc.head() {
        let script_nodes: Vec<NodeId> = doc
            .children(head_id)
            .filter(|&id| is_element(&doc, id, "script") && get_attr(&doc, id, "src").is_none())
            .collect();

        for (idx, node_id) in script_nodes.into_iter().enumerate() {
            if let Some(min_js) = minified.get(&idx) {
                replace_text_content(&mut doc, node_id, min_js);
            }
        }
    }

    Ok(doc.to_html())
}

/// Replace all text content of an element with new text
fn replace_text_content(doc: &mut Document, node_id: NodeId, new_text: &str) {
    // Remove all existing children
    let children: Vec<NodeId> = doc.children(node_id).collect();
    for child in children {
        doc.remove(child);
    }
    // Add new text node
    let text_node = doc.create_text(new_text.to_string());
    doc.append_child(node_id, text_node);
}

// ============================================================================
// URL Rewriting
// ============================================================================

fn rewrite_urls_in_doc(doc: &mut Document, path_map: &HashMap<String, String>) {
    // Rewrite URLs in <head>
    if let Some(head_id) = doc.head() {
        rewrite_urls_in_subtree(doc, head_id, path_map);
    }

    // Rewrite URLs in <body>
    if let Some(body_id) = doc.body() {
        rewrite_urls_in_subtree(doc, body_id, path_map);
    }
}

fn rewrite_urls_in_subtree(
    doc: &mut Document,
    node_id: NodeId,
    path_map: &HashMap<String, String>,
) {
    // Collect children first to avoid borrow issues
    let children: Vec<NodeId> = doc.children(node_id).collect();

    // Process this node
    if let Some(tag) = tag_name(doc, node_id) {
        match tag {
            "a" | "link" => {
                if let Some(href) = get_attr(doc, node_id, "href") {
                    if let Some(new_url) = path_map.get(&href) {
                        set_attr(doc, node_id, "href", new_url);
                    }
                }
            }
            "script" => {
                if let Some(src) = get_attr(doc, node_id, "src") {
                    if let Some(new_url) = path_map.get(&src) {
                        set_attr(doc, node_id, "src", new_url);
                    }
                }
            }
            "img" => {
                if let Some(src) = get_attr(doc, node_id, "src") {
                    if let Some(new_url) = path_map.get(&src) {
                        set_attr(doc, node_id, "src", new_url);
                    }
                }
                // Handle srcset
                if let Some(srcset) = get_attr(doc, node_id, "srcset") {
                    let new_srcset = rewrite_srcset(&srcset, path_map);
                    set_attr(doc, node_id, "srcset", &new_srcset);
                }
            }
            "source" => {
                if let Some(srcset) = get_attr(doc, node_id, "srcset") {
                    let new_srcset = rewrite_srcset(&srcset, path_map);
                    set_attr(doc, node_id, "srcset", &new_srcset);
                }
            }
            "video" | "audio" | "iframe" => {
                if let Some(src) = get_attr(doc, node_id, "src") {
                    if let Some(new_url) = path_map.get(&src) {
                        set_attr(doc, node_id, "src", new_url);
                    }
                }
            }
            _ => {}
        }
    }

    // Recurse into children
    for child_id in children {
        rewrite_urls_in_subtree(doc, child_id, path_map);
    }
}

fn rewrite_srcset(srcset: &str, path_map: &HashMap<String, String>) -> String {
    srcset
        .split(',')
        .map(|entry| {
            let entry = entry.trim();
            let parts: Vec<&str> = entry.split_whitespace().collect();
            if parts.is_empty() {
                return entry.to_string();
            }

            let url = parts[0];
            let descriptor = parts.get(1).copied().unwrap_or("");
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

// ============================================================================
// Dead Link Marking
// ============================================================================

fn mark_dead_links_in_doc(doc: &mut Document, known_routes: &HashSet<String>) -> bool {
    let mut had_dead = false;

    if let Some(body_id) = doc.body() {
        // Collect all <a> elements first
        let mut anchors = Vec::new();
        collect_anchors(doc, body_id, &mut anchors);

        for node_id in anchors {
            if let Some(href) = get_attr(doc, node_id, "href") {
                if is_dead_link(&href, known_routes) {
                    set_attr(doc, node_id, "data-dead", "true");
                    had_dead = true;
                }
            }
        }
    }

    had_dead
}

fn collect_anchors(doc: &Document, node_id: NodeId, anchors: &mut Vec<NodeId>) {
    if is_element(doc, node_id, "a") {
        anchors.push(node_id);
    }
    for child_id in doc.children(node_id) {
        collect_anchors(doc, child_id, anchors);
    }
}

fn is_dead_link(href: &str, known_routes: &HashSet<String>) -> bool {
    // Skip external links, anchors, mailto, etc.
    if href.starts_with("http://")
        || href.starts_with("https://")
        || href.starts_with('#')
        || href.starts_with("mailto:")
        || href.starts_with("tel:")
        || href.starts_with("javascript:")
        || href.starts_with("/__")
        || !href.starts_with('/')
    {
        return false;
    }

    // Skip static files
    let static_extensions = [
        ".css", ".js", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".woff", ".woff2", ".ttf",
        ".eot", ".pdf", ".zip", ".tar", ".gz", ".webp", ".jxl", ".xml", ".txt", ".md", ".wasm",
    ];

    if static_extensions.iter().any(|ext| href.ends_with(ext)) {
        return false;
    }

    let path = href.split('#').next().unwrap_or(href);
    if path.is_empty() {
        return false;
    }

    let target = normalize_route(path);

    // Check if route exists
    !(known_routes.contains(&target)
        || known_routes.contains(&format!("{}/", target.trim_end_matches('/')))
        || known_routes.contains(target.trim_end_matches('/')))
}

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

// ============================================================================
// Code Button Injection
// ============================================================================

fn inject_code_buttons_in_doc(
    doc: &mut Document,
    code_metadata: &HashMap<String, CodeExecutionMetadata>,
) -> bool {
    let mut had_buttons = false;

    if let Some(body_id) = doc.body() {
        // Collect all <pre> elements and .code-block divs
        let mut targets = Vec::new();
        collect_code_targets(doc, body_id, &mut targets);

        for target in targets {
            match target {
                CodeTarget::Pre(node_id) => {
                    let code_text = get_text_content(doc, node_id);
                    let normalized = normalize_code_for_matching(&code_text);

                    // Add position:relative to pre element
                    let existing_style = get_attr(doc, node_id, "style").unwrap_or_default();
                    if !existing_style.contains("position") {
                        set_attr(
                            doc,
                            node_id,
                            "style",
                            &format!("position:relative;{}", existing_style),
                        );
                    }

                    // Create and append buttons
                    if let Some(meta) = code_metadata.get(&normalized) {
                        let btn = create_build_info_button(doc, meta);
                        doc.append_child(node_id, btn);
                    }
                    let copy_btn = create_copy_button(doc);
                    doc.append_child(node_id, copy_btn);

                    had_buttons = true;
                }
                CodeTarget::CodeBlockDiv(node_id) => {
                    // Find the pre inside and get its code text
                    let mut code_text = String::new();
                    for child_id in doc.children(node_id) {
                        if is_element(doc, child_id, "pre") {
                            code_text = get_text_content(doc, child_id);
                            break;
                        }
                    }

                    if !code_text.is_empty() {
                        let normalized = normalize_code_for_matching(&code_text);

                        // Add buttons to the div
                        if let Some(meta) = code_metadata.get(&normalized) {
                            let btn = create_build_info_button(doc, meta);
                            doc.append_child(node_id, btn);
                        }
                        let copy_btn = create_copy_button(doc);
                        doc.append_child(node_id, copy_btn);

                        had_buttons = true;
                    }
                }
            }
        }
    }

    had_buttons
}

enum CodeTarget {
    Pre(NodeId),
    CodeBlockDiv(NodeId),
}

fn collect_code_targets(doc: &Document, node_id: NodeId, targets: &mut Vec<CodeTarget>) {
    if let Some(tag) = tag_name(doc, node_id) {
        if tag == "pre" {
            targets.push(CodeTarget::Pre(node_id));
            return; // Don't recurse into pre
        }
        if tag == "div" {
            if let Some(class) = get_attr(doc, node_id, "class") {
                if class.contains("code-block") {
                    targets.push(CodeTarget::CodeBlockDiv(node_id));
                    return; // Don't recurse into code-block
                }
            }
        }
    }

    for child_id in doc.children(node_id) {
        collect_code_targets(doc, child_id, targets);
    }
}

fn normalize_code_for_matching(code: &str) -> String {
    code.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn create_copy_button(doc: &mut Document) -> NodeId {
    let btn = doc.create_element("button");
    set_attr(doc, btn, "class", "copy-btn");
    let text = doc.create_text("Copy".to_string());
    doc.append_child(btn, text);
    btn
}

fn create_build_info_button(doc: &mut Document, meta: &CodeExecutionMetadata) -> NodeId {
    let rustc_short = meta
        .rustc_version
        .lines()
        .next()
        .unwrap_or(&meta.rustc_version);

    let btn = doc.create_element("button");
    set_attr(doc, btn, "class", "build-info-btn verified");
    set_attr(
        doc,
        btn,
        "title",
        &format!("Verified: {}", html_escape::encode_text(rustc_short)),
    );

    // Store metadata as data attribute for JS to use
    let json = metadata_to_json(meta);
    set_attr(doc, btn, "data-build-info", &json);

    let text = doc.create_text("\u{2139}".to_string()); // Unicode info symbol
    doc.append_child(btn, text);
    btn
}

fn metadata_to_json(meta: &CodeExecutionMetadata) -> String {
    let deps_json: Vec<String> = meta
        .dependencies
        .iter()
        .map(|d| {
            let source = match &d.source {
                cell_html_proto::DependencySource::CratesIo => "crates.io".to_string(),
                cell_html_proto::DependencySource::Git { url, commit } => {
                    format!("git:{}@{}", url, &commit[..7.min(commit.len())])
                }
                cell_html_proto::DependencySource::Path { path } => format!("path:{}", path),
            };
            format!(
                r#"{{"name":"{}","version":"{}","source":"{}"}}"#,
                json_escape(&d.name),
                json_escape(&d.version),
                json_escape(&source)
            )
        })
        .collect();

    format!(
        r#"{{"rustc_version":"{}","cargo_version":"{}","target":"{}","timestamp":"{}","cache_hit":{},"platform":"{}","arch":"{}","dependencies":[{}]}}"#,
        json_escape(&meta.rustc_version),
        json_escape(&meta.cargo_version),
        json_escape(&meta.target),
        json_escape(&meta.timestamp),
        meta.cache_hit,
        json_escape(&meta.platform),
        json_escape(&meta.arch),
        deps_json.join(",")
    )
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

// ============================================================================
// Injection helpers
// ============================================================================

/// Apply a typed injection to the HTML document tree
fn apply_injection(doc: &mut Document, injection: &Injection) {
    match injection {
        Injection::HeadStyle { css } => {
            if let Some(head_id) = doc.head() {
                let style = doc.create_element("style");
                let text = doc.create_text(css.clone());
                doc.append_child(style, text);
                doc.append_child(head_id, style);
            }
        }
        Injection::HeadScript { js, module } => {
            if let Some(head_id) = doc.head() {
                let script = doc.create_element("script");
                if *module {
                    set_attr(doc, script, "type", "module");
                }
                let text = doc.create_text(js.clone());
                doc.append_child(script, text);
                doc.append_child(head_id, script);
            }
        }
        Injection::BodyScript { js, module } => {
            if let Some(body_id) = doc.body() {
                let script = doc.create_element("script");
                if *module {
                    set_attr(doc, script, "type", "module");
                }
                let text = doc.create_text(js.clone());
                doc.append_child(script, text);
                doc.append_child(body_id, script);
            }
        }
    }
}

// ============================================================================
// Cell Setup
// ============================================================================

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("html", |handle| {
        let processor = HtmlProcessorImpl::new(handle);
        HtmlProcessorDispatcher::new(processor)
    })
}
