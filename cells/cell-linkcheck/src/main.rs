//! Dodeca linkcheck plugin (dodeca-mod-linkcheck)
//!
//! This plugin handles external link checking with per-domain rate limiting.

use std::collections::HashMap;
use std::time::Duration;

use url::Url;

use cell_linkcheck_proto::{
    LinkCheckInput, LinkCheckOutput, LinkCheckResult, LinkChecker, LinkCheckerServer, LinkStatus,
};

/// LinkChecker implementation
pub struct LinkCheckerImpl {
    /// HTTP client for making requests
    client: reqwest::Client,
}

impl LinkCheckerImpl {
    fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("dodeca-linkcheck/1.0")
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("failed to create HTTP client");

        Self { client }
    }

    /// Extract domain from URL for rate limiting
    fn get_domain(url: &str) -> Option<String> {
        Url::parse(url).ok().and_then(|u| u.host_str().map(|s| s.to_string()))
    }

    /// Check a single URL
    async fn check_single_url(&self, url: &str, timeout_secs: u64) -> LinkStatus {
        // Validate URL format
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return LinkStatus {
                status: "failed".to_string(),
                code: None,
                message: Some(format!("Invalid URL format: {}", url)),
            };
        }

        let timeout = Duration::from_secs(timeout_secs);

        match tokio::time::timeout(timeout, self.client.head(url).send()).await {
            Ok(Ok(response)) => {
                let status_code = response.status().as_u16();
                if response.status().is_success() || response.status().is_redirection() {
                    LinkStatus {
                        status: "ok".to_string(),
                        code: None,
                        message: None,
                    }
                } else if status_code == 405 {
                    // Method not allowed - try GET instead
                    match tokio::time::timeout(timeout, self.client.get(url).send()).await {
                        Ok(Ok(response)) => {
                            let status_code = response.status().as_u16();
                            if response.status().is_success() || response.status().is_redirection()
                            {
                                LinkStatus {
                                    status: "ok".to_string(),
                                    code: None,
                                    message: None,
                                }
                            } else {
                                LinkStatus {
                                    status: "error".to_string(),
                                    code: Some(status_code),
                                    message: None,
                                }
                            }
                        }
                        Ok(Err(e)) => LinkStatus {
                            status: "failed".to_string(),
                            code: None,
                            message: Some(e.to_string()),
                        },
                        Err(_) => LinkStatus {
                            status: "failed".to_string(),
                            code: None,
                            message: Some("request timed out".to_string()),
                        },
                    }
                } else {
                    LinkStatus {
                        status: "error".to_string(),
                        code: Some(status_code),
                        message: None,
                    }
                }
            }
            Ok(Err(e)) => LinkStatus {
                status: "failed".to_string(),
                code: None,
                message: Some(e.to_string()),
            },
            Err(_) => LinkStatus {
                status: "failed".to_string(),
                code: None,
                message: Some("request timed out".to_string()),
            },
        }
    }
}

impl LinkChecker for LinkCheckerImpl {
    async fn check_links(&self, input: LinkCheckInput) -> LinkCheckResult {
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

            let status = self.check_single_url(&url, input.timeout_secs).await;

            tracing::debug!(url = %url, status = %status.status, "checked link");
            results.insert(url, status);
        }

        LinkCheckResult::Success {
            output: LinkCheckOutput { results },
        }
    }
}

dodeca_cell_runtime::cell_service!(
    LinkCheckerServer<LinkCheckerImpl>,
    LinkCheckerImpl
);

dodeca_cell_runtime::run_cell!(LinkCheckerImpl::new());
