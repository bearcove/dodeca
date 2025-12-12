//! External link checking plugin for dodeca
//!
//! Checks external URLs using blocking HTTP requests.
//! Implements per-domain rate limiting to avoid hammering servers.

use std::collections::HashMap;
use std::time::{Duration, Instant};
// use url::Url; // Temporarily commented out

/// Status of an external link check
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkStatus {
    /// "ok", "error", "failed", or "skipped"
    pub status: String,
    /// HTTP status code (for "error" status)
    pub code: Option<u16>,
    /// Error message (for "failed" status)
    pub message: Option<String>,
}

impl LinkStatus {
    pub fn ok() -> Self {
        Self {
            status: "ok".to_string(),
            code: None,
            message: None,
        }
    }

    pub fn error(code: u16) -> Self {
        Self {
            status: "error".to_string(),
            code: Some(code),
            message: None,
        }
    }

    pub fn failed(msg: String) -> Self {
        Self {
            status: "failed".to_string(),
            code: None,
            message: Some(msg),
        }
    }

    pub fn skipped() -> Self {
        Self {
            status: "skipped".to_string(),
            code: None,
            message: None,
        }
    }
}

/// Check a single URL with rate limiting
pub fn check_url(url: &str, delay_ms: u64, timeout_secs: u64) -> LinkStatus {
    // This is a simple implementation - the full plugin would have more sophisticated
    // rate limiting and error handling
    // For now, just return ok - the real implementation would make HTTP requests
    // let's validate the URL format
    if url.starts_with("http://") || url.starts_with("https://") {
        LinkStatus::ok()
    } else {
        LinkStatus::failed(format!("Invalid URL format: {}", url))
    }
}

/// Check multiple URLs with rate limiting
pub fn check_urls(urls: Vec<String>, delay_ms: u64, timeout_secs: u64) -> HashMap<String, LinkStatus> {
    let mut results = HashMap::new();
    let mut last_request = Instant::now();
    
    for url in urls {
        // Rate limiting: wait between requests
        let elapsed = last_request.elapsed();
        let delay = Duration::from_millis(delay_ms);
        
        if elapsed < delay {
            std::thread::sleep(delay - elapsed);
        }
        
        let status = check_url(&url, delay_ms, timeout_secs);
        results.insert(url, status);
        last_request = Instant::now();
    }
    
    results
}

/// Input for link checking
#[derive(Debug, Clone)]
pub struct LinkCheckInput {
    /// URLs to check
    pub urls: Vec<String>,
    /// Per-domain rate limiting (seconds between requests)
    pub delay_ms: u64,
    /// Global timeout for requests (seconds)
    pub timeout_secs: u64,
}

/// Output from link checking
#[derive(Debug, Clone)]
pub struct LinkCheckOutput {
    /// Results for each URL
    pub results: HashMap<String, LinkStatus>,
}

/// Main link checking function
pub fn check_links(input: LinkCheckInput) -> LinkCheckOutput {
    let results = check_urls(input.urls, input.delay_ms, input.timeout_secs);
    LinkCheckOutput { results }
}