//! Dodeca WebP cell (cell-webp)
//!
//! This cell handles WebP encoding and decoding.

use cell_webp_proto::{WebPEncodeInput, WebPProcessor, WebPProcessorServer, WebPResult};

/// WebP processor implementation
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

use std::sync::Arc;

struct CellService(Arc<WebPProcessorServer<WebPProcessorImpl>>);

impl rapace_cell::ServiceDispatch for CellService {
    fn dispatch(
        &self,
        method_id: u32,
        payload: &[u8],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<rapace::Frame, rapace::RpcError>>
                + Send
                + 'static,
        >,
    > {
        let server = self.0.clone();
        let payload = payload.to_vec();
        Box::pin(async move { server.dispatch(method_id, &payload).await })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = WebPProcessorServer::new(WebPProcessorImpl);
    rapace_cell::run(CellService(Arc::new(server))).await?;
    Ok(())
}
