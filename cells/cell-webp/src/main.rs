//! Dodeca WebP cell (cell-webp)
//!
//! This cell handles WebP encoding and decoding.

use cell_webp_proto::{WebPEncodeInput, WebPProcessor, WebPProcessorDispatcher, WebPResult};
use dodeca_cell_runtime::run_cell;

/// WebP processor implementation
#[derive(Clone)]
pub struct WebPProcessorImpl;

impl WebPProcessor for WebPProcessorImpl {
    async fn decode_webp(&self, data: Vec<u8>) -> WebPResult {
        let decoder = webp::Decoder::new(&data);
        let image = match decoder.decode() {
            Some(img) => img,
            None => {
                return WebPResult::Error {
                    message: "Failed to decode WebP".to_string(),
                };
            }
        };

        WebPResult::DecodeSuccess {
            pixels: (*image).to_vec(),
            width: image.width(),
            height: image.height(),
            channels: if image.is_alpha() { 4 } else { 3 },
        }
    }

    async fn encode_webp(&self, input: WebPEncodeInput) -> WebPResult {
        if input.pixels.len() != (input.width * input.height * 4) as usize {
            return WebPResult::Error {
                message: format!(
                    "Expected {} bytes for {}x{} RGBA, got {}",
                    input.width * input.height * 4,
                    input.width,
                    input.height,
                    input.pixels.len()
                ),
            };
        }

        let encoder = webp::Encoder::from_rgba(&input.pixels, input.width, input.height);
        let webp = encoder.encode(input.quality as f32);

        WebPResult::EncodeSuccess {
            data: webp.to_vec(),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("webp", WebPProcessorDispatcher::new(WebPProcessorImpl))
}
