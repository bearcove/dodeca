//! Vite dev server proxy
//!
//! Proxies HTTP requests and WebSocket connections to a Vite dev server
//! for seamless frontend development with HMR support.

use axum::{
    body::Body,
    extract::{FromRequestParts, State, WebSocketUpgrade, ws::Message},
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

use crate::RouterContext;

/// Localhost addresses to try when connecting to Vite.
/// IPv6 first (common default on modern systems), then IPv4.
const LOCALHOST_ADDRS: &[&str] = &["[::1]", "127.0.0.1"];

/// Cached Vite port - fetched once from host
static VITE_PORT: OnceCell<Option<u16>> = OnceCell::const_new();

/// Get the Vite port, fetching from host if not yet cached
async fn get_vite_port(ctx: &Arc<dyn RouterContext>) -> Option<u16> {
    *VITE_PORT
        .get_or_init(|| async {
            match ctx.host_client().get_vite_port().await {
                Ok(port) => {
                    tracing::trace!(port = ?port, "fetched vite port from host");
                    port
                }
                Err(e) => {
                    tracing::warn!(error = ?e, "failed to get vite port from host");
                    None
                }
            }
        })
        .await
}

/// Check if a path should be proxied to Vite
pub fn is_vite_path(path: &str) -> bool {
    let is_vite = path.starts_with("/@vite/")
        || path.starts_with("/src/")
        || path.starts_with("/@id/")
        || path.starts_with("/@fs/")
        || path.starts_with("/@react-refresh")
        || path.starts_with("/__vite-plugin")
        || path.starts_with("/node_modules/.vite/")
        || path.starts_with("/node_modules/")
        || path.ends_with(".hot-update.json")
        || path.ends_with(".hot-update.js")
        // Common frontend extensions that Vite serves
        || path.ends_with(".ts")
        || path.ends_with(".tsx")
        || path.ends_with(".jsx")
        || path.ends_with(".vue")
        || path.ends_with(".svelte");

    tracing::trace!(path = %path, is_vite = %is_vite, "checked if path is vite path");
    is_vite
}

/// Check if request is a Vite HMR WebSocket upgrade
pub fn is_vite_hmr_websocket(req: &Request<Body>) -> bool {
    // Must be a websocket upgrade
    let is_ws = req
        .headers()
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));

    if !is_ws {
        return false;
    }

    // Check for vite-hmr protocol
    req.headers()
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("vite-hmr"))
}

fn is_websocket_upgrade(req: &Request<Body>) -> bool {
    req.headers()
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

/// Proxy requests to Vite dev server (handles both HTTP and WebSocket)
pub async fn vite_proxy_handler(
    State(ctx): State<Arc<dyn RouterContext>>,
    req: Request<Body>,
) -> Response<Body> {
    let vite_port = match get_vite_port(&ctx).await {
        Some(p) => {
            tracing::trace!(port = %p, "vite port available");
            p
        }
        None => {
            tracing::debug!("vite server not running, returning 404");
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Vite server not running"))
                .unwrap();
        }
    };

    let method = req.method().clone();
    let original_uri = req.uri().to_string();
    let path = req.uri().path().to_string();
    let query = req
        .uri()
        .query()
        .map(|q| format!("?{}", q))
        .unwrap_or_default();

    tracing::debug!(method = %method, path = %path, "proxying request to vite");

    // Check if this is a WebSocket upgrade request (for HMR)
    if is_websocket_upgrade(&req) {
        tracing::debug!(uri = %original_uri, "detected websocket upgrade for vite HMR");
        return handle_websocket_upgrade(req, vite_port, path, query).await;
    }

    // Regular HTTP proxy
    proxy_http_request(req, vite_port, &method, &path, &query).await
}

/// Handle WebSocket upgrade for Vite HMR
async fn handle_websocket_upgrade(
    req: Request<Body>,
    vite_port: u16,
    path: String,
    query: String,
) -> Response<Body> {
    // Log the incoming protocol header
    if let Some(protocol) = req.headers().get("sec-websocket-protocol") {
        tracing::debug!(protocol = ?protocol, "incoming websocket protocol header");
    } else {
        tracing::warn!("no sec-websocket-protocol header in request");
    }

    let (mut parts, _body) = req.into_parts();

    let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::warn!(error = %e, "failed to extract websocket upgrade");
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from(format!("WebSocket upgrade failed: {}", e)))
                .unwrap();
        }
    };

    tracing::trace!(path = %path, "upgrading to websocket for vite HMR");

    ws.protocols(["vite-hmr"])
        .on_upgrade(move |socket| async move {
            tracing::debug!(path = %path, "websocket connection established, starting vite proxy");
            if let Err(e) = handle_vite_ws(socket, vite_port, &path, &query).await {
                tracing::warn!(error = %e, path = %path, "vite websocket proxy error");
            }
            tracing::debug!(path = %path, "vite websocket connection closed");
        })
        .into_response()
}

/// Proxy an HTTP request to Vite, trying both IPv6 and IPv4
async fn proxy_http_request(
    req: Request<Body>,
    vite_port: u16,
    method: &hyper::Method,
    path: &str,
    query: &str,
) -> Response<Body> {
    // Capture headers before consuming body
    let headers: Vec<_> = req
        .headers()
        .iter()
        .filter(|(name, _)| *name != header::HOST)
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();

    tracing::trace!(header_count = %headers.len(), "captured request headers");

    // Buffer body for potential retry across addresses
    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => {
            tracing::trace!(body_size = %b.len(), "buffered request body");
            b
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to read request body for vite proxy");
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from("Failed to read request body"))
                .unwrap();
        }
    };

    let client = Client::builder(TokioExecutor::new()).build_http();
    let mut last_error = None;

    for addr in LOCALHOST_ADDRS {
        let target_uri = format!("http://{}:{}{}{}", addr, vite_port, path, query);
        tracing::trace!(target = %target_uri, "attempting vite proxy connection");

        let mut proxy_req_builder = Request::builder().method(method.clone()).uri(&target_uri);

        for (name, value) in &headers {
            proxy_req_builder = proxy_req_builder.header(name, value);
        }

        let proxy_req = proxy_req_builder
            .body(Body::from(body_bytes.clone()))
            .unwrap();

        match client.request(proxy_req).await {
            Ok(res) => {
                let status = res.status();
                if status.is_server_error() {
                    // Vite returned 5xx - log details for debugging
                    let response_headers: Vec<_> = res
                        .headers()
                        .iter()
                        .map(|(k, v)| format!("{}={:?}", k, v))
                        .collect();
                    let request_headers: Vec<_> = headers
                        .iter()
                        .map(|(k, v)| format!("{}={:?}", k, v))
                        .collect();
                    tracing::warn!(
                        status = %status,
                        path = %path,
                        addr = %addr,
                        ?response_headers,
                        ?request_headers,
                        "vite returned server error"
                    );
                } else {
                    tracing::debug!(
                        status = %status,
                        path = %path,
                        addr = %addr,
                        "vite proxy success"
                    );
                }
                let (parts, body) = res.into_parts();
                return Response::from_parts(parts, Body::new(body));
            }
            Err(e) => {
                tracing::trace!(
                    error = %e,
                    addr = %addr,
                    path = %path,
                    "vite proxy attempt failed, will try next address"
                );
                last_error = Some((e, addr));
            }
        }
    }

    // All attempts failed
    let (error, last_addr) = last_error.unwrap();
    tracing::warn!(
        error = %error,
        path = %path,
        last_addr = %last_addr,
        addrs_tried = ?LOCALHOST_ADDRS,
        "vite proxy failed on all addresses"
    );
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .body(Body::from(format!("Vite proxy error: {}", error)))
        .unwrap()
}

/// Handle WebSocket proxy to Vite for HMR
async fn handle_vite_ws(
    client_socket: axum::extract::ws::WebSocket,
    vite_port: u16,
    path: &str,
    query: &str,
) -> eyre::Result<()> {
    use tokio_tungstenite::connect_async_with_config;
    use tokio_tungstenite::tungstenite::http::Request;

    // Try both IPv6 and IPv4 for WebSocket too
    let mut last_error = None;

    for addr in LOCALHOST_ADDRS {
        let vite_url = format!("ws://{}:{}{}{}", addr, vite_port, path, query);
        tracing::trace!(vite_url = %vite_url, "attempting vite websocket connection");

        let request = Request::builder()
            .uri(&vite_url)
            .header("Sec-WebSocket-Protocol", "vite-hmr")
            .header("Host", format!("{}:{}", addr, vite_port))
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(())
            .unwrap();

        let connect_timeout = Duration::from_secs(5);
        let connect_result = tokio::time::timeout(
            connect_timeout,
            connect_async_with_config(request, None, false),
        )
        .await;

        match connect_result {
            Ok(Ok((vite_ws, _response))) => {
                tracing::debug!(addr = %addr, path = %path, "vite websocket connected");
                return run_websocket_proxy(client_socket, vite_ws).await;
            }
            Ok(Err(e)) => {
                tracing::trace!(
                    addr = %addr,
                    error = %e,
                    "vite websocket connection failed, trying next address"
                );
                last_error = Some(e.into());
            }
            Err(_) => {
                tracing::trace!(
                    addr = %addr,
                    timeout_secs = %connect_timeout.as_secs(),
                    "vite websocket connection timed out, trying next address"
                );
                last_error = Some(eyre::eyre!(
                    "Timeout connecting to Vite WebSocket after {:?}",
                    connect_timeout
                ));
            }
        }
    }

    let error = last_error.unwrap();
    tracing::warn!(
        error = %error,
        path = %path,
        addrs_tried = ?LOCALHOST_ADDRS,
        "vite websocket failed on all addresses"
    );
    Err(error)
}

/// Run the bidirectional WebSocket proxy
async fn run_websocket_proxy(
    client_socket: axum::extract::ws::WebSocket,
    vite_ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> eyre::Result<()> {
    let (mut client_tx, mut client_rx) = client_socket.split();
    let (mut vite_tx, mut vite_rx) = vite_ws.split();

    let client_to_vite = async {
        while let Some(msg) = client_rx.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    tracing::trace!(len = %text.len(), "client->vite text message");
                    let text_str: String = text.to_string();
                    if vite_tx
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            text_str.into(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Binary(data)) => {
                    tracing::trace!(len = %data.len(), "client->vite binary message");
                    let data_vec: Vec<u8> = data.to_vec();
                    if vite_tx
                        .send(tokio_tungstenite::tungstenite::Message::Binary(
                            data_vec.into(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::trace!("client sent close");
                    break;
                }
                Err(e) => {
                    tracing::trace!(error = %e, "client websocket error");
                    break;
                }
                _ => {}
            }
        }
    };

    let vite_to_client = async {
        while let Some(msg) = vite_rx.next().await {
            match msg {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    tracing::trace!(len = %text.len(), "vite->client text message");
                    let text_str: String = text.to_string();
                    if client_tx
                        .send(Message::Text(text_str.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Binary(data)) => {
                    tracing::trace!(len = %data.len(), "vite->client binary message");
                    let data_vec: Vec<u8> = data.to_vec();
                    if client_tx
                        .send(Message::Binary(data_vec.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                    tracing::trace!("vite sent close");
                    break;
                }
                Err(e) => {
                    tracing::trace!(error = %e, "vite websocket error");
                    break;
                }
                _ => {}
            }
        }
    };

    tokio::select! {
        _ = client_to_vite => {
            tracing::trace!("client->vite direction ended");
        }
        _ = vite_to_client => {
            tracing::trace!("vite->client direction ended");
        }
    }

    Ok(())
}
