//! RPC protocol for dodeca JXL cell
//!
//! Defines services for JPEG XL encoding and decoding.

use facet::Facet;

/// Input for JXL encoding
#[derive(Debug, Clone, Facet)]
pub struct JXLEncodeInput {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub quality: u8,
}

/// Result of JXL processing operations
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum JXLResult {
    /// Successfully decoded JXL
    DecodeSuccess {
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        channels: u8,
    },
    /// Successfully encoded JXL
    EncodeSuccess { data: Vec<u8> },
    /// Error during processing
    Error { message: String },
}

/// JXL processing service implemented by the cell.
///
/// The host calls these methods to process JPEG XL images.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait JXLProcessor {
    /// Decode JPEG XL to RGBA pixels
    async fn decode_jxl(&self, data: Vec<u8>) -> JXLResult;

    /// Encode RGBA pixels to JPEG XL
    async fn encode_jxl(&self, input: JXLEncodeInput) -> JXLResult;
}
