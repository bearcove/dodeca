//! Dodeca HTTP module (dodeca-mod-http)
//!
//! This binary handles HTTP serving for dodeca via L4 tunneling:
//!
//! Architecture:
//! ```text
//! Browser → Host (TCP) → rapace tunnel → Plugin → internal axum
//!                                              ↓
//!                              ContentService RPC → Host (Salsa DB)
//! ```
//!
//! The plugin:
//! - Runs axum internally on localhost (not exposed to network)
//! - Implements TcpTunnel service (host opens tunnels for each browser connection)
//! - Calls ContentService on host for all content (HTML, CSS, static files)
//! - Opens WebSocketTunnel to host for devtools (just pipes bytes)

use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::Result;
use rapace::StreamTransport;
use rapace_testkit::RpcSession;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::UnixStream;

use dodeca_serve_protocol::{
    ContentServiceClient,
    WebSocketTunnelClient,
};

mod devtools;
mod tunnel;

/// Type alias for our transport
type PluginTransport = StreamTransport<ReadHalf<UnixStream>, WriteHalf<UnixStream>>;

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

/// CLI arguments
struct Args {
    /// Unix socket path to connect to host
    host_socket: PathBuf,
}

fn parse_args() -> Result<Args> {
    let mut host_socket = None;

    for arg in std::env::args().skip(1) {
        if let Some(value) = arg.strip_prefix("--host-socket=") {
            host_socket = Some(PathBuf::from(value));
        }
        // Note: --bind is no longer used - host does the TCP binding
    }

    Ok(Args {
        host_socket: host_socket.ok_or_else(|| color_eyre::eyre::eyre!("--host-socket required"))?,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("dodeca_mod_http=info".parse()?)
        )
        .init();

    let args = parse_args()?;
    tracing::info!("Connecting to host: {}", args.host_socket.display());

    // Connect to host via Unix socket
    let stream = UnixStream::connect(&args.host_socket).await?;
    tracing::info!("Connected to host");

    // Create rapace stream transport wrapped in RpcSession
    let transport: PluginTransport = StreamTransport::new(stream);
    let transport = Arc::new(transport);

    // Plugin uses even channel IDs (2, 4, 6, ...)
    // Host uses odd channel IDs (1, 3, 5, ...)
    let session = Arc::new(RpcSession::with_channel_start(transport, 2));

    // Start internal HTTP server on localhost (OS-assigned port)
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let internal_port = listener.local_addr()?.port();
    tracing::info!("Internal HTTP server on 127.0.0.1:{}", internal_port);

    // Build plugin context
    let ctx = Arc::new(PluginContext {
        session: session.clone(),
    });

    // Create the tunnel service (host calls this to open HTTP tunnels)
    let tunnel_service = Arc::new(tunnel::TcpTunnelImpl::new(session.clone(), internal_port));

    // Set dispatcher for TcpTunnel service (host → plugin calls)
    session.set_dispatcher(tunnel::create_tunnel_dispatcher(tunnel_service));

    // Spawn the RPC session demux loop
    let session_clone = session.clone();
    tokio::spawn(async move {
        if let Err(e) = session_clone.run().await {
            tracing::error!(error = ?e, "RPC session error - host connection lost");
        }
    });

    // Build axum router
    let app = build_router(ctx);

    // Run internal HTTP server
    tracing::info!("Plugin ready, waiting for tunnel connections");
    axum::serve(listener, app).await?;

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

    use dodeca_serve_protocol::ServeContent;

    /// Cache control headers
    const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
    const CACHE_NO_CACHE: &str = "no-cache, no-store, must-revalidate";

    /// Handle content requests by calling host via RPC
    async fn content_handler(
        State(ctx): State<Arc<PluginContext>>,
        request: Request,
    ) -> Response {
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
            ServeContent::Html { content, route: _ } => {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
                    .body(Body::from(content))
                    .unwrap()
            }
            ServeContent::Css { content } => {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
                    .header(header::CACHE_CONTROL, CACHE_IMMUTABLE)
                    .body(Body::from(content))
                    .unwrap()
            }
            ServeContent::Static { content, mime } => {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime)
                    .header(header::CACHE_CONTROL, CACHE_IMMUTABLE)
                    .body(Body::from(content))
                    .unwrap()
            }
            ServeContent::StaticNoCache { content, mime } => {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime)
                    .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
                    .body(Body::from(content))
                    .unwrap()
            }
            ServeContent::Search { content, mime } => {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime)
                    .body(Body::from(content))
                    .unwrap()
            }
            ServeContent::NotFound { similar_routes: _ } => {
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .body(Body::from("Not Found"))
                    .unwrap()
            }
        }
    }

    /// Logging middleware
    async fn log_requests(request: Request, next: Next) -> Response {
        let method = request.method().to_string();
        let path = request.uri().path().to_string();
        let start = Instant::now();

        let mut response = next.run(request).await;

        // Add header to identify this is served by the plugin
        response.headers_mut().insert(
            "x-served-by",
            "dodeca-mod-http".parse().unwrap(),
        );

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
