//! Dodeca SASS plugin (dodeca-mod-sass)
//!
//! This plugin handles SASS/SCSS compilation using grass.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use color_eyre::Result;
use dodeca_plugin_runtime::{PluginTracing, add_tracing_service};
use rapace::RpcSession;
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace_plugin::{DispatcherBuilder, ServiceDispatch};

use mod_sass_proto::{SassCompiler, SassResult, SassInput, SassCompilerServer};

/// Type alias for our transport (SHM-based for zero-copy)
type PluginTransport = ShmTransport;

/// SASS compiler implementation
pub struct SassCompilerImpl;

impl SassCompiler for SassCompilerImpl {
    async fn compile_sass(&self, input: SassInput) -> SassResult {
        let files = input.files;

        // Find main.scss
        let main_content = match files.get("main.scss") {
            Some(content) => content,
            None => return SassResult::Error {
                message: "main.scss not found in files".to_string(),
            },
        };

        // Create an in-memory filesystem for grass
        let fs = InMemorySassFs::new(&files);

        // Compile with grass using in-memory fs
        let options = grass::Options::default().fs(&fs);

        match grass::from_string(main_content.clone(), &options) {
            Ok(css) => SassResult::Success { css },
            Err(e) => SassResult::Error {
                message: format!("SASS compilation failed: {}", e),
            },
        }
    }
}

/// In-memory filesystem for grass SASS compiler
#[derive(Debug)]
struct InMemorySassFs {
    files: HashMap<PathBuf, Vec<u8>>,
}

impl InMemorySassFs {
    fn new(sass_map: &HashMap<String, String>) -> Self {
        let files = sass_map
            .iter()
            .map(|(path, content)| (PathBuf::from(path), content.as_bytes().to_vec()))
            .collect();
        Self { files }
    }
}

impl grass::Fs for InMemorySassFs {
    fn is_dir(&self, path: &Path) -> bool {
        // Check if any file is under this directory
        self.files.keys().any(|f| f.starts_with(path))
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        self.files.get(path).cloned().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found: {path:?}"),
            )
        })
    }
}

/// Service wrapper for SassCompiler to satisfy ServiceDispatch
struct SassCompilerService(Arc<SassCompilerServer<SassCompilerImpl>>);

impl ServiceDispatch for SassCompilerService {
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
    slot_size: 65536,   // 64KB per slot (fits most SASS files)
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
    let dispatcher = dispatcher.add_service(SassCompilerService(Arc::new(SassCompilerServer::new(
        SassCompilerImpl,
    ))));
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    // Run the RPC session demux loop (this is the main event loop now)
    tracing::info!("SASS plugin ready, waiting for requests");
    if let Err(e) = session.run().await {
        tracing::error!(error = ?e, "RPC session error - host connection lost");
    }

    Ok(())
}