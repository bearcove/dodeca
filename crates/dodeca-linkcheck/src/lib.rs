//! External link checking plugin for dodeca
//!
//! Checks external URLs using blocking HTTP requests.
//! Implements per-domain rate limiting to avoid hammering servers.

use facet::Facet;
use plugcard::{plugcard, PlugResult};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use url::Url;

plugcard::export_plugin!();

/// Status of an external link check
#[derive(Facet, Debug, Clone, PartialEq, Eq)]
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
        Self { status: "ok".to_string(), code: None, message: None }
    }

    pub fn error(code: u16) -> Self {
        Self { status: "error".to_string(), code: Some(code), message: None }
    }

    pub fn failed(msg: String) -> Self {
        Self { status: "failed".to_string(), code: None, message: Some(msg) }
    }

    pub fn skipped() -> Self {
        Self { status: "skipped".to_string(), code: None, message: None }
    }
}

/// Options for link checking
#[derive(Facet, Debug, Clone)]
pub struct CheckOptions {
    /// Domains to skip checking
    pub skip_domains: Vec<String>,
    /// Minimum delay between requests to the same domain (milliseconds)
    pub rate_limit_ms: u64,
    /// Request timeout in seconds
    pub timeout_secs: u64,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            skip_domains: Vec::new(),
            rate_limit_ms: 1000,
            timeout_secs: 10,
        }
    }
}

/// Result of checking multiple URLs
#[derive(Facet, Debug, Clone)]
pub struct CheckResult {
    /// Status for each URL (in same order as input)
    pub statuses: Vec<LinkStatus>,
    /// Number of URLs that were actually checked (not skipped)
    pub checked_count: u32,
}

/// Input for batch URL checking
#[derive(Facet)]
struct CheckUrlsInput {
    urls: Vec<String>,
    options: CheckOptions,
}

/// Check a batch of external URLs
///
/// Returns status for each URL in the same order as input.
/// Uses blocking HTTP requests with per-domain rate limiting.
#[plugcard]
pub fn check_urls(urls: Vec<String>, options: CheckOptions) -> PlugResult<CheckResult> {
    let skip_domains: HashSet<String> = options.skip_domains.iter().cloned().collect();
    let rate_limit = Duration::from_millis(options.rate_limit_ms);
    let timeout = Duration::from_secs(options.timeout_secs);

    // Build HTTP client
    let client = match reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent("dodeca-link-checker/1.0")
        .build()
    {
        Ok(c) => c,
        Err(e) => return PlugResult::Err(format!("Failed to build HTTP client: {e}")),
    };

    // Track last request time per domain for rate limiting
    let mut domain_last_request: HashMap<String, Instant> = HashMap::new();
    let mut statuses = Vec::with_capacity(urls.len());
    let mut checked_count = 0u32;

    for url in &urls {
        // Extract domain
        let domain = match Url::parse(url) {
            Ok(u) => u.host_str().map(|s| s.to_string()),
            Err(_) => None,
        };

        let Some(domain) = domain else {
            statuses.push(LinkStatus::skipped());
            continue;
        };

        // Check if domain should be skipped
        if skip_domains.contains(&domain) {
            statuses.push(LinkStatus::skipped());
            continue;
        }

        // Apply rate limiting
        if let Some(&last_request) = domain_last_request.get(&domain) {
            let elapsed = last_request.elapsed();
            if elapsed < rate_limit {
                let wait_time = rate_limit - elapsed;
                std::thread::sleep(wait_time);
            }
        }

        // Make HTTP HEAD request
        let status = check_single_url(&client, url);

        // Update last request time
        domain_last_request.insert(domain.clone(), Instant::now());

        // Handle Retry-After for rate-limited responses
        if status.code == Some(429) || status.code == Some(503) {
            // Add extra delay for rate-limited responses
            std::thread::sleep(Duration::from_secs(2));
        }

        checked_count += 1;
        statuses.push(status);
    }

    PlugResult::Ok(CheckResult {
        statuses,
        checked_count,
    })
}

/// Check a single URL
fn check_single_url(client: &reqwest::blocking::Client, url: &str) -> LinkStatus {
    match client.head(url).send() {
        Ok(response) => {
            let status_code = response.status().as_u16();
            if (200..400).contains(&status_code) {
                LinkStatus::ok()
            } else {
                LinkStatus::error(status_code)
            }
        }
        Err(e) => LinkStatus::failed(e.to_string()),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn debug_struct_size() -> usize {
    std::mem::size_of::<plugcard::MethodSignature>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_urls_skips_invalid() {
        let urls = vec!["not-a-url".to_string()];
        let options = CheckOptions::default();

        let result = check_urls(urls, options);
        let PlugResult::Ok(result) = result else {
            panic!("Expected Ok");
        };

        assert_eq!(result.statuses.len(), 1);
        assert_eq!(result.statuses[0], LinkStatus::skipped());
        assert_eq!(result.checked_count, 0);
    }

    #[test]
    fn test_check_urls_skips_domains() {
        let urls = vec!["https://example.com/page".to_string()];
        let options = CheckOptions {
            skip_domains: vec!["example.com".to_string()],
            ..Default::default()
        };

        let result = check_urls(urls, options);
        let PlugResult::Ok(result) = result else {
            panic!("Expected Ok");
        };

        assert_eq!(result.statuses.len(), 1);
        assert_eq!(result.statuses[0], LinkStatus::skipped());
        assert_eq!(result.checked_count, 0);
    }
}
