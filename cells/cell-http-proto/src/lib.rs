//! Shared protocol types for the dodeca dev server.
//!
//! `ContentService` is now a direct Rust trait used by local HTTP serving code.

use facet::Facet;

// Re-export types from dodeca-protocol that are used by the devtools interface.
pub use dodeca_protocol::{EvalResult, ScopeEntry, ScopeValue};

/// Content returned by the host for a given path
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum ServeContent {
    /// HTML page content
    Html {
        content: String,
        route: String,
        generation: u64,
    },
    /// CSS stylesheet
    Css { content: String, generation: u64 },
    /// Static file with MIME type (immutable, cacheable)
    Static {
        content: Vec<u8>,
        mime: String,
        generation: u64,
    },
    /// Static file that should not be cached
    StaticNoCache {
        content: Vec<u8>,
        mime: String,
        generation: u64,
    },
    /// Redirect to another URL (302 temporary redirect)
    Redirect { location: String, generation: u64 },
    /// Not found - rendered 404 HTML page
    NotFound { html: String, generation: u64 },
}

/// The authenticated requester, as forwarded by an auth proxy (oauth2-proxy in
/// front of dodeca, backed by Forgejo OIDC). The cell fills this from the
/// `X-Forwarded-*` headers; `None` means an unauthenticated request. The host
/// uses it to gate `/_dodeca/*` (status, editing) and to attribute edits.
#[derive(Debug, Clone, Facet)]
pub struct Identity {
    /// Stable user id (`X-Forwarded-User`).
    pub user: String,
    /// Email (`X-Forwarded-Email`), used as the git author email.
    pub email: String,
    /// Display/preferred name (`X-Forwarded-Preferred-Username`).
    pub name: String,
    /// Group memberships (`X-Forwarded-Groups`), for the editor allowlist.
    pub groups: Vec<String>,
}

/// Content service provided to the local HTTP router.
#[allow(async_fn_in_trait)]
pub trait ContentService {
    /// Find content for a given path (HTML, CSS, static files, devtools assets).
    /// `identity` is the forwarded auth identity, or `None` if unauthenticated.
    async fn find_content(
        &self,
        path: String,
        identity: Option<crate::Identity>,
    ) -> crate::ServeContent;

    /// Get scope entries for devtools (variable inspector)
    async fn get_scope(&self, route: String, path: Vec<String>) -> Vec<crate::ScopeEntry>;

    /// Evaluate an expression in the context of a route (REPL)
    async fn eval_expression(&self, route: String, expression: String) -> crate::EvalResult;
}
