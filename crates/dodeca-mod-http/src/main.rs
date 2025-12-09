//! Dodeca HTTP module (dodeca-mod-http)
//!
//! This binary runs the HTTP server for dodeca. It:
//! - Connects to the host via rapace RPC
//! - Listens on HTTP (127.0.0.1:PORT)
//! - Handles all HTTP requests via axum router
//! - Calls back to the host for all content (HTML, CSS, static files, devtools assets)
//! - Manages devtools WebSocket connections

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use color_eyre::Result;
use rapace::StreamTransport;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::UnixStream;
use tokio::sync::broadcast;

use dodeca_serve_protocol::{ContentServiceClient, ServeContent};

mod devtools;

/// Type alias for the transport we use
type PluginTransport = StreamTransport<ReadHalf<UnixStream>, WriteHalf<UnixStream>>;

/// Plugin context shared across handlers
pub struct PluginContext {
    /// RPC client to call host for content
    pub content_client: ContentServiceClient<PluginTransport>,
    /// Live reload broadcast (plugin-internal)
    pub livereload_tx: broadcast::Sender<devtools::LiveReloadMsg>,
    /// Flag to signal shutdown when host connection is lost
    pub host_dead: AtomicBool,
}

/// Cache control headers
const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
const CACHE_NO_CACHE: &str = "no-cache, no-store, must-revalidate";

/// CLI arguments
struct Args {
    /// Unix socket path to connect to host
    host_socket: PathBuf,
    /// HTTP bind address
    bind: SocketAddr,
}

fn parse_args() -> Result<Args> {
    let mut host_socket = None;
    let mut bind = None;

    for arg in std::env::args().skip(1) {
        if let Some(value) = arg.strip_prefix("--host-socket=") {
            host_socket = Some(PathBuf::from(value));
        } else if let Some(value) = arg.strip_prefix("--bind=") {
            bind = Some(value.parse()?);
        }
    }

    Ok(Args {
        host_socket: host_socket.ok_or_else(|| color_eyre::eyre::eyre!("--host-socket required"))?,
        bind: bind.ok_or_else(|| color_eyre::eyre::eyre!("--bind required"))?,
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

    // Create rapace stream transport
    let transport: PluginTransport = StreamTransport::new(stream);
    let transport = Arc::new(transport);

    // Create content service client
    let content_client = ContentServiceClient::new(transport);

    // Create live reload broadcast
    let (livereload_tx, _) = broadcast::channel(16);

    // Build context
    let ctx = Arc::new(PluginContext {
        content_client,
        livereload_tx,
        host_dead: AtomicBool::new(false),
    });

    // Build router
    let app = build_router(ctx.clone());

    // Start HTTP server
    tracing::info!("HTTP server listening on {}", args.bind);
    let listener = tokio::net::TcpListener::bind(args.bind).await?;

    // Run server with graceful shutdown on host death
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            // Poll the host_dead flag
            loop {
                if ctx.host_dead.load(Ordering::Relaxed) {
                    tracing::error!("Host connection lost, shutting down");
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await?;

    Ok(())
}

/// Build the axum router
pub fn build_router(ctx: Arc<PluginContext>) -> Router {
    Router::new()
        // Devtools WebSocket endpoint
        .route("/_/ws", get(devtools::ws_handler))
        // Legacy endpoints
        .route("/__dodeca", get(devtools::ws_handler))
        .route("/__livereload", get(devtools::ws_handler))
        // All other content (HTML, CSS, static, devtools assets) - ask host
        .fallback(content_handler)
        .with_state(ctx)
        .layer(middleware::from_fn(log_requests))
}

/// Handle content requests
///
/// This is the "dumb frontend" - it just asks the host what to serve
/// for any path, including devtools assets like /_/*.js, /_/*.wasm.
async fn content_handler(
    State(ctx): State<Arc<PluginContext>>,
    request: Request,
) -> Response {
    let path = request.uri().path().to_string();

    // Call host to get content
    let content = match ctx.content_client.find_content(path.clone()).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("RPC error fetching {}: {:?}", path, e);
            // Signal host death - this will trigger graceful shutdown
            ctx.host_dead.store(true, Ordering::Relaxed);
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
        tracing::info!("{} {} -> {} in {:.1}ms", method, path, status, latency_ms);
    }

    response
}
