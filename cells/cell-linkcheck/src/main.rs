//! Dodeca linkcheck cell (cell-linkcheck)
//!
//! This cell handles external link checking with per-domain rate limiting.

use std::collections::HashMap;
use std::time::Duration;

use dodeca_cell_runtime::run_cell;
use url::Url;

use cell_linkcheck_proto::{
    LinkCheckInput, LinkCheckOutput, LinkCheckResult, LinkChecker, LinkCheckerDispatcher,
    LinkDiagnostics, LinkStatus,
};

/// Generate a realistic browser User-Agent string.
///
/// We're not happy about this, but many websites return 404 or 403 for perfectly
/// valid pages when they see an honest bot-like User-Agent. Using a browser-like
/// UA is the only way to get accurate link checking results.
fn generate_user_agent() -> String {
    // Chrome 131 on macOS
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".to_string()
}

/// LinkChecker implementation
#[derive(Clone)]
pub struct LinkCheckerImpl {
    /// HTTP client for making requests
    client: reqwest::Client,
}

impl LinkCheckerImpl {
    fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(generate_user_agent())
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("failed to create HTTP client");

        Self { client }
    }

    /// Extract domain from URL for rate limiting
    fn get_domain(url: &str) -> Option<String> {
        Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
    }

    /// Check a single URL
    async fn check_single_url(&self, url: &str, timeout_secs: u64) -> LinkStatus {
        // Validate URL format
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return LinkStatus::Failed {
                message: format!("Invalid URL format: {}", url),
            };
        }

        let timeout = Duration::from_secs(timeout_secs);

        // Build request with explicit headers
        let request = self
            .client
            .get(url)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .build();

        let request = match request {
            Ok(r) => r,
            Err(e) => {
                return LinkStatus::Failed {
                    message: format!("Failed to build request: {e}"),
                };
            }
        };

        // Capture request headers for diagnostics
        let request_headers: Vec<(String, String)> = request
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("?").to_string()))
            .collect();

        // Always use GET - HEAD is unreliable (many servers don't implement it correctly)
        match tokio::time::timeout(timeout, self.client.execute(request)).await {
            Ok(Ok(response)) => {
                let status_code = response.status().as_u16();

                if response.status().is_success() || response.status().is_redirection() {
                    LinkStatus::Ok
                } else {
                    // Capture response headers for error responses
                    let response_headers: Vec<(String, String)> = response
                        .headers()
                        .iter()
                        .filter(|(k, _)| {
                            let name = k.as_str().to_lowercase();
                            matches!(
                                name.as_str(),
                                "content-type"
                                    | "server"
                                    | "x-frame-options"
                                    | "location"
                                    | "cf-ray"
                                    | "x-cache"
                            )
                        })
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("?").to_string()))
                        .collect();

                    // Get response body snippet
                    let response_body = response
                        .text()
                        .await
                        .ok()
                        .map(|text| {
                            let cleaned: String = text
                                .chars()
                                .take(500)
                                .map(|c| if c.is_whitespace() { ' ' } else { c })
                                .collect();
                            cleaned.trim().to_string()
                        })
                        .unwrap_or_default();

                    LinkStatus::HttpError {
                        code: status_code,
                        diagnostics: LinkDiagnostics {
                            request_headers,
                            response_headers,
                            response_body,
                        },
                    }
                }
            }
            Ok(Err(e)) => LinkStatus::Failed {
                message: e.to_string(),
            },
            Err(_) => LinkStatus::Failed {
                message: "request timed out".to_string(),
            },
        }
    }
}

impl LinkChecker for LinkCheckerImpl {
    async fn check_links(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        input: LinkCheckInput,
    ) -> LinkCheckResult {
        let mut results: HashMap<String, LinkStatus> = HashMap::new();
        let mut last_request_per_domain: HashMap<String, tokio::time::Instant> = HashMap::new();
        let delay = Duration::from_millis(input.delay_ms);

        for url in input.urls {
            // Rate limiting per domain
            if let Some(domain) = Self::get_domain(&url) {
                if let Some(last) = last_request_per_domain.get(&domain) {
                    let elapsed = last.elapsed();
                    if elapsed < delay {
                        tokio::time::sleep(delay - elapsed).await;
                    }
                }
                last_request_per_domain.insert(domain, tokio::time::Instant::now());
            }

            let start = std::time::Instant::now();
            let status = self.check_single_url(&url, input.timeout_secs).await;
            let elapsed_ms = start.elapsed().as_millis();

            // Log each link check with timing and result
            match &status {
                LinkStatus::Ok => {
                    tracing::info!(url = %url, elapsed_ms = %elapsed_ms, "OK");
                }
                LinkStatus::HttpError { code, .. } => {
                    tracing::info!(url = %url, elapsed_ms = %elapsed_ms, status = %code, "HTTP error");
                }
                LinkStatus::Failed { message } => {
                    tracing::info!(url = %url, elapsed_ms = %elapsed_ms, error = %message, "failed");
                }
                LinkStatus::Skipped => {
                    tracing::info!(url = %url, elapsed_ms = %elapsed_ms, "skipped");
                }
            }

            results.insert(url, status);
        }

        LinkCheckResult::Success {
            output: LinkCheckOutput { results },
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("linkcheck", |_handle| {
        LinkCheckerDispatcher::new(LinkCheckerImpl::new())
    })
}
