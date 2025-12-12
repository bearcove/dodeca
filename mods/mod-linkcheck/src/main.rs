//! Dodeca linkcheck plugin (dodeca-mod-linkcheck)
//!
//! This plugin handles external link checking with per-domain rate limiting.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use color_eyre::Result;
use dodeca_plugin_runtime::{PluginTracing, add_tracing_service};
use rapace::RpcSession;
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace_plugin::{DispatcherBuilder, ServiceDispatch};
use url::Url;

use mod_linkcheck_proto::{
    LinkCheckInput, LinkCheckOutput, LinkCheckResult, LinkChecker, LinkCheckerServer, LinkStatus,
};

/// Type alias for our transport (SHM-based for zero-copy)
type PluginTransport = ShmTransport;

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

/// Service wrapper for LinkChecker to satisfy ServiceDispatch
struct LinkCheckerService(Arc<LinkCheckerServer<LinkCheckerImpl>>);

impl ServiceDispatch for LinkCheckerService {
    fn dispatch(
        &self,
        method_id: u32,
        payload: &[u8],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<rapace::Frame, rapace::RpcError>>
                + Send
                + 'static,
        >,
    > {
        let server = self.0.clone();
        let bytes = payload.to_vec();
        Box::pin(async move { server.dispatch(method_id, &bytes).await })
    }
}

/// SHM configuration - must match host's config
const SHM_CONFIG: ShmSessionConfig = ShmSessionConfig {
    ring_capacity: 256, // 256 descriptors in flight
    slot_size: 65536,   // 64KB per slot
    slot_count: 128,    // 128 slots = 8MB total
};

/// CLI arguments
struct Args {
    /// SHM file path for zero-copy communication with host
    shm_path: PathBuf,
}

fn parse_args() -> Result<Args> {
    let mut shm_path = None;

    for arg in std::env::args().skip(1) {
        if let Some(value) = arg.strip_prefix("--shm-path=") {
            shm_path = Some(PathBuf::from(value));
        }
    }

    Ok(Args {
        shm_path: shm_path.ok_or_else(|| color_eyre::eyre::eyre!("--shm-path required"))?,
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let args = parse_args()?;

    // Wait for the host to create the SHM file
    for i in 0..50 {
        if args.shm_path.exists() {
            break;
        }
        if i == 49 {
            return Err(color_eyre::eyre::eyre!(
                "SHM file not created by host: {}",
                args.shm_path.display()
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Open the SHM session (plugin side)
    let shm_session = ShmSession::open_file(&args.shm_path, SHM_CONFIG)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to open SHM: {:?}", e))?;

    // Create SHM transport
    let transport: Arc<PluginTransport> = Arc::new(ShmTransport::new(shm_session));

    // Plugin uses even channel IDs (2, 4, 6, ...)
    // Host uses odd channel IDs (1, 3, 5, ...)
    let session = Arc::new(RpcSession::with_channel_start(transport, 2));

    // Initialize tracing to forward logs to host via RapaceTracingLayer
    let PluginTracing { tracing_config, .. } = dodeca_plugin_runtime::init_tracing(session.clone());

    tracing::info!("Connected to host via SHM");

    // Wrap services with rapace-plugin multi-service dispatcher
    let dispatcher = DispatcherBuilder::new();
    let dispatcher = add_tracing_service(dispatcher, tracing_config);
    let dispatcher = dispatcher.add_service(LinkCheckerService(Arc::new(LinkCheckerServer::new(
        LinkCheckerImpl::new(),
    ))));
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    // Run the RPC session demux loop (this is the main event loop now)
    tracing::info!("Linkcheck plugin ready, waiting for requests");
    if let Err(e) = session.run().await {
        tracing::error!(error = ?e, "RPC session error - host connection lost");
    }

    Ok(())
}
