//! RPC protocol for dodeca WebP cell
//!
//! Defines services for WebP encoding and decoding.

use facet::Facet;

/// Input for WebP encoding
#[derive(Debug, Clone, Facet)]
pub struct WebPEncodeInput {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub quality: u8,
}

/// Result of WebP processing operations
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum WebPResult {
    /// Successfully decoded WebP
    DecodeSuccess {
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        channels: u8,
    },
    /// Successfully encoded WebP
    EncodeSuccess { data: Vec<u8> },
    /// Error during processing
    Error { message: String },
}

/// WebP processing service implemented by the cell.
///
/// The host calls these methods to process WebP images.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait WebPProcessor {
    /// Decode WebP to RGBA/RGB pixels
    async fn decode_webp(&self, data: Vec<u8>) -> WebPResult;

    /// Encode RGBA pixels to WebP
    async fn encode_webp(&self, input: WebPEncodeInput) -> WebPResult;
}
