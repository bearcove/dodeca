//! Dodeca image plugin (dodeca-mod-image)
//!
//! This plugin handles image decoding, resizing, and thumbhash generation.

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use color_eyre::Result;
use dodeca_plugin_runtime::{PluginTracing, add_tracing_service};
use image::{DynamicImage, ImageEncoder, Rgb, Rgba};
use rapace::RpcSession;
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace_plugin::{DispatcherBuilder, ServiceDispatch};

use mod_image_proto::{DecodedImage, ImageProcessor, ImageResult, ImageProcessorServer, ResizeInput, ThumbhashInput};

/// Type alias for our transport (SHM-based for zero-copy)
type PluginTransport = ShmTransport;

/// Image processor implementation
pub struct ImageProcessorImpl;

impl ImageProcessor for ImageProcessorImpl {
    async fn decode_png(&self, data: Vec<u8>) -> ImageResult {
        decode_format(&data, image::ImageFormat::Png)
    }

    async fn decode_jpeg(&self, data: Vec<u8>) -> ImageResult {
        decode_format(&data, image::ImageFormat::Jpeg)
    }

    async fn decode_gif(&self, data: Vec<u8>) -> ImageResult {
        decode_format(&data, image::ImageFormat::Gif)
    }

    async fn resize_image(&self, input: ResizeInput) -> ImageResult {
        let img = match pixels_to_dynamic_image(&input.pixels, input.width, input.height, input.channels) {
            Some(img) => img,
            None => return ImageResult::Error {
                message: "Invalid pixel data".to_string(),
            },
        };

        // Maintain aspect ratio
        let aspect = input.height as f64 / input.width as f64;
        let target_height = (input.target_width as f64 * aspect).round() as u32;

        let resized = img.resize_exact(
            input.target_width,
            target_height,
            image::imageops::FilterType::Lanczos3,
        );

        let rgba = resized.to_rgba8();
        ImageResult::Success {
            image: DecodedImage {
                width: rgba.width(),
                height: rgba.height(),
                pixels: rgba.into_raw(),
                channels: 4,
            },
        }
    }

    async fn generate_thumbhash_data_url(&self, input: ThumbhashInput) -> ImageResult {
        let img = match pixels_to_dynamic_image(&input.pixels, input.width, input.height, 4) {
            Some(img) => img,
            None => return ImageResult::Error {
                message: "Invalid pixel data".to_string(),
            },
        };

        // Thumbhash works best with small images, resize if needed
        let thumb_img = if input.width > 100 || input.height > 100 {
            img.resize(100, 100, image::imageops::FilterType::Triangle)
        } else {
            img
        };

        let rgba = thumb_img.to_rgba8();
        let hash = thumbhash::rgba_to_thumb_hash(
            thumb_img.width() as usize,
            thumb_img.height() as usize,
            rgba.as_raw(),
        );

        // Decode thumbhash back to RGBA for the placeholder image
        let (w, h, rgba_pixels) = match thumbhash::thumb_hash_to_rgba(&hash) {
            Ok(result) => result,
            Err(()) => return ImageResult::Error {
                message: "Failed to decode thumbhash".to_string(),
            },
        };

        // Create a tiny PNG from the decoded thumbhash
        let img_buf: image::RgbaImage =
            match image::ImageBuffer::from_raw(w as u32, h as u32, rgba_pixels) {
                Some(buf) => buf,
                None => return ImageResult::Error {
                    message: "Failed to create image buffer".to_string(),
                },
            };

        let mut png_bytes = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
        if let Err(e) = encoder.write_image(
            img_buf.as_raw(),
            img_buf.width(),
            img_buf.height(),
            image::ExtendedColorType::Rgba8,
        ) {
            return ImageResult::Error {
                message: format!("Failed to encode PNG: {e}"),
            };
        }

        // Encode as data URL
        let base64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
        ImageResult::ThumbhashSuccess {
            data_url: format!("data:image/png;base64,{}", base64),
        }
    }
}

fn decode_format(data: &[u8], format: image::ImageFormat) -> ImageResult {
    let img = match image::load_from_memory_with_format(data, format) {
        Ok(img) => img,
        Err(e) => return ImageResult::Error {
            message: format!("Failed to decode image: {e}"),
        },
    };

    let rgba = img.to_rgba8();
    ImageResult::Success {
        image: DecodedImage {
            width: rgba.width(),
            height: rgba.height(),
            pixels: rgba.into_raw(),
            channels: 4,
        },
    }
}

/// Convert raw pixels to DynamicImage
fn pixels_to_dynamic_image(
    pixels: &[u8],
    width: u32,
    height: u32,
    channels: u8,
) -> Option<DynamicImage> {
    match channels {
        3 => {
            let img_buf =
                image::ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width, height, pixels.to_vec())?;
            Some(DynamicImage::from(img_buf))
        }
        4 => {
            let img_buf =
                image::ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, pixels.to_vec())?;
            Some(DynamicImage::from(img_buf))
        }
        _ => None,
    }
}

/// Service wrapper for ImageProcessor to satisfy ServiceDispatch
struct ImageProcessorService(Arc<ImageProcessorServer<ImageProcessorImpl>>);

impl ServiceDispatch for ImageProcessorService {
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
    slot_size: 65536,   // 64KB per slot (fits most image data)
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
    let dispatcher = dispatcher.add_service(ImageProcessorService(Arc::new(ImageProcessorServer::new(
        ImageProcessorImpl,
    ))));
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    // Run the RPC session demux loop (this is the main event loop now)
    tracing::info!("Image plugin ready, waiting for requests");
    if let Err(e) = session.run().await {
        tracing::error!(error = ?e, "RPC session error - host connection lost");
    }

    Ok(())
}