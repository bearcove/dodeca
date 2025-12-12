//! Dodeca JXL plugin (dodeca-mod-jxl)
//!
//! This plugin handles JPEG XL encoding and decoding.

use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::Result;
use dodeca_plugin_runtime::{PluginTracing, add_tracing_service};
use jpegxl_rs::encode::EncoderFrame;
use rapace::RpcSession;
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace_plugin::{DispatcherBuilder, ServiceDispatch};

use mod_jxl_proto::{JXLProcessor, JXLResult, JXLEncodeInput, JXLProcessorServer};

/// Type alias for our transport (SHM-based for zero-copy)
type PluginTransport = ShmTransport;

/// JXL processor implementation
pub struct JXLProcessorImpl;

impl JXLProcessor for JXLProcessorImpl {
    async fn decode_jxl(&self, data: Vec<u8>) -> JXLResult {
        let decoder = match jpegxl_rs::decoder_builder().build() {
            Ok(d) => d,
            Err(e) => return JXLResult::Error {
                message: format!("Failed to create JXL decoder: {e}"),
            },
        };

        let (metadata, pixels) = match decoder.decode_with::<u8>(&data) {
            Ok(result) => result,
            Err(e) => return JXLResult::Error {
                message: format!("Failed to decode JXL: {e}"),
            },
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
            Err(e) => return JXLResult::Error {
                message: format!("Failed to create JXL encoder: {e}"),
            },
        };

        encoder.has_alpha = true;
        let frame = EncoderFrame::new(&input.pixels).num_channels(4);
        let result = match encoder.encode_frame::<_, u8>(&frame, input.width, input.height) {
            Ok(r) => r,
            Err(e) => return JXLResult::Error {
                message: format!("Failed to encode JXL: {e}"),
            },
        };

        JXLResult::EncodeSuccess { data: result.data.to_vec() }
    }
}

/// Service wrapper for JXLProcessor to satisfy ServiceDispatch
struct JXLProcessorService(Arc<JXLProcessorServer<JXLProcessorImpl>>);

impl ServiceDispatch for JXLProcessorService {
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
        let bytes = payload.to_vec();
        Box::pin(async move { server.dispatch(method_id, &bytes).await })
    }
}

/// SHM configuration - must match host's config
const SHM_CONFIG: ShmSessionConfig = ShmSessionConfig {
    ring_capacity: 256, // 256 descriptors in flight
    slot_size: 65536,   // 64KB per slot (fits most JXL data)
    slot_count: 128,    // 128 slots = 8MB total
};

/// CLI arguments
struct Args {
    /// SHM file path for zero-copy communication with host
    shm_path: PathBuf,
}

fn parse_args() -> Result<Args> {
    let mut shm_path = None;

    for arg in std::env::args().skip(1) {
        if let Some(value) = arg.strip_prefix("--shm-path=") {
            shm_path = Some(PathBuf::from(value));
        }
    }

    Ok(Args {
        shm_path: shm_path.ok_or_else(|| color_eyre::eyre::eyre!("--shm-path required"))?,
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let args = parse_args()?;

    // Wait for the host to create the SHM file
    for i in 0..50 {
        if args.shm_path.exists() {
            break;
        }
        if i == 49 {
            return Err(color_eyre::eyre::eyre!(
                "SHM file not created by host: {}",
                args.shm_path.display()
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Open the SHM session (plugin side)
    let shm_session = ShmSession::open_file(&args.shm_path, SHM_CONFIG)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to open SHM: {:?}", e))?;

    // Create SHM transport
    let transport: Arc<PluginTransport> = Arc::new(ShmTransport::new(shm_session));

    // Plugin uses even channel IDs (2, 4, 6, ...)
    // Host uses odd channel IDs (1, 3, 5, ...)
    let session = Arc::new(RpcSession::with_channel_start(transport, 2));

    // Initialize tracing to forward logs to host via RapaceTracingLayer
    let PluginTracing { tracing_config, .. } = dodeca_plugin_runtime::init_tracing(session.clone());

    tracing::info!("Connected to host via SHM");

    // Wrap services with rapace-plugin multi-service dispatcher
    let dispatcher = DispatcherBuilder::new();
    let dispatcher = add_tracing_service(dispatcher, tracing_config);
    let dispatcher = dispatcher.add_service(JXLProcessorService(Arc::new(JXLProcessorServer::new(
        JXLProcessorImpl,
    ))));
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    // Run the RPC session demux loop (this is the main event loop now)
    tracing::info!("JXL plugin ready, waiting for requests");
    if let Err(e) = session.run().await {
        tracing::error!(error = ?e, "RPC session error - host connection lost");
    }

    Ok(())
}