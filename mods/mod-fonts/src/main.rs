//! Dodeca fonts plugin (dodeca-mod-fonts)
//!
//! This plugin handles font analysis, subsetting, and compression.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::Result;
use dodeca_plugin_runtime::{PluginTracing, add_tracing_service};
use rapace::RpcSession;
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace_plugin::{DispatcherBuilder, ServiceDispatch};

use mod_fonts_proto::{FontProcessor, FontResult, FontAnalysis, FontFace, SubsetFontInput, FontProcessorServer};

/// Type alias for our transport (SHM-based for zero-copy)
type PluginTransport = ShmTransport;

/// Font processor implementation
pub struct FontProcessorImpl;

impl FontProcessor for FontProcessorImpl {
    async fn analyze_fonts(&self, html: String, css: String) -> FontResult {
        let analysis = fontcull::analyze_fonts(&html, &css);

        // Convert HashSet<char> to sorted Vec<char> for deterministic serialization
        let chars_per_font: HashMap<String, Vec<char>> = analysis
            .chars_per_font
            .into_iter()
            .map(|(family, chars)| {
                let mut chars_vec: Vec<char> = chars.into_iter().collect();
                chars_vec.sort();
                (family, chars_vec)
            })
            .collect();

        let font_faces: Vec<FontFace> = analysis
            .font_faces
            .into_iter()
            .map(|f| FontFace {
                family: f.family,
                src: f.src,
                weight: f.weight,
                style: f.style,
            })
            .collect();

        FontResult::AnalysisSuccess {
            analysis: FontAnalysis {
                chars_per_font,
                font_faces,
            },
        }
    }

    async fn extract_css_from_html(&self, html: String) -> FontResult {
        FontResult::CssSuccess {
            css: fontcull::extract_css_from_html(&html),
        }
    }

    async fn decompress_font(&self, data: Vec<u8>) -> FontResult {
        match fontcull::decompress_font(&data) {
            Ok(decompressed) => FontResult::DecompressSuccess { data: decompressed },
            Err(e) => FontResult::Error {
                message: format!("Failed to decompress font: {e}"),
            },
        }
    }

    async fn subset_font(&self, input: SubsetFontInput) -> FontResult {
        let char_set: HashSet<char> = input.chars.into_iter().collect();

        match fontcull::subset_font_data(&input.data, &char_set) {
            Ok(subsetted) => FontResult::SubsetSuccess { data: subsetted },
            Err(e) => FontResult::Error {
                message: format!("Failed to subset font: {e}"),
            },
        }
    }

    async fn compress_to_woff2(&self, data: Vec<u8>) -> FontResult {
        match fontcull::compress_to_woff2(&data) {
            Ok(woff2) => FontResult::CompressSuccess { data: woff2 },
            Err(e) => FontResult::Error {
                message: format!("Failed to compress to WOFF2: {e}"),
            },
        }
    }
}

/// Service wrapper for FontProcessor to satisfy ServiceDispatch
struct FontProcessorService(Arc<FontProcessorServer<FontProcessorImpl>>);

impl ServiceDispatch for FontProcessorService {
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
    slot_size: 65536,   // 64KB per slot (fits most font data)
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
    let dispatcher = dispatcher.add_service(FontProcessorService(Arc::new(FontProcessorServer::new(
        FontProcessorImpl,
    ))));
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    // Run the RPC session demux loop (this is the main event loop now)
    tracing::info!("Fonts plugin ready, waiting for requests");
    if let Err(e) = session.run().await {
        tracing::error!(error = ?e, "RPC session error - host connection lost");
    }

    Ok(())
}