//! Dodeca DevTools browser runtime.
//!
//! The visible DevTools UI lives in the TypeScript bundle. This WASM crate
//! keeps the small Rust/browser pieces: live DOM patching, CSS reload,
//! route subscriptions, dead-link actions, and open-in-editor actions.
//!
//! This crate is WASM-only.

#![cfg(target_arch = "wasm32")]

use wasm_bindgen::prelude::*;

mod dead_links;
mod open_in_editor;
mod protocol;
mod state;

pub use protocol::{DevtoolsEvent, ErrorInfo, ScopeValue};

/// Mount the DevTools runtime into the page.
#[wasm_bindgen]
pub fn mount_devtools() {
    tracing_wasm::set_as_global_default_with_config(
        tracing_wasm::WASMLayerConfigBuilder::new()
            .set_max_level(tracing::Level::DEBUG)
            .build(),
    );

    wasm_bindgen_futures::spawn_local(async {
        if let Err(e) = state::connect_websocket().await {
            tracing::error!("WebSocket connection failed: {e}");
        }
    });

    dead_links::install();
    open_in_editor::install();
}
