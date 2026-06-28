//! ContentService implementation for the local HTTP router.
//!
//! This implements the ContentService trait from cell-http-proto,
//! allowing the HTTP router to fetch content from the picante DB.

use std::sync::Arc;

use cell_http_proto::{ContentService, Identity, ServeContent};
use dodeca_protocol::{EvalResult, ScopeEntry};

use crate::coverage::{CoverageEndpoint, CoverageOutputFormat, CoverageSelector};
use crate::serve::{SiteServer, get_devtools_asset};

/// ContentService implementation that wraps SiteServer
#[derive(Clone)]
pub struct HostContentService {
    server: Arc<SiteServer>,
}

impl HostContentService {
    pub fn new(server: Arc<SiteServer>) -> Self {
        Self { server }
    }
}

impl ContentService for HostContentService {
    async fn find_content(&self, path: String, identity: Option<Identity>) -> ServeContent {
        // Stall until the current revision is fully ready.
        self.server.wait_revision_ready().await;

        // Get current generation
        let generation = self.server.current_generation();

        // Status page. Gated only when `auth` is configured (deployed behind
        // oauth2-proxy); fail-closed there — no identity → bounce to the proxy
        // login. With no `auth` config (local `ddc serve`) it's open.
        if path == "/_dodeca/status" {
            let auth_enabled = crate::config::global_config().is_some_and(|c| c.auth.is_some());
            if auth_enabled && identity.is_none() {
                return ServeContent::Redirect {
                    location: format!("/oauth2/start?rd={path}"),
                    generation,
                };
            }
            return ServeContent::StaticNoCache {
                content: self.server.status_html().into_bytes(),
                mime: "text/html; charset=utf-8".to_string(),
                generation,
            };
        }

        // Well-known editor-token endpoint: a verified editor mints an
        // identity-scoped session token here, as JSON (facet-json), instead of
        // scraping the shell's `data-token`. Identity-scoped, so no page route is
        // needed. Same gate as the shell — fail closed for non-editors.
        if path == "/_dodeca/edit-token" {
            return match self.server.mint_edit_token(identity.as_ref()) {
                Some(token) => {
                    let body = facet_json::to_string(&dodeca_protocol::EditTokenResponse { token })
                        .unwrap_or_else(|_| "{}".to_string());
                    ServeContent::StaticNoCache {
                        content: body.into_bytes(),
                        mime: "application/json; charset=utf-8".to_string(),
                        generation,
                    }
                }
                None => ServeContent::NotFound {
                    html: "<!doctype html><title>not found</title>not found".to_string(),
                    generation,
                },
            };
        }

        // Well-known semantic search endpoint (dev): an agent curls
        // `/_dodeca/knowledge/search?q=…&k=…` and gets the most relevant pages as
        // JSON. `path` carries the query string for this prefix (see cell-http).
        if let Some(rest) = path.strip_prefix("/_dodeca/knowledge/search") {
            let params = parse_query_string(rest);
            let query = params.get("q").map(String::as_str).unwrap_or("");
            let k = params
                .get("k")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(8)
                .clamp(1, 50);
            let response = self.server.knowledge_search(query, k).await;
            let body = facet_json::to_string(&response).unwrap_or_else(|_| "{}".to_string());
            return ServeContent::StaticNoCache {
                content: body.into_bytes(),
                mime: "application/json; charset=utf-8".to_string(),
                generation,
            };
        }

        // Pages related to a given one: `/_dodeca/knowledge/related?route=/x&k=…`.
        if let Some(rest) = path.strip_prefix("/_dodeca/knowledge/related") {
            let params = parse_query_string(rest);
            let route = params.get("route").map(String::as_str).unwrap_or("");
            let k = params
                .get("k")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(8)
                .clamp(1, 50);
            let response = self.server.knowledge_related(route, k).await;
            let body = facet_json::to_string(&response).unwrap_or_else(|_| "{}".to_string());
            return ServeContent::StaticNoCache {
                content: body.into_bytes(),
                mime: "application/json; charset=utf-8".to_string(),
                generation,
            };
        }

        // Page connection graph: `?format=json` returns the neighborhood
        // (outbound / backlinks / related); otherwise an interactive SVG view.
        if let Some(rest) = path.strip_prefix("/_dodeca/knowledge/graph") {
            let params = parse_query_string(rest);
            let route = params.get("route").map(String::as_str).unwrap_or("");
            let k = params
                .get("k")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(10)
                .clamp(1, 50);
            if params.get("format").map(String::as_str) == Some("json") {
                let body = match self.server.knowledge_graph(route, k).await {
                    Some(g) => facet_json::to_string(&g).unwrap_or_else(|_| "{}".to_string()),
                    None => "{}".to_string(),
                };
                return ServeContent::StaticNoCache {
                    content: body.into_bytes(),
                    mime: "application/json; charset=utf-8".to_string(),
                    generation,
                };
            }
            return ServeContent::StaticNoCache {
                content: crate::knowledge::graph_view_html(route).into_bytes(),
                mime: "text/html; charset=utf-8".to_string(),
                generation,
            };
        }

        if path == "/_dodeca/annotations.json" {
            let response = self.server.annotation_index().await;
            let body = facet_json::to_string(&response).unwrap_or_else(|_| "{}".to_string());
            return ServeContent::StaticNoCache {
                content: body.into_bytes(),
                mime: "application/json; charset=utf-8".to_string(),
                generation,
            };
        }
        if path == "/_dodeca/annotations" || path == "/_dodeca/annotations.html" {
            let response = self.server.annotation_index().await;
            let body = crate::annotations::render_html(&response).await;
            return ServeContent::StaticNoCache {
                content: body.into_bytes(),
                mime: "text/html; charset=utf-8".to_string(),
                generation,
            };
        }
        if path == "/_dodeca/annotations.md" {
            let response = self.server.annotation_index().await;
            let body = crate::annotations::render_markdown(&response);
            return ServeContent::StaticNoCache {
                content: body.into_bytes(),
                mime: "text/markdown; charset=utf-8".to_string(),
                generation,
            };
        }

        // Coverage query API: suffix chooses representation (`.json` for typed
        // DTOs, `.md` for agent/human-readable output).
        if let Some(rest) = path.strip_prefix("/_dodeca/coverage/") {
            if let Some((endpoint, format, selector)) = parse_coverage_endpoint(rest) {
                if let Some(output) = self
                    .server
                    .coverage_output(endpoint, format, selector)
                    .await
                {
                    return ServeContent::StaticNoCache {
                        content: output.body.into_bytes(),
                        mime: output.format.mime().to_string(),
                        generation,
                    };
                }
            }
            return ServeContent::NotFound {
                html: "<!doctype html><title>not found</title>not found".to_string(),
                generation,
            };
        }

        // In-browser editor shell. Fail closed: mint a token only for a verified
        // editor; anyone else is treated as if the page doesn't exist (we don't
        // reveal that it's editable). Unauthenticated requests behind the proxy
        // bounce to login first.
        if let Some(rest) = path.strip_prefix("/_dodeca/edit/") {
            match self.server.mint_edit_token(identity.as_ref()) {
                Some(token) => {
                    let route = format!("/{}", rest.trim_start_matches('/'));
                    let html = crate::edit_shell::render_edit_shell(&route, &token);
                    return ServeContent::StaticNoCache {
                        content: html.into_bytes(),
                        mime: "text/html; charset=utf-8".to_string(),
                        generation,
                    };
                }
                None => {
                    let auth_enabled =
                        crate::config::global_config().is_some_and(|c| c.auth.is_some());
                    if auth_enabled && identity.is_none() {
                        return ServeContent::Redirect {
                            location: format!("/oauth2/start?rd={path}"),
                            generation,
                        };
                    }
                    return ServeContent::NotFound {
                        html: "<!doctype html><title>not found</title>not found".to_string(),
                        generation,
                    };
                }
            }
        }

        // Git webhook: `/_dodeca/pull` pulls every git source; `/_dodeca/pull/<name>`
        // pulls one. The push-driven (ideal) counterpart to `--git-poll`. The
        // file watcher re-renders whatever the pull brings in.
        if path == "/_dodeca/pull" || path.starts_with("/_dodeca/pull/") {
            let name = path
                .strip_prefix("/_dodeca/pull/")
                .filter(|s| !s.is_empty());
            let started = self.server.pull_git_sources(name);
            return ServeContent::StaticNoCache {
                content: format!("pulling {started} source(s)\n").into_bytes(),
                mime: "text/plain; charset=utf-8".to_string(),
                generation,
            };
        }

        // Built browser-editor bundle (vite/Monaco) at /_/edit/*. Public assets
        // (JS/CSS) — the editing capability behind them is token-gated.
        if path.starts_with("/_/edit/")
            && let Some((content, mime)) = crate::serve::get_editor_asset(&path)
        {
            // No-cache: the entry (`edit.js`/`edit.css`) is unhashed and changes
            // on rebuild, so it must not be cached immutably.
            return ServeContent::StaticNoCache {
                content,
                mime: mime.to_string(),
                generation,
            };
        }

        // Annotation-overlay bundle at /_/annotate/* (dev-only inline notes).
        if path.starts_with("/_/annotate/")
            && let Some((content, mime)) = crate::serve::get_annotate_asset(&path)
        {
            return ServeContent::StaticNoCache {
                content,
                mime: mime.to_string(),
                generation,
            };
        }

        // Check devtools assets first (/_/*.js, /_/*.wasm, /_/snippets/*)
        if path.starts_with("/_/")
            && let Some((content, mime)) = get_devtools_asset(&path)
        {
            return ServeContent::StaticNoCache {
                content,
                mime: mime.to_string(),
                generation,
            };
        }

        // Check for rule redirects (/@rule.id -> /page/#r-rule.id)
        if let Some(rule_id) = path.strip_prefix("/@") {
            if let Some(location) = self.server.find_rule_redirect(rule_id).await {
                return ServeContent::Redirect {
                    location,
                    generation,
                };
            }
        }

        // Per-viewer editor flag: true iff this identity would be granted an
        // edit token (the SAME gate as `mint_edit_token`). Threaded into the
        // render as a tracked `can_edit` argument so the template can show an
        // Edit button to verified editors only — and so picante memoizes the
        // editor render separately from the shared anonymous one.
        let can_edit = self.server.can_edit(identity.as_ref());

        // Try finding content through the main find_content path
        self.server.find_content_for_rpc(&path, can_edit).await
    }

    async fn get_scope(&self, route: String, path: Vec<String>) -> Vec<ScopeEntry> {
        self.server.get_scope_for_route(&route, &path).await
    }

    async fn eval_expression(&self, route: String, expression: String) -> EvalResult {
        match self
            .server
            .eval_expression_for_route(&route, &expression)
            .await
        {
            Ok(value) => EvalResult::Ok(value),
            Err(msg) => EvalResult::Err(msg),
        }
    }
}

/// Parse a URL query string (`?q=foo+bar&k=5` or `q=…`) into decoded key/value
/// pairs. Tiny and dependency-free; handles `+` and `%XX` escapes.
fn parse_query_string(s: &str) -> std::collections::HashMap<String, String> {
    let s = s.strip_prefix('?').unwrap_or(s);
    let mut map = std::collections::HashMap::new();
    for pair in s.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(percent_decode(k), percent_decode(v));
    }
    map
}

fn parse_coverage_endpoint(
    rest: &str,
) -> Option<(CoverageEndpoint, CoverageOutputFormat, CoverageSelector)> {
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let params = parse_query_string(query);
    let (path, format) = strip_coverage_suffix(path)?;
    let selector =
        CoverageSelector::new(params.get("source").cloned(), params.get("impl").cloned());
    let endpoint = match path {
        "status" => CoverageEndpoint::Status,
        "uncovered" => CoverageEndpoint::Uncovered,
        "untested" => CoverageEndpoint::Untested,
        "unmapped" => CoverageEndpoint::Unmapped,
        "stale" => CoverageEndpoint::Stale,
        "invalid" => CoverageEndpoint::Invalid,
        "validate" => CoverageEndpoint::Validate {
            threshold: params.get("threshold").and_then(|v| v.parse().ok()),
        },
        rule if rule.starts_with("rule/") => CoverageEndpoint::Rule {
            id: percent_decode(rule.trim_start_matches("rule/")),
        },
        _ => return None,
    };
    Some((endpoint, format, selector))
}

fn strip_coverage_suffix(path: &str) -> Option<(&str, CoverageOutputFormat)> {
    if let Some(path) = path.strip_suffix(".json") {
        Some((path, CoverageOutputFormat::Json))
    } else if let Some(path) = path.strip_suffix(".md") {
        Some((path, CoverageOutputFormat::Markdown))
    } else {
        None
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(a), Some(b)) => {
                    out.push(a * 16 + b);
                    i += 3;
                }
                _ => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
