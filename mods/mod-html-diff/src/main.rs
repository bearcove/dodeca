//! Dodeca HTML diff plugin (dodeca-mod-html-diff)
//!
//! This plugin handles HTML DOM diffing for live reload.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::Result;
use dodeca_plugin_runtime::{PluginTracing, add_tracing_service};
use rapace::RpcSession;
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace_plugin::{DispatcherBuilder, ServiceDispatch};

use mod_html_diff_proto::{HtmlDiffer, HtmlDiffResult, DiffInput, DiffResult, HtmlDifferServer};

// Re-export protocol types
pub use dodeca_protocol::{NodePath, Patch};

/// Type alias for our transport (SHM-based for zero-copy)
type PluginTransport = ShmTransport;

/// HTML differ implementation - ported from original
pub struct HtmlDifferImpl;

impl HtmlDiffer for HtmlDifferImpl {
    async fn diff_html(&self, input: DiffInput) -> HtmlDiffResult {
        let old_dom = match parse_html(&input.old_html) {
            Some(dom) => dom,
            None => return HtmlDiffResult::Error {
                message: "Failed to parse old HTML".to_string(),
            },
        };

        let new_dom = match parse_html(&input.new_html) {
            Some(dom) => dom,
            None => return HtmlDiffResult::Error {
                message: "Failed to parse new HTML".to_string(),
            },
        };

        let result = diff(&old_dom, &new_dom);

        HtmlDiffResult::Success {
            result: DiffResult {
                patches: result.patches,
                nodes_compared: result.nodes_compared,
                nodes_skipped: result.nodes_skipped,
            },
        }
    }
}

// ============================================================================
// Internal DOM representation (copied from original)
// ============================================================================

/// A node in our simplified DOM tree
#[derive(Debug, Clone, PartialEq, Eq)]
struct DomNode {
    /// Element tag name (e.g., "div", "p") or "#text" for text nodes
    tag: String,
    /// Attributes (empty for text nodes)
    attrs: HashMap<String, String>,
    /// Text content (for text nodes) or empty
    text: String,
    /// Child nodes
    children: Vec<DomNode>,
    /// Precomputed hash of this subtree (for fast comparison)
    subtree_hash: u64,
}

/// Internal result of diffing two DOM trees
struct InternalDiffResult {
    patches: Vec<Patch>,
    nodes_compared: usize,
    nodes_skipped: usize,
}

impl DomNode {
    fn element(tag: &str, attrs: HashMap<String, String>, children: Vec<DomNode>) -> Self {
        let mut node = Self {
            tag: tag.to_string(),
            attrs,
            text: String::new(),
            children,
            subtree_hash: 0,
        };
        node.subtree_hash = node.compute_hash();
        node
    }

    fn text(text: &str) -> Self {
        let mut node = Self {
            tag: "#text".to_string(),
            attrs: HashMap::new(),
            text: text.to_string(),
            children: Vec::new(),
            subtree_hash: 0,
        };
        node.subtree_hash = node.compute_hash();
        node
    }

    fn compute_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        
        // Hash tag and text
        self.tag.hash(&mut hasher);
        self.text.hash(&mut hasher);
        
        // Hash attributes in sorted order
        let mut attr_keys: Vec<_> = self.attrs.keys().collect();
        attr_keys.sort();
        for key in attr_keys {
            key.hash(&mut hasher);
            self.attrs[key].hash(&mut hasher);
        }
        
        // Hash children
        for child in &self.children {
            child.subtree_hash.hash(&mut hasher);
        }
        
        hasher.finish()
    }
}

fn parse_html(html: &str) -> Option<DomNode> {
    // This is a simplified HTML parser - the full implementation would be more complex
    // For now, just return a basic structure
    Some(DomNode::element("html", HashMap::new(), vec![
        DomNode::text(html)
    ]))
}

fn diff(old_dom: &DomNode, new_dom: &DomNode) -> InternalDiffResult {
    let mut result = InternalDiffResult {
        patches: Vec::new(),
        nodes_compared: 0,
        nodes_skipped: 0,
    };

    // Simple diff implementation - the full one would be much more complex
    if old_dom.subtree_hash == new_dom.subtree_hash {
        result.nodes_skipped += 1;
        return result;
    }

    result.nodes_compared += 1;
    
    // For now, just replace the entire content if hashes don't match
    result.patches.push(Patch::Replace {
        path: NodePath(vec![]),
        html: format!("<{}>{}</{}>", new_dom.tag, new_dom.text, new_dom.tag),
    });

    result
}

/// Service wrapper for HtmlDiffer to satisfy ServiceDispatch
struct HtmlDifferService(Arc<HtmlDifferServer<HtmlDifferImpl>>);

impl ServiceDispatch for HtmlDifferService {
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
    slot_size: 65536,   // 64KB per slot (fits most HTML files)
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
    let dispatcher = dispatcher.add_service(HtmlDifferService(Arc::new(HtmlDifferServer::new(
        HtmlDifferImpl,
    ))));
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    // Run the RPC session demux loop (this is the main event loop now)
    tracing::info!("HTML diff plugin ready, waiting for requests");
    if let Err(e) = session.run().await {
        tracing::error!(error = ?e, "RPC session error - host connection lost");
    }

    Ok(())
}