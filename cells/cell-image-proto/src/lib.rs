//! RPC protocol for dodeca image cell
//!
//! Defines services for image decoding, resizing, and thumbhash generation.

use facet::Facet;

/// Decoded image data
#[derive(Debug, Clone, Facet)]
pub struct DecodedImage {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub channels: u8,
}

/// Result of image processing operations
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum ImageResult {
    /// Successfully processed image
    Success { image: DecodedImage },
    /// Successfully generated thumbhash data URL
    ThumbhashSuccess { data_url: String },
    /// Error during processing
    Error { message: String },
}

/// Input for resize operation
#[derive(Debug, Clone, Facet)]
pub struct ResizeInput {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub target_width: u32,
}

/// Input for thumbhash generation
#[derive(Debug, Clone, Facet)]
pub struct ThumbhashInput {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Image processing service implemented by the cell.
///
/// The host calls these methods to process image content.
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait ImageProcessor {
    /// Decode a PNG image to RGBA pixels
    async fn decode_png(&self, data: Vec<u8>) -> ImageResult;

    /// Decode a JPEG image to RGBA pixels
    async fn decode_jpeg(&self, data: Vec<u8>) -> ImageResult;

    /// Decode a GIF image to RGBA pixels (first frame only)
    async fn decode_gif(&self, data: Vec<u8>) -> ImageResult;

    /// Resize an image maintaining aspect ratio using Lanczos3 filter
    async fn resize_image(&self, input: ResizeInput) -> ImageResult;

    /// Generate a thumbhash data URL from RGBA pixels
    async fn generate_thumbhash_data_url(&self, input: ThumbhashInput) -> ImageResult;
}
