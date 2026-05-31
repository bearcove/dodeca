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

        // Status page — gated: requires an authenticated identity (forwarded by
        // oauth2-proxy). Fail-closed: no identity → bounce to the proxy login.
        if path == "/_dodeca/status" {
            if identity.is_none() {
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
