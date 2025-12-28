//! Dodeca HTTP cell (cell-http)
//!
//! This binary handles HTTP serving for dodeca via L4 tunneling:
//!
//! Architecture:
//! ```text
//! Browser → Host (TCP) → rapace tunnel → Cell → internal axum
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

use rapace::transport::shm::ShmTransport;
use rapace_cell::CellSession;

use cell_http_proto::{ContentServiceClient, TcpTunnelServer, WebSocketTunnelClient};

mod devtools;
mod tunnel;

/// Cell context shared across HTTP handlers
pub struct CellContext {
    /// RPC session for bidirectional communication with host
    pub session: Arc<CellSession>,
}

impl CellContext {
    /// Create a ContentServiceClient for calling the host
    pub fn content_client(&self) -> ContentServiceClient<ShmTransport> {
        ContentServiceClient::new(self.session.clone())
    }

    /// Create a WebSocketTunnelClient for opening devtools tunnels to host
    pub fn ws_tunnel_client(&self) -> WebSocketTunnelClient<ShmTransport> {
        WebSocketTunnelClient::new(self.session.clone())
    }
}

rapace_cell::cell_service!(
    TcpTunnelServer<tunnel::TcpTunnelImpl>,
    tunnel::TcpTunnelImpl
);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run_with_session(|session: Arc<CellSession>| {
        // Build cell context
        let ctx = Arc::new(CellContext {
            session: session.clone(),
        });

        // Build axum router
        let app = build_router(ctx);

        // Create the tunnel implementation
        let tunnel_impl = tunnel::TcpTunnelImpl::new(session, app);

        CellService::from(tunnel_impl)
    })
    .await?;
    Ok(())
}

/// Build the axum router for the internal HTTP server
fn build_router(ctx: Arc<CellContext>) -> axum::Router {
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
    async fn content_handler(State(ctx): State<Arc<CellContext>>, request: Request) -> Response {
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
