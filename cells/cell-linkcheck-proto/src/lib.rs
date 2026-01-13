//! RPC protocol for dodeca linkcheck cell
//!
//! Defines services for external link checking.

use facet::Facet;
use std::collections::HashMap;

/// Status of an external link check
#[derive(Debug, Clone, Facet, PartialEq, Eq)]
pub struct LinkStatus {
    /// "ok", "error", "failed", or "skipped"
    pub status: String,
    /// HTTP status code (for "error" status)
    pub code: Option<u16>,
    /// Error message (for "failed" status)
    pub message: Option<String>,
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
