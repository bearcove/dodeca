//! Dodeca HTML processing plugin (dodeca-mod-html)
//!
//! This plugin handles HTML transformations:
//! - URL rewriting (href, src, srcset attributes)
//! - Dead link marking
//! - Build info button injection

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use color_eyre::Result;
use html5ever::serialize::{SerializeOpts, serialize};
use html5ever::tendril::TendrilSink;
use html5ever::{parse_document, LocalName, QualName, local_name, ns};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};

use cell_html_proto::{
    CodeExecutionMetadata, HtmlProcessor, HtmlProcessorServer, HtmlResult,
};

/// HTML processor implementation
pub struct HtmlProcessorImpl;

impl HtmlProcessor for HtmlProcessorImpl {
    async fn rewrite_urls(&self, html: String, path_map: HashMap<String, String>) -> HtmlResult {
        match rewrite_urls_impl(&html, &path_map) {
            Ok(result) => HtmlResult::Success { html: result },
            Err(e) => HtmlResult::Error {
                message: format!("URL rewriting failed: {}", e),
            },
        }
    }

    async fn mark_dead_links(&self, html: String, known_routes: HashSet<String>) -> HtmlResult {
        match mark_dead_links_impl(&html, &known_routes) {
            Ok((result, had_dead)) => HtmlResult::SuccessWithFlag {
                html: result,
                flag: had_dead,
            },
            Err(e) => HtmlResult::Error {
                message: format!("Dead link marking failed: {}", e),
            },
        }
    }

    async fn inject_build_info(
        &self,
        html: String,
        code_metadata: HashMap<String, CodeExecutionMetadata>,
    ) -> HtmlResult {
        match inject_build_info_impl(&html, &code_metadata) {
            Ok((result, had_buttons)) => HtmlResult::SuccessWithFlag {
                html: result,
                flag: had_buttons,
            },
            Err(e) => HtmlResult::Error {
                message: format!("Build info injection failed: {}", e),
            },
        }
    }
}

// ============================================================================
// URL Rewriting Implementation
// ============================================================================

fn rewrite_urls_impl(html: &str, path_map: &HashMap<String, String>) -> Result<String> {
    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();

    walk_and_rewrite(&dom.document, path_map);

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

    Ok(String::from_utf8(output).unwrap_or_else(|_| html.to_string()))
}

fn walk_and_rewrite(handle: &Handle, path_map: &HashMap<String, String>) {
    if let NodeData::Element { attrs, .. } = &handle.data {
        let mut attrs = attrs.borrow_mut();

        // Rewrite href
        if let Some(attr) = attrs.iter_mut().find(|a| a.name.local == local_name!("href")) {
            if let Some(new_val) = path_map.get(attr.value.as_ref()) {
                attr.value = new_val.clone().into();
            }
        }

        // Rewrite src
        if let Some(attr) = attrs.iter_mut().find(|a| a.name.local == local_name!("src")) {
            if let Some(new_val) = path_map.get(attr.value.as_ref()) {
                attr.value = new_val.clone().into();
            }
        }

        // Rewrite srcset
        if let Some(attr) = attrs.iter_mut().find(|a| a.name.local == local_name!("srcset")) {
            let new_srcset = rewrite_srcset(&attr.value, path_map);
            attr.value = new_srcset.into();
        }
    }

    for child in handle.children.borrow().iter() {
        walk_and_rewrite(child, path_map);
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
// Dead Link Marking Implementation
// ============================================================================

fn mark_dead_links_impl(html: &str, known_routes: &HashSet<String>) -> Result<(String, bool)> {
    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();

    let had_dead = Rc::new(RefCell::new(false));
    walk_and_mark_dead(&dom.document, known_routes, &had_dead);
    let had_dead_result = *had_dead.borrow();

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
    Ok((result, had_dead_result))
}

fn walk_and_mark_dead(handle: &Handle, known_routes: &HashSet<String>, had_dead: &Rc<RefCell<bool>>) {
    if let NodeData::Element { name, attrs, .. } = &handle.data {
        if name.local == local_name!("a") {
            let mut attrs = attrs.borrow_mut();
            if let Some(href_attr) = attrs.iter().find(|a| a.name.local == local_name!("href")) {
                let href = href_attr.value.as_ref();

                if !href.starts_with("http://")
                    && !href.starts_with("https://")
                    && !href.starts_with('#')
                    && !href.starts_with("mailto:")
                    && !href.starts_with("tel:")
                    && !href.starts_with("javascript:")
                    && !href.starts_with("/__")
                    && href.starts_with('/')
                {
                    let static_extensions = [
                        ".css", ".js", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".woff",
                        ".woff2", ".ttf", ".eot", ".pdf", ".zip", ".tar", ".gz", ".webp", ".jxl",
                        ".xml", ".txt", ".md", ".wasm",
                    ];

                    if !static_extensions.iter().any(|ext| href.ends_with(ext)) {
                        let path = href.split('#').next().unwrap_or(href);
                        if !path.is_empty() {
                            let target = normalize_route(path);

                            let exists = known_routes.contains(&target)
                                || known_routes.contains(&format!("{}/", target.trim_end_matches('/')))
                                || known_routes.contains(target.trim_end_matches('/'));

                            if !exists {
                                attrs.push(html5ever::Attribute {
                                    name: QualName::new(None, ns!(), LocalName::from("data-dead")),
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

    for child in handle.children.borrow().iter() {
        walk_and_mark_dead(child, known_routes, had_dead);
    }
}

fn normalize_route(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => { parts.pop(); }
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
// Build Info Injection Implementation (using lol_html)
// ============================================================================

fn inject_build_info_impl(
    html: &str,
    code_metadata: &HashMap<String, CodeExecutionMetadata>,
) -> Result<(String, bool)> {
    use lol_html::{RewriteStrSettings, element, text, rewrite_str};
    use lol_html::html_content::ContentType;

    if code_metadata.is_empty() {
        return Ok((html.to_string(), false));
    }

    let current_code_text = Rc::new(RefCell::new(String::new()));
    let had_buttons = Rc::new(RefCell::new(false));
    let metadata_map = Rc::new(code_metadata.clone());

    let current_code_text_ref = current_code_text.clone();
    let had_buttons_ref = had_buttons.clone();

    let result = rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                element!("pre", |el| {
                    current_code_text_ref.borrow_mut().clear();

                    let code_text_ref = current_code_text_ref.clone();
                    let had_buttons_inner = had_buttons_ref.clone();
                    let metadata_map_ref = metadata_map.clone();

                    if let Some(handlers) = el.end_tag_handlers() {
                        let handler: lol_html::EndTagHandler<'static> = Box::new(move |end_tag| {
                            let code_text = code_text_ref.borrow();
                            let normalized = normalize_code_for_matching(&code_text);

                            if let Some(meta) = metadata_map_ref.get(&normalized) {
                                *had_buttons_inner.borrow_mut() = true;
                                let json = metadata_to_json(meta);
                                let rustc_short = meta.rustc_version
                                    .lines()
                                    .next()
                                    .unwrap_or(&meta.rustc_version);

                                let button_html = format!(
                                    r#"<button class="build-info-btn verified" aria-label="View build info" title="Verified: {}" onclick="showBuildInfoPopup({})">&#9432;</button>"#,
                                    escape_html_attr(rustc_short),
                                    escape_html_attr(&json)
                                );
                                end_tag.before(&button_html, ContentType::Html);
                            }
                            Ok(())
                        });
                        handlers.push(handler);
                    }
                    Ok(())
                }),
                text!("pre code", |t| {
                    current_code_text.borrow_mut().push_str(t.as_str());
                    Ok(())
                }),
            ],
            ..Default::default()
        },
    );

    let had_buttons_result = *had_buttons.borrow();

    match result {
        Ok(rewritten) => Ok((rewritten, had_buttons_result)),
        Err(e) => Err(color_eyre::eyre::eyre!("lol_html error: {:?}", e)),
    }
}

fn normalize_code_for_matching(code: &str) -> String {
    code.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn metadata_to_json(meta: &CodeExecutionMetadata) -> String {
    let deps_json: Vec<String> = meta.dependencies.iter().map(|d| {
        let source = match &d.source {
            cell_html_proto::DependencySource::CratesIo => "crates.io".to_string(),
            cell_html_proto::DependencySource::Git { url, commit } => format!("git:{}@{}", url, &commit[..7.min(commit.len())]),
            cell_html_proto::DependencySource::Path { path } => format!("path:{}", path),
        };
        format!(r#"{{"name":"{}","version":"{}","source":"{}"}}"#,
            escape_json(&d.name),
            escape_json(&d.version),
            escape_json(&source)
        )
    }).collect();

    format!(
        r#"{{"rustc_version":"{}","cargo_version":"{}","target":"{}","timestamp":"{}","cache_hit":{},"platform":"{}","arch":"{}","dependencies":[{}]}}"#,
        escape_json(&meta.rustc_version),
        escape_json(&meta.cargo_version),
        escape_json(&meta.target),
        escape_json(&meta.timestamp),
        meta.cache_hit,
        escape_json(&meta.platform),
        escape_json(&meta.arch),
        deps_json.join(",")
    )
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn escape_html_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ============================================================================
// Plugin Setup
// ============================================================================

dodeca_cell_runtime::cell_service!(
    HtmlProcessorServer<HtmlProcessorImpl>,
    HtmlProcessorImpl
);

dodeca_cell_runtime::run_cell!(HtmlProcessorImpl);
