//! Dodeca JXL cell (cell-jxl)
//!
//! This cell handles JPEG XL encoding and decoding.

use jpegxl_rs::encode::EncoderFrame;

use cell_jxl_proto::{JXLEncodeInput, JXLProcessor, JXLProcessorServer, JXLResult};

/// JXL processor implementation
pub struct JXLProcessorImpl;

impl JXLProcessor for JXLProcessorImpl {
    async fn decode_jxl(&self, data: Vec<u8>) -> JXLResult {
        let decoder = match jpegxl_rs::decoder_builder().build() {
            Ok(d) => d,
            Err(e) => {
                return JXLResult::Error {
                    message: format!("Failed to create JXL decoder: {e}"),
                };
            }
        };

        let (metadata, pixels) = match decoder.decode_with::<u8>(&data) {
            Ok(result) => result,
            Err(e) => {
                return JXLResult::Error {
                    message: format!("Failed to decode JXL: {e}"),
                };
            }
        };

        JXLResult::DecodeSuccess {
            pixels,
            width: metadata.width,
            height: metadata.height,
            channels: metadata.num_color_channels as u8
                + if metadata.has_alpha_channel { 1 } else { 0 },
        }
    }

    async fn encode_jxl(&self, input: JXLEncodeInput) -> JXLResult {
        if input.pixels.len() != (input.width * input.height * 4) as usize {
            return JXLResult::Error {
                message: format!(
                    "Expected {} bytes for {}x{} RGBA, got {}",
                    input.width * input.height * 4,
                    input.width,
                    input.height,
                    input.pixels.len()
                ),
            };
        }

        // quality 0-100 maps to JXL distance (lower distance = better quality)
        // quality 100 -> distance ~0 (lossless territory)
        // quality 80 -> distance ~2 (high quality)
        // quality 0 -> distance ~15 (low quality)
        let distance = (100.0 - input.quality as f32) / 100.0 * 15.0;

        let mut encoder = match jpegxl_rs::encoder_builder()
            .quality(distance.max(0.1)) // quality() is actually distance in jpegxl-rs
            .build()
        {
            Ok(e) => e,
            Err(e) => {
                return JXLResult::Error {
                    message: format!("Failed to create JXL encoder: {e}"),
                };
            }
        };

        encoder.has_alpha = true;
        let frame = EncoderFrame::new(&input.pixels).num_channels(4);
        let result = match encoder.encode_frame::<_, u8>(&frame, input.width, input.height) {
            Ok(r) => r,
            Err(e) => {
                return JXLResult::Error {
                    message: format!("Failed to encode JXL: {e}"),
                };
            }
        };

        JXLResult::EncodeSuccess {
            data: result.data.to_vec(),
        }
    }
}

use std::sync::Arc;

struct CellService(Arc<JXLProcessorServer<JXLProcessorImpl>>);

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
    let server = JXLProcessorServer::new(JXLProcessorImpl);
    rapace_cell::run(CellService(Arc::new(server))).await?;
    Ok(())
}
