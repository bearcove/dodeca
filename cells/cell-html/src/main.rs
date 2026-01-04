//! Dodeca HTML processing cell (cell-html)
//!
//! This cell handles all HTML transformations using facet-html:
//! - Parsing and serialization
//! - URL rewriting (href, src, srcset attributes)
//! - Dead link marking
//! - Code button injection (copy + build info)
//! - Script/style injection
//! - Inline CSS/JS minification (via callbacks to host)
//! - HTML structural minification
//! - DOM diffing for live reload

use std::collections::{HashMap, HashSet};

use color_eyre::Result;
use facet_html::{self as fhtml};
use facet_html_dom::*;

use cell_html_proto::{
    CodeExecutionMetadata, DiffResult, HtmlDiffResult, HtmlHostClient, HtmlProcessInput,
    HtmlProcessResult, HtmlProcessor, HtmlProcessorServer, HtmlResult, Injection,
};
use rapace::transport::shm::ShmTransport;
use rapace_cell::CellSession;
use std::sync::Arc;

mod diff;

/// HTML processor implementation
pub struct HtmlProcessorImpl {
    /// Session for making host callbacks
    session: Arc<CellSession>,
}

impl HtmlProcessorImpl {
    pub fn new(session: Arc<CellSession>) -> Self {
        Self { session }
    }

    /// Get a client for calling back to the host
    fn host_client(&self) -> HtmlHostClient<ShmTransport> {
        HtmlHostClient::new(self.session.clone())
    }
}

impl HtmlProcessor for HtmlProcessorImpl {
    async fn process(&self, input: HtmlProcessInput) -> HtmlProcessResult {
        // Parse HTML once
        let mut doc: Html = match fhtml::from_str(&input.html) {
            Ok(doc) => doc,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    html_len = input.html.len(),
                    "facet-html parse failed"
                );
                return HtmlProcessResult::Error {
                    message: format!("HTML parse error: {}", e),
                };
            }
        };

        let mut had_dead_links = false;
        let mut had_code_buttons = false;

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

        // 4. Inline CSS/JS minification (via host callbacks)
        if let Some(ref minify_opts) = input.minify {
            let host = self.host_client();
            if minify_opts.minify_inline_css
                && let Err(e) = minify_inline_css(&host, &mut doc).await
            {
                tracing::warn!("CSS minification failed: {}", e);
            }
            if minify_opts.minify_inline_js
                && let Err(e) = minify_inline_js(&host, &mut doc).await
            {
                tracing::warn!("JS minification failed: {}", e);
            }
        }

        // 5. Content injections (on the tree)
        for injection in &input.injections {
            apply_injection(&mut doc, injection);
        }

        // 6. Serialize back to string
        let html = if input.minify.as_ref().is_some_and(|m| m.minify_html) {
            match fhtml::to_string(&doc) {
                Ok(s) => s,
                Err(e) => {
                    return HtmlProcessResult::Error {
                        message: format!("HTML serialize error: {}", e),
                    };
                }
            }
        } else {
            match fhtml::to_string_pretty(&doc) {
                Ok(s) => s,
                Err(e) => {
                    return HtmlProcessResult::Error {
                        message: format!("HTML serialize error: {}", e),
                    };
                }
            }
        };

        HtmlProcessResult::Success {
            html,
            had_dead_links,
            had_code_buttons,
        }
    }

    async fn diff(&self, old_html: String, new_html: String) -> HtmlDiffResult {
        tracing::debug!(
            old_len = old_html.len(),
            new_len = new_html.len(),
            "diffing HTML"
        );

        match diff::diff_html(&old_html, &new_html) {
            Ok(patches) => {
                tracing::debug!(count = patches.len(), "generated patches");
                for (i, patch) in patches.iter().enumerate() {
                    tracing::debug!(index = i, ?patch, "patch");
                }

                let nodes_compared = patches.len();
                HtmlDiffResult::Success {
                    result: DiffResult {
                        patches,
                        nodes_compared,
                        nodes_skipped: 0,
                    },
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "diff failed");
                HtmlDiffResult::Error { message: e }
            }
        }
    }

    // === Legacy methods ===

    async fn rewrite_urls(&self, html: String, path_map: HashMap<String, String>) -> HtmlResult {
        let mut doc: Html = match fhtml::from_str(&html) {
            Ok(doc) => doc,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    html_len = html.len(),
                    "facet-html parse failed in rewrite_urls"
                );
                return HtmlResult::Error {
                    message: format!("HTML parse error: {}", e),
                };
            }
        };

        rewrite_urls_in_doc(&mut doc, &path_map);

        match fhtml::to_string_pretty(&doc) {
            Ok(html) => HtmlResult::Success { html },
            Err(e) => HtmlResult::Error {
                message: format!("HTML serialize error: {}", e),
            },
        }
    }

    async fn mark_dead_links(&self, html: String, known_routes: HashSet<String>) -> HtmlResult {
        let mut doc: Html = match fhtml::from_str(&html) {
            Ok(doc) => doc,
            Err(e) => {
                return HtmlResult::Error {
                    message: format!("HTML parse error: {}", e),
                };
            }
        };

        let had_dead = mark_dead_links_in_doc(&mut doc, &known_routes);

        match fhtml::to_string_pretty(&doc) {
            Ok(html) => HtmlResult::SuccessWithFlag {
                html,
                flag: had_dead,
            },
            Err(e) => HtmlResult::Error {
                message: format!("HTML serialize error: {}", e),
            },
        }
    }

    async fn inject_code_buttons(
        &self,
        html: String,
        code_metadata: HashMap<String, CodeExecutionMetadata>,
    ) -> HtmlResult {
        let mut doc: Html = match fhtml::from_str(&html) {
            Ok(doc) => doc,
            Err(e) => {
                return HtmlResult::Error {
                    message: format!("HTML parse error: {}", e),
                };
            }
        };

        let had_buttons = inject_code_buttons_in_doc(&mut doc, &code_metadata);

        match fhtml::to_string_pretty(&doc) {
            Ok(html) => HtmlResult::SuccessWithFlag {
                html,
                flag: had_buttons,
            },
            Err(e) => HtmlResult::Error {
                message: format!("HTML serialize error: {}", e),
            },
        }
    }
}

// ============================================================================
// Inline CSS/JS minification
// ============================================================================

/// Minify inline `<style>` content via host callback
async fn minify_inline_css(host: &HtmlHostClient<ShmTransport>, doc: &mut Html) -> Result<()> {
    // Process <style> elements in <head>
    if let Some(head) = &mut doc.head {
        for style in &mut head.style {
            if !style.text.trim().is_empty() {
                match host.minify_css(style.text.clone()).await {
                    Ok(cell_html_proto::MinifyCssResult::Success { css }) => {
                        style.text = css;
                    }
                    Ok(cell_html_proto::MinifyCssResult::Error { message }) => {
                        tracing::warn!("CSS minification error: {}", message);
                    }
                    Err(e) => {
                        tracing::warn!("CSS minification RPC error: {}", e);
                    }
                }
            }
        }
    }

    // TODO: Process inline style elements in body if needed
    Ok(())
}

/// Minify inline `<script>` content via host callback
async fn minify_inline_js(host: &HtmlHostClient<ShmTransport>, doc: &mut Html) -> Result<()> {
    // Process <script> elements in <head> (only inline scripts, not external)
    if let Some(head) = &mut doc.head {
        for script in &mut head.script {
            // Only minify if it's inline (no src attribute)
            if script.src.is_none() && !script.text.trim().is_empty() {
                match host.minify_js(script.text.clone()).await {
                    Ok(cell_html_proto::MinifyJsResult::Success { js }) => {
                        script.text = js;
                    }
                    Ok(cell_html_proto::MinifyJsResult::Error { message }) => {
                        tracing::warn!("JS minification error: {}", message);
                    }
                    Err(e) => {
                        tracing::warn!("JS minification RPC error: {}", e);
                    }
                }
            }
        }
    }

    // TODO: Process script elements in body if needed
    Ok(())
}

// ============================================================================
// URL Rewriting
// ============================================================================

fn rewrite_urls_in_doc(doc: &mut Html, path_map: &HashMap<String, String>) {
    // Rewrite URLs in <head>
    if let Some(head) = &mut doc.head {
        // Link elements
        for link in &mut head.link {
            if let Some(href) = &link.href
                && let Some(new_url) = path_map.get(href)
            {
                link.href = Some(new_url.clone());
            }
        }

        // Script elements
        for script in &mut head.script {
            if let Some(src) = &script.src
                && let Some(new_url) = path_map.get(src)
            {
                script.src = Some(new_url.clone());
            }
        }
    }

    // Rewrite URLs in <body>
    if let Some(body) = &mut doc.body {
        rewrite_urls_in_flow_content(&mut body.children, path_map);
    }
}

fn rewrite_urls_in_flow_content(children: &mut [FlowContent], path_map: &HashMap<String, String>) {
    for child in children {
        match child {
            FlowContent::A(a) => {
                if let Some(href) = &a.href
                    && let Some(new_url) = path_map.get(href)
                {
                    a.href = Some(new_url.clone());
                }
                rewrite_urls_in_phrasing_content(&mut a.children, path_map);
            }
            FlowContent::Img(img) => {
                if let Some(src) = &img.src
                    && let Some(new_url) = path_map.get(src)
                {
                    img.src = Some(new_url.clone());
                }
                // TODO: Handle srcset
            }
            FlowContent::Div(div) => {
                rewrite_urls_in_flow_content(&mut div.children, path_map);
            }
            FlowContent::P(p) => {
                rewrite_urls_in_phrasing_content(&mut p.children, path_map);
            }
            FlowContent::Section(section) => {
                rewrite_urls_in_flow_content(&mut section.children, path_map);
            }
            FlowContent::Article(article) => {
                rewrite_urls_in_flow_content(&mut article.children, path_map);
            }
            FlowContent::Nav(nav) => {
                rewrite_urls_in_flow_content(&mut nav.children, path_map);
            }
            FlowContent::Header(header) => {
                rewrite_urls_in_flow_content(&mut header.children, path_map);
            }
            FlowContent::Footer(footer) => {
                rewrite_urls_in_flow_content(&mut footer.children, path_map);
            }
            FlowContent::Main(main) => {
                rewrite_urls_in_flow_content(&mut main.children, path_map);
            }
            FlowContent::Aside(aside) => {
                rewrite_urls_in_flow_content(&mut aside.children, path_map);
            }
            FlowContent::H1(h) => rewrite_urls_in_phrasing_content(&mut h.children, path_map),
            FlowContent::H2(h) => rewrite_urls_in_phrasing_content(&mut h.children, path_map),
            FlowContent::H3(h) => rewrite_urls_in_phrasing_content(&mut h.children, path_map),
            FlowContent::H4(h) => rewrite_urls_in_phrasing_content(&mut h.children, path_map),
            FlowContent::H5(h) => rewrite_urls_in_phrasing_content(&mut h.children, path_map),
            FlowContent::H6(h) => rewrite_urls_in_phrasing_content(&mut h.children, path_map),
            FlowContent::Ul(ul) => {
                for li in &mut ul.li {
                    rewrite_urls_in_flow_content(&mut li.children, path_map);
                }
            }
            FlowContent::Ol(ol) => {
                for li in &mut ol.li {
                    rewrite_urls_in_flow_content(&mut li.children, path_map);
                }
            }
            FlowContent::Blockquote(bq) => {
                rewrite_urls_in_flow_content(&mut bq.children, path_map);
            }
            FlowContent::Pre(pre) => {
                rewrite_urls_in_phrasing_content(&mut pre.children, path_map);
            }
            FlowContent::Figure(fig) => {
                rewrite_urls_in_flow_content(&mut fig.children, path_map);
            }
            FlowContent::Table(table) => {
                // TODO: Handle table structure
                let _ = table;
            }
            FlowContent::Form(form) => {
                rewrite_urls_in_flow_content(&mut form.children, path_map);
            }
            FlowContent::Details(details) => {
                rewrite_urls_in_flow_content(&mut details.children, path_map);
            }
            FlowContent::Iframe(iframe) => {
                if let Some(src) = &iframe.src
                    && let Some(new_url) = path_map.get(src)
                {
                    iframe.src = Some(new_url.clone());
                }
            }
            FlowContent::Video(video) => {
                if let Some(src) = &video.src
                    && let Some(new_url) = path_map.get(src)
                {
                    video.src = Some(new_url.clone());
                }
            }
            FlowContent::Audio(audio) => {
                if let Some(src) = &audio.src
                    && let Some(new_url) = path_map.get(src)
                {
                    audio.src = Some(new_url.clone());
                }
            }
            FlowContent::Picture(picture) => {
                for source in &mut picture.source {
                    if let Some(srcset) = &source.srcset {
                        source.srcset = Some(rewrite_srcset(srcset, path_map));
                    }
                }
                if let Some(img) = &mut picture.img
                    && let Some(src) = &img.src
                    && let Some(new_url) = path_map.get(src)
                {
                    img.src = Some(new_url.clone());
                }
            }
            _ => {}
        }
    }
}

fn rewrite_urls_in_phrasing_content(
    children: &mut [PhrasingContent],
    path_map: &HashMap<String, String>,
) {
    for child in children {
        match child {
            PhrasingContent::A(a) => {
                if let Some(href) = &a.href
                    && let Some(new_url) = path_map.get(href)
                {
                    a.href = Some(new_url.clone());
                }
                rewrite_urls_in_phrasing_content(&mut a.children, path_map);
            }
            PhrasingContent::Img(img) => {
                if let Some(src) = &img.src
                    && let Some(new_url) = path_map.get(src)
                {
                    img.src = Some(new_url.clone());
                }
            }
            PhrasingContent::Span(span) => {
                rewrite_urls_in_phrasing_content(&mut span.children, path_map);
            }
            PhrasingContent::Strong(strong) => {
                rewrite_urls_in_phrasing_content(&mut strong.children, path_map);
            }
            PhrasingContent::Em(em) => {
                rewrite_urls_in_phrasing_content(&mut em.children, path_map);
            }
            PhrasingContent::Code(code) => {
                rewrite_urls_in_phrasing_content(&mut code.children, path_map);
            }
            _ => {}
        }
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

fn mark_dead_links_in_doc(doc: &mut Html, known_routes: &HashSet<String>) -> bool {
    let mut had_dead = false;

    if let Some(body) = &mut doc.body {
        had_dead = mark_dead_links_in_flow_content(&mut body.children, known_routes);
    }

    had_dead
}

fn mark_dead_links_in_flow_content(
    children: &mut [FlowContent],
    known_routes: &HashSet<String>,
) -> bool {
    let mut had_dead = false;

    for child in children {
        match child {
            FlowContent::A(a) => {
                if let Some(href) = &a.href
                    && is_dead_link(href, known_routes)
                {
                    // Add data-dead attribute
                    // Note: GlobalAttrs doesn't have data-* support directly,
                    // we'd need to extend it or use a different approach
                    // For now, we'll track that we found dead links
                    had_dead = true;
                    // TODO: Add data-dead attribute when facet-html supports it
                }
                if mark_dead_links_in_phrasing_content(&mut a.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Div(div) => {
                if mark_dead_links_in_flow_content(&mut div.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::P(p) => {
                if mark_dead_links_in_phrasing_content(&mut p.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Section(section) => {
                if mark_dead_links_in_flow_content(&mut section.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Article(article) => {
                if mark_dead_links_in_flow_content(&mut article.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Nav(nav) => {
                if mark_dead_links_in_flow_content(&mut nav.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Header(header) => {
                if mark_dead_links_in_flow_content(&mut header.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Footer(footer) => {
                if mark_dead_links_in_flow_content(&mut footer.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Main(main) => {
                if mark_dead_links_in_flow_content(&mut main.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Aside(aside) => {
                if mark_dead_links_in_flow_content(&mut aside.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::H1(h) => {
                if mark_dead_links_in_phrasing_content(&mut h.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::H2(h) => {
                if mark_dead_links_in_phrasing_content(&mut h.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::H3(h) => {
                if mark_dead_links_in_phrasing_content(&mut h.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::H4(h) => {
                if mark_dead_links_in_phrasing_content(&mut h.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::H5(h) => {
                if mark_dead_links_in_phrasing_content(&mut h.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::H6(h) => {
                if mark_dead_links_in_phrasing_content(&mut h.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Ul(ul) => {
                for li in &mut ul.li {
                    if mark_dead_links_in_flow_content(&mut li.children, known_routes) {
                        had_dead = true;
                    }
                }
            }
            FlowContent::Ol(ol) => {
                for li in &mut ol.li {
                    if mark_dead_links_in_flow_content(&mut li.children, known_routes) {
                        had_dead = true;
                    }
                }
            }
            FlowContent::Blockquote(bq) => {
                if mark_dead_links_in_flow_content(&mut bq.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Pre(pre) => {
                if mark_dead_links_in_phrasing_content(&mut pre.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Figure(fig) => {
                if mark_dead_links_in_flow_content(&mut fig.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Form(form) => {
                if mark_dead_links_in_flow_content(&mut form.children, known_routes) {
                    had_dead = true;
                }
            }
            FlowContent::Details(details) => {
                if mark_dead_links_in_flow_content(&mut details.children, known_routes) {
                    had_dead = true;
                }
            }
            _ => {}
        }
    }

    had_dead
}

fn mark_dead_links_in_phrasing_content(
    children: &mut [PhrasingContent],
    known_routes: &HashSet<String>,
) -> bool {
    let mut had_dead = false;

    for child in children {
        match child {
            PhrasingContent::A(a) => {
                if let Some(href) = &a.href
                    && is_dead_link(href, known_routes)
                {
                    had_dead = true;
                    // TODO: Add data-dead attribute
                }
                if mark_dead_links_in_phrasing_content(&mut a.children, known_routes) {
                    had_dead = true;
                }
            }
            PhrasingContent::Span(span) => {
                if mark_dead_links_in_phrasing_content(&mut span.children, known_routes) {
                    had_dead = true;
                }
            }
            PhrasingContent::Strong(strong) => {
                if mark_dead_links_in_phrasing_content(&mut strong.children, known_routes) {
                    had_dead = true;
                }
            }
            PhrasingContent::Em(em) => {
                if mark_dead_links_in_phrasing_content(&mut em.children, known_routes) {
                    had_dead = true;
                }
            }
            PhrasingContent::Code(code) => {
                if mark_dead_links_in_phrasing_content(&mut code.children, known_routes) {
                    had_dead = true;
                }
            }
            _ => {}
        }
    }

    had_dead
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
    doc: &mut Html,
    code_metadata: &HashMap<String, CodeExecutionMetadata>,
) -> bool {
    let mut had_buttons = false;

    if let Some(body) = &mut doc.body {
        had_buttons = inject_code_buttons_in_flow_content(&mut body.children, code_metadata);
    }

    had_buttons
}

fn inject_code_buttons_in_flow_content(
    children: &mut [FlowContent],
    code_metadata: &HashMap<String, CodeExecutionMetadata>,
) -> bool {
    let mut had_buttons = false;

    for child in children.iter_mut() {
        match child {
            FlowContent::Pre(pre) => {
                // Extract text content from the pre element
                let code_text = extract_text_from_phrasing(&pre.children);
                let normalized = normalize_code_for_matching(&code_text);

                // Add position:relative to pre element
                let existing_style = pre.attrs.style.clone().unwrap_or_default();
                if !existing_style.contains("position") {
                    pre.attrs.style = Some(format!("position:relative;{}", existing_style));
                }

                // Create copy button
                let copy_button = create_copy_button();

                // Check if we have build info for this code
                let build_info_button =
                    code_metadata.get(&normalized).map(create_build_info_button);

                // Add buttons as children at the end
                // Note: This adds them as phrasing content inside pre
                if let Some(btn) = build_info_button {
                    pre.children.push(btn);
                }
                pre.children.push(copy_button);

                had_buttons = true;
            }
            FlowContent::Div(div) => {
                if inject_code_buttons_in_flow_content(&mut div.children, code_metadata) {
                    had_buttons = true;
                }
            }
            FlowContent::Section(section) => {
                if inject_code_buttons_in_flow_content(&mut section.children, code_metadata) {
                    had_buttons = true;
                }
            }
            FlowContent::Article(article) => {
                if inject_code_buttons_in_flow_content(&mut article.children, code_metadata) {
                    had_buttons = true;
                }
            }
            FlowContent::Main(main) => {
                if inject_code_buttons_in_flow_content(&mut main.children, code_metadata) {
                    had_buttons = true;
                }
            }
            FlowContent::Aside(aside) => {
                if inject_code_buttons_in_flow_content(&mut aside.children, code_metadata) {
                    had_buttons = true;
                }
            }
            FlowContent::Header(header) => {
                if inject_code_buttons_in_flow_content(&mut header.children, code_metadata) {
                    had_buttons = true;
                }
            }
            FlowContent::Footer(footer) => {
                if inject_code_buttons_in_flow_content(&mut footer.children, code_metadata) {
                    had_buttons = true;
                }
            }
            FlowContent::Nav(nav) => {
                if inject_code_buttons_in_flow_content(&mut nav.children, code_metadata) {
                    had_buttons = true;
                }
            }
            FlowContent::Blockquote(bq) => {
                if inject_code_buttons_in_flow_content(&mut bq.children, code_metadata) {
                    had_buttons = true;
                }
            }
            FlowContent::Figure(fig) => {
                if inject_code_buttons_in_flow_content(&mut fig.children, code_metadata) {
                    had_buttons = true;
                }
            }
            FlowContent::Details(details) => {
                if inject_code_buttons_in_flow_content(&mut details.children, code_metadata) {
                    had_buttons = true;
                }
            }
            _ => {}
        }
    }

    had_buttons
}

fn extract_text_from_phrasing(children: &[PhrasingContent]) -> String {
    let mut text = String::new();
    for child in children {
        match child {
            PhrasingContent::Text(t) => text.push_str(t),
            PhrasingContent::Code(code) => {
                text.push_str(&extract_text_from_phrasing(&code.children));
            }
            PhrasingContent::Span(span) => {
                text.push_str(&extract_text_from_phrasing(&span.children));
            }
            PhrasingContent::Strong(strong) => {
                text.push_str(&extract_text_from_phrasing(&strong.children));
            }
            PhrasingContent::Em(em) => {
                text.push_str(&extract_text_from_phrasing(&em.children));
            }
            _ => {}
        }
    }
    text
}

fn normalize_code_for_matching(code: &str) -> String {
    code.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn create_copy_button() -> PhrasingContent {
    PhrasingContent::Button(Button {
        attrs: GlobalAttrs {
            class: Some("copy-btn".to_string()),
            ..Default::default()
        },
        children: vec![PhrasingContent::Text("Copy".to_string())],
        ..Default::default()
    })
}

fn create_build_info_button(meta: &CodeExecutionMetadata) -> PhrasingContent {
    // TODO: facet-html doesn't support onclick or data-* attributes yet.
    // The build info popup functionality needs one of:
    // 1. facet-format-html to support arbitrary attributes
    // 2. Post-processing to add onclick handler
    // For now, we create the button without the onclick.
    let _json = metadata_to_json(meta);
    let rustc_short = meta
        .rustc_version
        .lines()
        .next()
        .unwrap_or(&meta.rustc_version);

    PhrasingContent::Button(Button {
        attrs: GlobalAttrs {
            class: Some("build-info-btn verified".to_string()),
            tooltip: Some(format!(
                "Verified: {}",
                html_escape::encode_text(rustc_short)
            )),
            ..Default::default()
        },
        children: vec![PhrasingContent::Text("\u{2139}".to_string())], // Unicode info symbol
        ..Default::default()
    })
}

fn metadata_to_json(meta: &CodeExecutionMetadata) -> String {
    // Use facet-json for proper JSON serialization
    // For now, manual construction to avoid adding another dependency
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
fn apply_injection(doc: &mut Html, injection: &Injection) {
    match injection {
        Injection::HeadStyle { css } => {
            let head = doc.head.get_or_insert_with(Default::default);
            head.style.push(Style {
                text: css.clone(),
                ..Default::default()
            });
        }
        Injection::HeadScript { js, module } => {
            let head = doc.head.get_or_insert_with(Default::default);
            head.script.push(Script {
                text: js.clone(),
                type_: if *module {
                    Some("module".to_string())
                } else {
                    None
                },
                ..Default::default()
            });
        }
        Injection::BodyScript { js, module } => {
            let body = doc.body.get_or_insert_with(Default::default);
            // Add script as a flow content element at the end of body
            body.children.push(FlowContent::Script(Script {
                text: js.clone(),
                type_: if *module {
                    Some("module".to_string())
                } else {
                    None
                },
                ..Default::default()
            }));
        }
    }
}

// ============================================================================
// Cell Setup
// ============================================================================

rapace_cell::cell_service!(HtmlProcessorServer<HtmlProcessorImpl>, HtmlProcessorImpl);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run_with_session(|session| {
        let processor = HtmlProcessorImpl::new(session);
        CellService::from(processor)
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_attributes_preserved() {
        let html = r#"<!DOCTYPE html><html><head></head><body><div class="test" data-icon="book" data-custom="42">Hello</div></body></html>"#;

        // Parse with facet-html
        let doc: Html = fhtml::from_str(html).expect("parse");

        // Check that data-* attributes are captured
        if let Some(body) = &doc.body {
            for child in &body.children {
                if let FlowContent::Div(div) = child {
                    eprintln!("div.attrs.class = {:?}", div.attrs.class);
                    eprintln!("div.attrs.extra = {:?}", div.attrs.extra);
                    assert!(
                        div.attrs.extra.contains_key("data-icon"),
                        "data-icon should be in extra"
                    );
                    assert_eq!(div.attrs.extra.get("data-icon").unwrap(), "book");
                    assert!(
                        div.attrs.extra.contains_key("data-custom"),
                        "data-custom should be in extra"
                    );
                    assert_eq!(div.attrs.extra.get("data-custom").unwrap(), "42");
                }
            }
        }

        // Serialize back
        let output = fhtml::to_string_pretty(&doc).expect("serialize");
        eprintln!("Output HTML:\n{}", output);

        // Check that data-* attributes are preserved
        assert!(
            output.contains("data-icon"),
            "data-icon should be in output"
        );
        assert!(output.contains("book"), "book value should be in output");
        assert!(
            output.contains("data-custom"),
            "data-custom should be in output"
        );
        assert!(output.contains("42"), "42 value should be in output");
    }
}
