//! Dodeca Devtools - Sycamore-powered developer tools overlay
//!
//! Provides interactive debugging tools for dodeca sites:
//! - Live-reload connection indicator
//! - Template error display
//!
//! This crate is WASM-only.

#![cfg(target_arch = "wasm32")]

use sycamore::prelude::*;
use wasm_bindgen::prelude::*;

mod protocol;
mod state;

pub use protocol::{DevtoolsEvent, ErrorInfo, ScopeValue};
pub use state::DevtoolsState;

/// Mount the devtools overlay into the page
#[wasm_bindgen]
pub fn mount_devtools() {
    // Set up tracing for WASM
    tracing_wasm::set_as_global_default_with_config(
        tracing_wasm::WASMLayerConfigBuilder::new()
            .set_max_level(tracing::Level::DEBUG)
            .build(),
    );

    // Mount Sycamore app
    sycamore::render(|| {
        view! {
            DevtoolsApp {}
        }
    });
}

/// Main devtools application component
#[component]
fn DevtoolsApp() -> View {
    // Create state signal
    let state = create_signal(DevtoolsState::default());

    // Connect to WebSocket on mount
    create_effect(move || {
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = state::connect_websocket(state).await {
                tracing::error!("WebSocket connection failed: {e}");
            }
        });
    });

    view! {
        div(class="dodeca-devtools") {
            // Inline minimal styles
            style {
                r#"
                .dodeca-devtools {
                    font-family: 'SF Mono', Monaco, 'Cascadia Code', 'Roboto Mono', Consolas, monospace;
                    font-size: 12px;
                }
                .dodeca-indicator {
                    position: fixed;
                    bottom: 1rem;
                    right: 1rem;
                    z-index: 99999;
                    display: flex;
                    align-items: center;
                    gap: 0.5rem;
                    padding: 0.5rem 0.75rem;
                    background: #1a1a1a;
                    color: #e5e5e5;
                    border: 1px solid #333;
                    border-radius: 0.375rem;
                    cursor: pointer;
                    box-shadow: 0 2px 8px rgba(0,0,0,0.3);
                }
                .dodeca-indicator:hover {
                    background: #252525;
                }
                .dodeca-status {
                    width: 8px;
                    height: 8px;
                    border-radius: 50%;
                }
                .status-connected {
                    background: #22c55e;
                    box-shadow: 0 0 4px #22c55e;
                }
                .status-connecting {
                    background: #f59e0b;
                    box-shadow: 0 0 4px #f59e0b;
                }
                .status-disconnected {
                    background: #ef4444;
                    box-shadow: 0 0 4px #ef4444;
                }
                .dodeca-error-panel {
                    position: fixed;
                    bottom: 0;
                    left: 0;
                    right: 0;
                    max-height: 50vh;
                    z-index: 99998;
                    background: #1a1a1a;
                    border-top: 2px solid #ef4444;
                    overflow: auto;
                    padding: 1rem;
                    color: #e5e5e5;
                }
                .error-header {
                    display: flex;
                    justify-content: space-between;
                    align-items: center;
                    margin-bottom: 1rem;
                    padding-bottom: 0.5rem;
                    border-bottom: 1px solid #333;
                }
                .error-title {
                    color: #ef4444;
                    font-weight: 600;
                    font-size: 14px;
                }
                .error-close {
                    background: transparent;
                    border: 1px solid #444;
                    color: #999;
                    padding: 0.25rem 0.5rem;
                    border-radius: 0.25rem;
                    cursor: pointer;
                    font-size: 11px;
                }
                .error-close:hover {
                    background: #252525;
                    color: #e5e5e5;
                }
                .error-item {
                    margin-bottom: 1rem;
                    padding: 0.75rem;
                    background: #252525;
                    border-left: 3px solid #ef4444;
                    border-radius: 0.25rem;
                }
                .error-message {
                    color: #fca5a5;
                    margin-bottom: 0.5rem;
                    font-weight: 500;
                }
                .error-location {
                    color: #a3a3a3;
                    font-size: 11px;
                    margin-bottom: 0.5rem;
                }
                .error-source {
                    background: #0a0a0a;
                    padding: 0.5rem;
                    border-radius: 0.25rem;
                    overflow-x: auto;
                    font-size: 11px;
                }
                .error-source pre {
                    margin: 0;
                    color: #d4d4d4;
                }
                "#
            }

            // Connection indicator
            ConnectionIndicator(state=*state)

            // Error panel (if errors exist)
            (if state.with(|s| !s.errors.is_empty()) {
                view! { ErrorPanel(state=*state) }
            } else {
                view! {}
            })
        }
    }
}

#[derive(Props)]
struct ConnectionIndicatorProps {
    state: ReadSignal<DevtoolsState>,
}

#[component]
fn ConnectionIndicator(props: ConnectionIndicatorProps) -> View {
    let state = props.state;
    let connection_state = create_memo(move || state.with(|s| s.connection_state));
    let error_count = create_memo(move || state.with(|s| s.errors.len()));

    view! {
        div(class="dodeca-indicator") {
            // Status dot
            div(class=format!("dodeca-status status-{}",
                match connection_state.get() {
                    state::ConnectionState::Connected => "connected",
                    state::ConnectionState::Connecting => "connecting",
                    state::ConnectionState::Disconnected => "disconnected",
                }
            ))

            // Status text
            span {
                (match connection_state.get() {
                    state::ConnectionState::Connected if error_count.get() > 0 =>
                        format!("{} error{}", error_count.get(), if error_count.get() == 1 { "" } else { "s" }),
                    state::ConnectionState::Connected => "dodeca".to_string(),
                    state::ConnectionState::Connecting => "connecting...".to_string(),
                    state::ConnectionState::Disconnected => "disconnected".to_string(),
                })
            }
        }
    }
}

#[derive(Props)]
struct ErrorPanelProps {
    state: ReadSignal<DevtoolsState>,
}

#[component]
fn ErrorPanel(props: ErrorPanelProps) -> View {
    let state = props.state;
    let errors = create_memo(move || state.with(|s| s.errors.clone()));

    view! {
        div(class="dodeca-error-panel") {
            div(class="error-header") {
                div(class="error-title") {
                    "⚠ Template Errors"
                }
                button(class="error-close") {
                    "✕ Close"
                }
            }

            // Error list
            Keyed(
                list=errors,
                view=|error| view! {
                    div(class="error-item") {
                        div(class="error-message") {
                            (error.message.clone())
                        }
                        ({
                            let location = match (&error.template, error.line, error.column) {
                                (Some(template), Some(line), Some(col)) =>
                                    Some(format!("{}:{}:{}", template, line, col)),
                                (Some(template), Some(line), None) =>
                                    Some(format!("{}:{}", template, line)),
                                (Some(template), None, None) =>
                                    Some(template.clone()),
                                _ => None,
                            };
                            if let Some(loc) = location {
                                view! {
                                    div(class="error-location") {
                                        (format!("at {}", loc))
                                    }
                                }
                            } else {
                                view! {}
                            }
                        })
                        (if let Some(snippet) = &error.source_snippet {
                            let source_text = snippet.lines.iter()
                                .map(|line| line.content.as_str())
                                .collect::<Vec<_>>()
                                .join("\n");
                            view! {
                                div(class="error-source") {
                                    pre { (source_text) }
                                }
                            }
                        } else {
                            view! {}
                        })
                    }
                },
                key=|error| error.message.clone(),
            )
        }
    }
}
