//! WASM client for dodeca live reload

use wasm_bindgen::JsValue;

// Re-export everything from hotmeal-wasm
pub use hotmeal_wasm::*;

/// Apply patches from a postcard-serialized blob.
/// This is the API expected by dodeca-devtools.
pub fn apply_patches_blob(patches_blob: &[u8]) -> Result<usize, JsValue> {
    let patches: Vec<hotmeal::Patch<'static>> =
        dodeca_protocol::facet_postcard::from_slice(patches_blob)
            .map_err(|e| JsValue::from_str(&format!("Failed to deserialize patches: {e}")))?;

    hotmeal_wasm::apply_patches(&patches)
}
