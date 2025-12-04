//! Dodeca Devtools - Dioxus-powered developer tools overlay
//!
//! Provides interactive debugging tools for dodeca sites:
//! - Template error overlay with source context
//! - Scope explorer for inspecting template variables
//! - Expression REPL for evaluating template expressions

use dioxus::prelude::*;
use wasm_bindgen::prelude::*;

mod protocol;
mod state;
mod components;

pub use protocol::{ClientMessage, ErrorInfo, ServerMessage, ScopeValue};
pub use state::DevtoolsState;

/// Mount the devtools overlay into the page
#[wasm_bindgen]
pub fn mount_devtools() {
    // Set up tracing for WASM (INFO level to avoid dioxus TRACE spam)
    tracing_wasm::set_as_global_default_with_config(
        tracing_wasm::WASMLayerConfigBuilder::new()
            .set_max_level(tracing::Level::INFO)
            .build()
    );

    // Create a container element for the devtools
    let window = web_sys::window().expect("no window");
    let document = window.document().expect("no document");

    // Create shadow host for isolation
    let host = document.create_element("div").expect("create div");
    host.set_id("dodeca-devtools");

    // Append to body
    document.body().expect("no body").append_child(&host).expect("append");

    // Launch Dioxus app in the shadow DOM
    dioxus_web::launch::launch_cfg(
        App,
        dioxus_web::Config::new().rootname("dodeca-devtools"),
    );
}

/// Main devtools application component
#[component]
fn App() -> Element {
    // Global state for devtools
    let state = use_context_provider(|| Signal::new(DevtoolsState::default()));

    // Connect to WebSocket on mount
    use_effect(move || {
        spawn(async move {
            if let Err(e) = state::connect_websocket(state).await {
                tracing::error!("WebSocket connection failed: {e}");
            }
        });
    });

    rsx! {
        components::DevtoolsOverlay {}
    }
}
