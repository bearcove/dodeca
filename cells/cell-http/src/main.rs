//! Dodeca HTTP cell (cell-http)
//!
//! This binary handles HTTP serving for dodeca via L4 tunneling:
//!
//! Architecture:
//! ```text
//! Browser → Host (TCP) → roam tunnel → Cell → internal axum
//!                                              ↓
//!                              ContentService RPC → Host (picante DB)
//!                                    ↑
//!                          (zero-copy via SHM)
//! ```
//!
//! The cell:
//! - Runs axum internally on localhost (not exposed to network)
//! - Implements TcpTunnel service (host opens tunnels for each browser connection)
//! - Calls ContentService on host for all content (HTML, CSS, static files)
//! - Opens WebSocketTunnel to host for devtools (just pipes bytes)
//! - Uses SHM transport for zero-copy content transfer

use std::sync::Arc;
use std::sync::OnceLock;

use roam::session::{ConnectionHandle, RoutedDispatcher};
use roam_shm::driver::establish_guest;
use roam_shm::guest::ShmGuest;
use roam_shm::spawn::SpawnArgs;
use roam_shm::transport::ShmGuestTransport;
use roam_tracing::{CellTracingDispatcher, init_cell_tracing};
use tracing_subscriber::prelude::*;

use cell_host_proto::{HostServiceClient, ReadyMsg};
use cell_http_proto::TcpTunnelDispatcher;

mod devtools;
mod tunnel;

/// Trait for router context - allows lazy initialization
pub trait RouterContext: Send + Sync + 'static {
    fn host_client(&self) -> HostServiceClient;
    fn handle(&self) -> &ConnectionHandle;
}

/// Lazy context that wraps `OnceLock<ConnectionHandle>`
struct LazyRouterContext {
    handle_cell: Arc<OnceLock<ConnectionHandle>>,
}

impl LazyRouterContext {
    fn get_handle(&self) -> &ConnectionHandle {
        self.handle_cell.get().expect("handle not initialized")
    }
}

impl RouterContext for LazyRouterContext {
    fn host_client(&self) -> HostServiceClient {
        HostServiceClient::new(self.get_handle().clone())
    }
    fn handle(&self) -> &ConnectionHandle {
        self.get_handle()
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = SpawnArgs::from_env()?;
    let guest = ShmGuest::attach_with_ticket(&args)?;
    let transport = ShmGuestTransport::new(guest);

    // Initialize cell-side tracing
    let (tracing_layer, tracing_service) = init_cell_tracing(1024);
    tracing_subscriber::registry().with(tracing_layer).init();

    // Lazy initialization pattern for bidirectional RPC
    let handle_cell: Arc<OnceLock<ConnectionHandle>> = Arc::new(OnceLock::new());

    let ctx: Arc<dyn RouterContext> = Arc::new(LazyRouterContext {
        handle_cell: handle_cell.clone(),
    });

    // Build axum router with lazy context
    let app = build_router(ctx.clone());

    // Create the tunnel implementation with lazy context
    let tunnel_impl = tunnel::TcpTunnelImpl::new(ctx, app);
    let user_dispatcher = TcpTunnelDispatcher::new(tunnel_impl);

    // Combine user's dispatcher with tracing dispatcher
    let tracing_dispatcher = CellTracingDispatcher::new(tracing_service);
    let dispatcher = RoutedDispatcher::new(
        tracing_dispatcher, // primary: handles tracing methods
        user_dispatcher,    // fallback: handles all cell-specific methods
    );

    let (handle, driver) = establish_guest(transport, dispatcher);

    // Spawn driver in background - must run before ready() so RPC can be processed
    let driver_handle = tokio::spawn(async move {
        if let Err(e) = driver.run().await {
            eprintln!("Driver error: {:?}", e);
        }
    });

    // Now initialize the handle cell
    let _ = handle_cell.set(handle.clone());

    // Signal readiness to host
    let host = HostServiceClient::new(handle.clone());
    host.ready(ReadyMsg {
        peer_id: args.peer_id.get() as u16,
        cell_name: "http".to_string(),
        pid: Some(std::process::id()),
        version: None,
        features: vec![],
    })
    .await?;

    // Wait for driver
    if let Err(e) = driver_handle.await {
        eprintln!("[cell-http] driver task panicked: {e:?}");
    }
    Ok(())
}

/// Build the axum router for the internal HTTP server
fn build_router(ctx: Arc<dyn RouterContext>) -> axum::Router {
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
    async fn content_handler(
        State(ctx): State<Arc<dyn RouterContext>>,
        request: Request,
    ) -> Response {
        let path = request.uri().path().to_string();
        let client = ctx.host_client();

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
            ServeContent::Html {
                content,
                route: _,
                generation,
            } => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
                .header("x-picante-generation", generation.to_string())
                .body(Body::from(content))
                .unwrap(),
            ServeContent::Css {
                content,
                generation,
            } => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
                .header(header::CACHE_CONTROL, CACHE_IMMUTABLE)
                .header("x-picante-generation", generation.to_string())
                .body(Body::from(content))
                .unwrap(),
            ServeContent::Static {
                content,
                mime,
                generation,
            } => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::CACHE_CONTROL, CACHE_IMMUTABLE)
                .header("x-picante-generation", generation.to_string())
                .body(Body::from(content))
                .unwrap(),
            ServeContent::StaticNoCache {
                content,
                mime,
                generation,
            } => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
                .header("x-picante-generation", generation.to_string())
                .body(Body::from(content))
                .unwrap(),
            ServeContent::Search {
                content,
                mime,
                generation,
            } => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header("x-picante-generation", generation.to_string())
                .body(Body::from(content))
                .unwrap(),
            ServeContent::Redirect {
                location,
                generation,
            } => Response::builder()
                .status(StatusCode::FOUND) // 302 Temporary Redirect
                .header(header::LOCATION, location)
                .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
                .header("x-picante-generation", generation.to_string())
                .body(Body::empty())
                .unwrap(),
            ServeContent::NotFound { html, generation } => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
                .header("x-picante-generation", generation.to_string())
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

        // Add header to identify this is served by the cell
        response
            .headers_mut()
            .insert("x-served-by", "cell-http".parse().unwrap());

        let status = response.status().as_u16();
        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

        if status >= 500 {
            tracing::error!("{} {} -> {} in {:.1}ms", method, path, status, latency_ms);
        } else {
            // 2xx, 3xx, 4xx are all normal in a dev server (404s are common during probing)
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
