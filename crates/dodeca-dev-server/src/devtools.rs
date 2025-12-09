//! Devtools WebSocket handler
//!
//! Handles the /_/ws endpoint for live reload and devtools communication.
//! Asset serving (JS, WASM, snippets) is handled by the host via ContentService.

use std::sync::Arc;

use axum::extract::{State, WebSocketUpgrade, ws::{Message, WebSocket}};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};

use dodeca_protocol::{ClientMessage, ServerMessage, facet_postcard};

use crate::PluginContext;

/// Live reload message types (plugin-internal broadcast)
#[derive(Clone, Debug)]
pub enum LiveReloadMsg {
    /// Full page reload
    Reload,
    /// DOM patches for a route
    Patches { route: String, patches: Vec<dodeca_protocol::Patch> },
    /// CSS update (new cache-busted path)
    CssUpdate { path: String },
    /// Template error
    Error {
        route: String,
        message: String,
        template: Option<String>,
        line: Option<u32>,
        snapshot_id: String,
    },
    /// Error resolved
    ErrorResolved { route: String },
}

/// WebSocket handler for devtools
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(ctx): State<Arc<PluginContext>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, ctx))
}

async fn handle_socket(socket: WebSocket, ctx: Arc<PluginContext>) {
    let (mut sender, mut receiver) = socket.split();
    let mut reload_rx = ctx.livereload_tx.subscribe();

    let mut current_route: Option<String> = None;

    tracing::info!("Browser connected for live reload");

    loop {
        tokio::select! {
            // Handle messages from host (broadcast)
            result = reload_rx.recv() => {
                match result {
                    Ok(LiveReloadMsg::Reload) => {
                        let msg = ServerMessage::Reload;
                        if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                            if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(LiveReloadMsg::Patches { route, patches }) => {
                        if current_route.as_ref().is_none_or(|r| r == &route) {
                            let msg = ServerMessage::Patches(patches);
                            if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                                if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(LiveReloadMsg::CssUpdate { path }) => {
                        let msg = ServerMessage::CssChanged { path };
                        if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                            if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(LiveReloadMsg::Error { route, message, template, line, snapshot_id }) => {
                        let msg = ServerMessage::Error(dodeca_protocol::ErrorInfo {
                            route: route.clone(),
                            message,
                            template,
                            line,
                            column: None,
                            source_snippet: None,
                            snapshot_id,
                            available_variables: vec![],
                        });
                        if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                            if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(LiveReloadMsg::ErrorResolved { route }) => {
                        let msg = ServerMessage::ErrorResolved { route };
                        if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                            if sender.send(Message::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }

            // Handle messages from browser
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        if let Ok(client_msg) = facet_postcard::from_bytes::<ClientMessage>(&data) {
                            match client_msg {
                                ClientMessage::Route { path } => {
                                    current_route = Some(path);
                                }
                                ClientMessage::GetScope { request_id, snapshot_id: _, path } => {
                                    // Call host's get_scope via RPC
                                    let route = current_route.clone().unwrap_or_default();
                                    let scope = ctx.content_client.get_scope(route, path.unwrap_or_default()).await
                                        .unwrap_or_default();
                                    let msg = ServerMessage::ScopeResponse { request_id, scope };
                                    if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                                        let _ = sender.send(Message::Binary(bytes.into())).await;
                                    }
                                }
                                ClientMessage::Eval { request_id, snapshot_id: _, expression } => {
                                    // Call host's eval_expression via RPC
                                    let route = current_route.clone().unwrap_or_default();
                                    let result = ctx.content_client.eval_expression(route, expression).await
                                        .unwrap_or_else(|e| dodeca_protocol::EvalResult::Err(format!("{:?}", e)));
                                    let msg = ServerMessage::EvalResponse { request_id, result };
                                    if let Ok(bytes) = facet_postcard::to_vec(&msg) {
                                        let _ = sender.send(Message::Binary(bytes.into())).await;
                                    }
                                }
                                ClientMessage::DismissError { route: _ } => {
                                    // No-op for now
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    tracing::info!("Browser disconnected");
}
