//! Vite dev server proxy
//!
//! Proxies HTTP requests and WebSocket connections to a Vite dev server
//! for seamless frontend development with HMR support.

use axum::{
    body::Body,
    extract::{FromRequestParts, State, WebSocketUpgrade},
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

/// Cached Vite port - fetched once from host
static VITE_PORT: OnceCell<Option<u16>> = OnceCell::const_new();

/// Get the Vite port, fetching from host if not yet cached
async fn get_vite_port(ctx: &Arc<dyn RouterContext>) -> Option<u16> {
    *VITE_PORT
        .get_or_init(|| async {
            match ctx.host_client().get_vite_port().await {
                Ok(port) => port,
                Err(e) => {
                    tracing::warn!("Failed to get Vite port from host: {:?}", e);
                    None
                }
            }
        })
        .await
}

/// Check if a path should be proxied to Vite
pub fn is_vite_path(path: &str) -> bool {
    // Vite-specific paths
    path.starts_with("/@vite/")
        || path.starts_with("/@id/")
        || path.starts_with("/@fs/")
        || path.starts_with("/@react-refresh")
        || path.starts_with("/node_modules/.vite/")
        || path.starts_with("/node_modules/")
        || path.ends_with(".hot-update.json")
        || path.ends_with(".hot-update.js")
        // Common frontend extensions that Vite serves
        || path.ends_with(".ts")
        || path.ends_with(".tsx")
        || path.ends_with(".jsx")
        || path.ends_with(".vue")
        || path.ends_with(".svelte")
}

/// Check if request has a WebSocket upgrade
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
        Some(p) => p,
        None => {
            // Vite not running, fall through to 404
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

    tracing::debug!(method = %method, uri = %original_uri, "proxying to vite");

    // Check if this is a WebSocket upgrade request (for HMR)
    if is_websocket_upgrade(&req) {
        tracing::info!(uri = %original_uri, "detected websocket upgrade request for Vite HMR");

        // Split into parts so we can extract WebSocketUpgrade
        let (mut parts, _body) = req.into_parts();

        // Manually extract WebSocketUpgrade from request parts
        let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
            Ok(ws) => ws,
            Err(e) => {
                tracing::error!(error = %e, "failed to extract websocket upgrade");
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from(format!("WebSocket upgrade failed: {}", e)))
                    .unwrap();
            }
        };

        let target_uri = format!("ws://127.0.0.1:{}{}{}", vite_port, path, query);
        tracing::info!(target = %target_uri, "upgrading websocket to vite");

        return ws
            .protocols(["vite-hmr"])
            .on_upgrade(move |socket| async move {
                tracing::info!(path = %path, "websocket connection established, starting proxy");
                if let Err(e) = handle_vite_ws(socket, vite_port, &path, &query).await {
                    tracing::error!(error = %e, path = %path, "vite websocket proxy error");
                }
                tracing::info!(path = %path, "websocket connection closed");
            })
            .into_response();
    }

    // Regular HTTP proxy
    let target_uri = format!("http://127.0.0.1:{}{}{}", vite_port, path, query);

    let client = Client::builder(TokioExecutor::new()).build_http();

    let mut proxy_req_builder = Request::builder().method(req.method()).uri(&target_uri);

    // Copy headers (except Host)
    for (name, value) in req.headers() {
        if name != header::HOST {
            proxy_req_builder = proxy_req_builder.header(name, value);
        }
    }

    let proxy_req = proxy_req_builder.body(req.into_body()).unwrap();

    match client.request(proxy_req).await {
        Ok(res) => {
            let status = res.status();
            tracing::debug!(status = %status, path = %path, "vite response");

            let (parts, body) = res.into_parts();
            Response::from_parts(parts, Body::new(body))
        }
        Err(e) => {
            tracing::error!(error = %e, target = %target_uri, "vite proxy error");
            Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from(format!("Vite proxy error: {}", e)))
                .unwrap()
        }
    }
}

/// Handle WebSocket proxy to Vite for HMR
async fn handle_vite_ws(
    client_socket: axum::extract::ws::WebSocket,
    vite_port: u16,
    path: &str,
    query: &str,
) -> eyre::Result<()> {
    use axum::extract::ws::Message;
    use tokio_tungstenite::connect_async_with_config;
    use tokio_tungstenite::tungstenite::http::Request;

    let vite_url = format!("ws://127.0.0.1:{}{}{}", vite_port, path, query);

    tracing::info!(vite_url = %vite_url, "connecting to vite websocket");

    // Build request with vite-hmr subprotocol
    let request = Request::builder()
        .uri(&vite_url)
        .header("Sec-WebSocket-Protocol", "vite-hmr")
        .header("Host", format!("127.0.0.1:{}", vite_port))
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

    let (vite_ws, _response) = match connect_result {
        Ok(Ok((ws, resp))) => {
            tracing::info!(vite_url = %vite_url, "successfully connected to vite websocket");
            (ws, resp)
        }
        Ok(Err(e)) => {
            tracing::info!(vite_url = %vite_url, error = %e, "failed to connect to vite websocket");
            return Err(e.into());
        }
        Err(_) => {
            tracing::info!(vite_url = %vite_url, "timeout connecting to vite websocket");
            return Err(eyre::eyre!(
                "Timeout connecting to Vite WebSocket after {:?}",
                connect_timeout
            ));
        }
    };

    let (mut client_tx, mut client_rx) = client_socket.split();
    let (mut vite_tx, mut vite_rx) = vite_ws.split();

    // Bidirectional proxy
    let client_to_vite = async {
        while let Some(msg) = client_rx.next().await {
            match msg {
                Ok(Message::Text(text)) => {
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
                Ok(Message::Close(_)) => break,
                Err(_) => break,
                _ => {}
            }
        }
    };

    let vite_to_client = async {
        while let Some(msg) = vite_rx.next().await {
            match msg {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
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
                    let data_vec: Vec<u8> = data.to_vec();
                    if client_tx
                        .send(Message::Binary(data_vec.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => break,
                Err(_) => break,
                _ => {}
            }
        }
    };

    tokio::select! {
        _ = client_to_vite => {}
        _ = vite_to_client => {}
    }

    Ok(())
}
