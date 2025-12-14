//! Dodeca HTTP module (dodeca-mod-http)
//!
//! This binary handles HTTP serving for dodeca via L4 tunneling:
//!
//! Architecture:
//! ```text
//! Browser → Host (TCP) → rapace tunnel → Plugin → internal axum
//!                                              ↓
//!                              ContentService RPC → Host (Salsa DB)
//!                                    ↑
//!                          (zero-copy via SHM)
//! ```
//!
//! The plugin:
//! - Runs axum internally on localhost (not exposed to network)
//! - Implements TcpTunnel service (host opens tunnels for each browser connection)
//! - Calls ContentService on host for all content (HTML, CSS, static files)
//! - Opens WebSocketTunnel to host for devtools (just pipes bytes)
//! - Uses SHM transport for zero-copy content transfer

use std::sync::Arc;

use color_eyre::Result;
use dodeca_cell_runtime::{CellTracing, add_tracing_service};
use rapace::RpcSession;
use rapace::transport::shm::HubPeerTransport;
use rapace_cell::{DispatcherBuilder, ServiceDispatch};

use cell_http_proto::{ContentServiceClient, TcpTunnelServer, WebSocketTunnelClient};

mod devtools;
mod tunnel;

/// Type alias for our transport (SHM-based for zero-copy)
type PluginTransport = HubPeerTransport;

/// Plugin context shared across HTTP handlers
pub struct PluginContext {
    /// RPC session for bidirectional communication with host
    pub session: Arc<RpcSession<PluginTransport>>,
}

impl PluginContext {
    /// Create a ContentServiceClient for calling the host
    pub fn content_client(&self) -> ContentServiceClient<PluginTransport> {
        ContentServiceClient::new(self.session.clone())
    }

    /// Create a WebSocketTunnelClient for opening devtools tunnels to host
    pub fn ws_tunnel_client(&self) -> WebSocketTunnelClient<PluginTransport> {
        WebSocketTunnelClient::new(self.session.clone())
    }
}

/// Service wrapper for TcpTunnel to satisfy ServiceDispatch
struct TcpTunnelService(Arc<TcpTunnelServer<tunnel::TcpTunnelImpl<PluginTransport>>>);

impl ServiceDispatch for TcpTunnelService {
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

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;

    // Use standardized argument parsing and transport creation
    let args = dodeca_cell_runtime::parse_args()?;
    let transport = dodeca_cell_runtime::create_hub_transport(&args).await?;

    // Plugin uses even channel IDs (2, 4, 6, ...)
    // Host uses odd channel IDs (1, 3, 5, ...)
    let session = Arc::new(RpcSession::with_channel_start(transport, 2));

    // Initialize tracing to forward logs to host via RapaceTracingLayer
    // The host controls the filter level via TracingConfig RPC
    let CellTracing { tracing_config, .. } = dodeca_cell_runtime::init_tracing(session.clone());

    tracing::info!("Connected to host via SHM");

    // Build plugin context
    let ctx = Arc::new(PluginContext {
        session: session.clone(),
    });

    // Build axum router (service)
    let app = build_router(ctx);

    // Create the tunnel implementation (host calls this to open HTTP tunnels)
    // Pass the app so we can serve it directly on tunnel streams
    let tunnel_impl = tunnel::TcpTunnelImpl::new(session.clone(), app);

    // Wrap services with rapace-cell multi-service dispatcher
    let dispatcher = DispatcherBuilder::new();
    let dispatcher = add_tracing_service(dispatcher, tracing_config);
    let dispatcher = dispatcher.add_service(TcpTunnelService(Arc::new(TcpTunnelServer::new(
        tunnel_impl,
    ))));
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    // Run the RPC session demux loop (this is the main event loop now)
    tracing::info!("Plugin ready, waiting for tunnel connections");
    if let Err(e) = session.run().await {
        tracing::error!(error = ?e, "RPC session error - host connection lost");
    }

    Ok(())
}

/// Build the axum router for the internal HTTP server
fn build_router(ctx: Arc<PluginContext>) -> axum::Router {
    use axum::{
        Router,
        body::Body,
        extract::{Request, State},
        http::{StatusCode, header},
        middleware::{self, Next},
        response::Response,
        routing::get,
    };
    use std::time::Instant;

    use cell_http_proto::ServeContent;

    /// Cache control headers
    const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
    const CACHE_NO_CACHE: &str = "no-cache, no-store, must-revalidate";

    /// Handle content requests by calling host via RPC
    async fn content_handler(State(ctx): State<Arc<PluginContext>>, request: Request) -> Response {
        let path = request.uri().path().to_string();
        let client = ctx.content_client();

        // Call host to get content
        let content = match client.find_content(path.clone()).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("RPC error fetching {}: {:?}", path, e);
                return Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Body::from("Host connection lost"))
                    .unwrap();
            }
        };

        // Convert ServeContent to HTTP response
        match content {
            ServeContent::Html { content, route: _ } => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
                .body(Body::from(content))
                .unwrap(),
            ServeContent::Css { content } => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
                .header(header::CACHE_CONTROL, CACHE_IMMUTABLE)
                .body(Body::from(content))
                .unwrap(),
            ServeContent::Static { content, mime } => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::CACHE_CONTROL, CACHE_IMMUTABLE)
                .body(Body::from(content))
                .unwrap(),
            ServeContent::StaticNoCache { content, mime } => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
                .body(Body::from(content))
                .unwrap(),
            ServeContent::Search { content, mime } => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .body(Body::from(content))
                .unwrap(),
            ServeContent::NotFound { html } => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
                .body(Body::from(html))
                .unwrap(),
        }
    }

    /// Logging middleware
    async fn log_requests(request: Request, next: Next) -> Response {
        let method = request.method().to_string();
        let path = request.uri().path().to_string();
        let start = Instant::now();

        let mut response = next.run(request).await;

        // Add header to identify this is served by the plugin
        response
            .headers_mut()
            .insert("x-served-by", "dodeca-mod-http".parse().unwrap());

        let status = response.status().as_u16();
        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

        if status >= 500 {
            tracing::error!("{} {} -> {} in {:.1}ms", method, path, status, latency_ms);
        } else if status >= 400 {
            tracing::warn!("{} {} -> {} in {:.1}ms", method, path, status, latency_ms);
        } else {
            tracing::debug!("{} {} -> {} in {:.1}ms", method, path, status, latency_ms);
        }

        response
    }

    Router::new()
        // Devtools WebSocket - opens tunnel to host, just pipes bytes
        .route("/_/ws", get(devtools::ws_handler))
        // Legacy endpoints
        .route("/__dodeca", get(devtools::ws_handler))
        .route("/__livereload", get(devtools::ws_handler))
        // All other content (HTML, CSS, static, devtools assets) - ask host
        .fallback(content_handler)
        .with_state(ctx)
        .layer(middleware::from_fn(log_requests))
}
