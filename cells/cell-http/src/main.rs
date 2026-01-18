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

use dodeca_cell_runtime::{ConnectionHandle, run_cell};

use cell_host_proto::HostServiceClient;
use cell_http_proto::TcpTunnelDispatcher;

mod devtools;
mod tunnel;
mod vite;

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

fn main() {
    let result = run_cell!("http", |handle| {
        let ctx: Arc<dyn RouterContext> = Arc::new(LazyRouterContext {
            handle_cell: handle,
        });

        // Build axum router with lazy context
        let app = build_router(ctx.clone());

        // Create the tunnel implementation with lazy context
        let tunnel_impl = tunnel::TcpTunnelImpl::new(ctx, app);
        TcpTunnelDispatcher::new(tunnel_impl)
    });

    if let Err(e) = result {
        eprintln!("[cell-http] error: {e}");
        std::process::exit(1);
    }
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

        // Check if this should be proxied to Vite
        if vite::is_vite_path(&path) || vite::is_vite_hmr_websocket(&request) {
            return vite::vite_proxy_handler(State(ctx), request).await;
        }

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
