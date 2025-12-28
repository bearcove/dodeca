//! ContentService implementation for the rapace RPC server
//!
//! This implements the ContentService trait from dodeca-serve-protocol,
//! allowing the cell to fetch content from the host's picante DB via RPC.

use std::sync::Arc;

use cell_http_proto::{ContentService, ServeContent};
use dodeca_protocol::{EvalResult, ScopeEntry};

use crate::serve::{SiteServer, get_devtools_asset, get_search_file_content};

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
    async fn find_content(&self, path: String) -> ServeContent {
        // Stall until the current revision is fully ready.
        self.server.wait_revision_ready().await;

        // Get current generation
        let generation = self.server.current_generation();

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

        // Check search files (pagefind)
        if let Some(content) = get_search_file_content(&self.server.search_files, &path) {
            return ServeContent::Search {
                content,
                mime: guess_mime(&path).to_string(),
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

/// Guess MIME type from file extension
fn guess_mime(path: &str) -> &'static str {
    if path.ends_with(".js") {
        "application/javascript"
    } else if path.ends_with(".wasm") {
        "application/wasm"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".html") {
        "text/html"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".webp") {
        "image/webp"
    } else if path.ends_with(".jxl") {
        "image/jxl"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else if path.ends_with(".woff") {
        "font/woff"
    } else if path.ends_with(".ttf") {
        "font/ttf"
    } else if path.ends_with(".otf") {
        "font/otf"
    } else {
        "application/octet-stream"
    }
}
