//! Dodeca CSS plugin (dodeca-mod-css)
//!
//! This plugin handles CSS URL rewriting and minification via lightningcss.

use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::Result;
use dodeca_plugin_runtime::{PluginTracing, add_tracing_service};
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::visitor::Visit;
use rapace::RpcSession;
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace_plugin::{DispatcherBuilder, ServiceDispatch};

use mod_css_proto::{CssProcessor, CssProcessorServer};

/// Type alias for our transport (SHM-based for zero-copy)
type PluginTransport = ShmTransport;

/// CSS processor implementation
pub struct CssProcessorImpl;

impl CssProcessor for CssProcessorImpl {
    async fn rewrite_and_minify(&self, css: String, path_map: std::collections::HashMap<String, String>) -> mod_css_proto::CssResult {
        // Parse the CSS
        let mut stylesheet = match StyleSheet::parse(&css, ParserOptions::default()) {
            Ok(s) => s,
            Err(e) => {
                return mod_css_proto::CssResult::Error {
                    message: format!("Failed to parse CSS: {:?}", e),
                };
            }
        };

        // Visit and rewrite URLs
        let mut visitor = UrlRewriter {
            path_map: &path_map,
        };
        if let Err(e) = stylesheet.visit(&mut visitor) {
            return mod_css_proto::CssResult::Error {
                message: format!("Failed to visit CSS: {:?}", e),
            };
        }

        // Serialize back to string with minification enabled
        let printer_options = PrinterOptions {
            minify: true,
            ..Default::default()
        };
        match stylesheet.to_css(printer_options) {
            Ok(result) => mod_css_proto::CssResult::Success { css: result.code },
            Err(e) => mod_css_proto::CssResult::Error {
                message: format!("Failed to serialize CSS: {:?}", e),
            },
        }
    }
}

/// Service wrapper for CssProcessor to satisfy ServiceDispatch
struct CssProcessorService(Arc<CssProcessorServer<CssProcessorImpl>>);

impl ServiceDispatch for CssProcessorService {
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
    slot_size: 65536,   // 64KB per slot (fits most CSS files)
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
    let dispatcher = dispatcher.add_service(CssProcessorService(Arc::new(CssProcessorServer::new(
        CssProcessorImpl,
    ))));
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    // Run the RPC session demux loop (this is the main event loop now)
    tracing::info!("CSS plugin ready, waiting for requests");
    if let Err(e) = session.run().await {
        tracing::error!(error = ?e, "RPC session error - host connection lost");
    }

    Ok(())
}



/// Visitor that rewrites URLs in CSS
struct UrlRewriter<'a> {
    path_map: &'a std::collections::HashMap<String, String>,
}

impl<'i, 'a> lightningcss::visitor::Visitor<'i> for UrlRewriter<'a> {
    type Error = std::convert::Infallible;

    fn visit_types(&self) -> lightningcss::visitor::VisitTypes {
        lightningcss::visit_types!(URLS)
    }

    fn visit_url(
        &mut self,
        url: &mut lightningcss::values::url::Url<'i>,
    ) -> Result<(), Self::Error> {
        let url_str = url.url.as_ref();
        if let Some(new_url) = self.path_map.get(url_str) {
            url.url = new_url.clone().into();
        }
        Ok(())
    }
}