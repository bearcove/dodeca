//! ContentService implementation for the roam RPC server
//!
//! This implements the ContentService trait from cell-http-proto,
//! allowing the HTTP cell to fetch content from the host's picante DB via RPC.

use std::sync::Arc;

use cell_http_proto::{ContentService, Identity, ServeContent};
use dodeca_protocol::{EvalResult, ScopeEntry};

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
            let auth_enabled = crate::config::global_config()
                .and_then(|c| c.auth.as_ref())
                .is_some();
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

        // In-browser editor shell. Fail closed: mint a token only for a verified
        // editor; anyone else is treated as if the page doesn't exist (we don't
        // reveal that it's editable). Unauthenticated requests behind the proxy
        // bounce to login first.
        if let Some(rest) = path.strip_prefix("/_dodeca/edit/") {
            match self.server.mint_edit_token(identity.as_ref()) {
                Some(token) => {
                    let route = format!("/{}", rest.trim_start_matches('/'));
                    let html = crate::edit_shell::render_edit_shell(
                        &route,
                        &token,
                        &crate::serve::editor_version(),
                    );
                    return ServeContent::StaticNoCache {
                        content: html.into_bytes(),
                        mime: "text/html; charset=utf-8".to_string(),
                        generation,
                    };
                }
                None => {
                    let auth_enabled = crate::config::global_config()
                        .and_then(|c| c.auth.as_ref())
                        .is_some();
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

        // Try finding content through the main find_content path
        self.server.find_content_for_rpc(&path).await
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
