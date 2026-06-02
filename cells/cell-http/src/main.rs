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
//! - Runs roam RPC session for devtools (forwarded to host via virtual connection)
//! - Uses SHM transport for zero-copy content transfer

use std::sync::Arc;

use dodeca_cell_runtime::HostHandle;

use cell_host_proto::HostServiceClient;
use cell_http_proto::TcpTunnelDispatcher;

mod devtools;
mod tunnel;
mod vite;

/// Router context: yields a `HostServiceClient` for calling back to the host.
pub trait RouterContext: Send + Sync + 'static {
    /// The cell's host handle (clone-cheap); `.client().await` opens the
    /// `HostService` virtual connection on first use.
    fn host(&self) -> HostHandle;
}

struct CellRouterContext {
    host: HostHandle,
}

impl RouterContext for CellRouterContext {
    fn host(&self) -> HostHandle {
        self.host.clone()
    }
}

/// Build the forwarded auth identity from a request's oauth2-proxy identity
/// headers. Prefers the `X-Auth-Request-*` set (what oauth2-proxy emits with
/// `--set-xauthrequest` in forward-auth / static-upstream mode — the shape the
/// cluster-wide Traefik forwardAuth gate copies onto the request), falling back
/// to `X-Forwarded-*` (set by `--pass-user-headers` when oauth2-proxy is an
/// inline reverse proxy). `None` if no user header is present — i.e.
/// unauthenticated. HTTP/header parsing stays in the cell; the host only ever
/// sees the resolved [`cell_http_proto::Identity`].
fn extract_identity(request: &axum::extract::Request) -> Option<cell_http_proto::Identity> {
    let headers = request.headers();
    // First non-empty value among `names`, tried in order.
    let get = |names: &[&str]| {
        names.iter().find_map(|name| {
            headers
                .get(*name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
                .filter(|v| !v.is_empty())
        })
    };
    let user = get(&["x-auth-request-user", "x-forwarded-user"])?;
    let groups = get(&["x-auth-request-groups", "x-forwarded-groups"])
        .map(|g| {
            g.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Some(cell_http_proto::Identity {
        user,
        email: get(&["x-auth-request-email", "x-forwarded-email"]).unwrap_or_default(),
        name: get(&[
            "x-auth-request-preferred-username",
            "x-forwarded-preferred-username",
        ])
        .unwrap_or_default(),
        groups,
    })
}

/// Convenience: get a connected `HostServiceClient`.
pub async fn host_client(ctx: &Arc<dyn RouterContext>) -> HostServiceClient {
    ctx.host().client().await
}

dodeca_cell_runtime::declare_cell!("http", |host| {
    let ctx: Arc<dyn RouterContext> = Arc::new(CellRouterContext { host });
    let app = build_router(ctx.clone());
    let tunnel_impl = tunnel::TcpTunnelImpl::new(ctx, app);
    TcpTunnelDispatcher::new(tunnel_impl)
});

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

        let client = crate::host_client(&ctx).await;

        // The forwarded auth identity (oauth2-proxy → these headers). The host
        // uses it to gate `/_dodeca/*`. HTTP/header parsing lives here in the
        // cell; the host only sees the resolved identity.
        let identity = extract_identity(&request);

        // Call host to get content
        let host_call_started_at = Instant::now();
        tracing::debug!(path, "http cell host find_content started");
        let content = match client.find_content(path.clone(), identity).await {
            Ok(c) => {
                tracing::debug!(
                    path,
                    elapsed_ms = host_call_started_at.elapsed().as_millis(),
                    "http cell host find_content finished"
                );
                c
            }
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
                .header(header::CONNECTION, "close")
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
                .header(header::CONNECTION, "close")
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
                .header(header::CONNECTION, "close")
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
                .header(header::CONNECTION, "close")
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
                .header(header::CONNECTION, "close")
                .header("x-picante-generation", generation.to_string())
                .body(Body::empty())
                .unwrap(),
            ServeContent::NotFound { html, generation } => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, CACHE_NO_CACHE)
                .header(header::CONNECTION, "close")
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
            tracing::error!(status, latency_ms, "{} {} -> {}", method, path, status);
        } else {
            tracing::info!(status, latency_ms, "{} {} -> {}", method, path, status);
        }

        response
    }

    Router::new()
        // Devtools WebSocket - roam RPC session forwarded to host
        .route("/_/ws", get(devtools::ws_handler))
        // Legacy endpoints
        .route("/__dodeca", get(devtools::ws_handler))
        .route("/__livereload", get(devtools::ws_handler))
        // All other content (HTML, CSS, static, devtools assets) - ask host
        .fallback(content_handler)
        .with_state(ctx)
        .layer(middleware::from_fn(log_requests))
}
