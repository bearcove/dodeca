//! RPC protocol for dodeca linkcheck cell
//!
//! Defines services for external link checking.

use facet::Facet;
use std::collections::HashMap;

/// Diagnostics for failed/error responses
#[derive(Debug, Clone, Facet, PartialEq, Eq)]
pub struct LinkDiagnostics {
    /// Request headers that were sent
    pub request_headers: Vec<(String, String)>,
    /// Response headers received (filtered to interesting ones)
    pub response_headers: Vec<(String, String)>,
    /// Response body snippet (first 500 chars)
    pub response_body: String,
}

/// Status of an external link check
#[derive(Debug, Clone, Facet, PartialEq, Eq)]
#[repr(u8)]
pub enum LinkStatus {
    /// Link is OK (2xx or 3xx response)
    Ok,
    /// HTTP error response (4xx, 5xx)
    HttpError {
        code: u16,
        diagnostics: LinkDiagnostics,
    },
    /// Request failed (network error, timeout, etc.)
    Failed { message: String },
    /// Skipped (e.g., rate limited)
    Skipped,
}

/// Input for link checking
#[derive(Debug, Clone, Facet)]
pub struct LinkCheckInput {
    /// URLs to check
    pub urls: Vec<String>,
    /// Per-domain rate limiting (seconds between requests)
    pub delay_ms: u64,
    /// Global timeout for requests (seconds)
    pub timeout_secs: u64,
}

/// Output from link checking
#[derive(Debug, Clone, Facet)]
pub struct LinkCheckOutput {
    /// Results for each URL
    pub results: HashMap<String, LinkStatus>,
}

/// Result of link checking operations
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum LinkCheckResult {
    /// Successfully checked links
    Success { output: LinkCheckOutput },
    /// Error during checking
    Error { message: String },
}

/// Link checking service implemented by the cell.
///
/// The host calls these methods to check external URLs.
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait LinkChecker {
    /// Check external URLs for validity
    async fn check_links(&self, input: LinkCheckInput) -> LinkCheckResult;
}
