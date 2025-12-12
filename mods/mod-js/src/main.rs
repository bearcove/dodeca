//! Dodeca JS plugin (dodeca-mod-js)
//!
//! This plugin handles JavaScript string literal rewriting using OXC.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::Result;
use dodeca_plugin_runtime::{PluginTracing, add_tracing_service};
use oxc::allocator::Allocator;
use oxc::ast::ast::{StringLiteral, TemplateLiteral};
use oxc::ast_visit::Visit;
use oxc::parser::Parser;
use oxc::span::SourceType;
use rapace::RpcSession;
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace_plugin::{DispatcherBuilder, ServiceDispatch};

use mod_js_proto::{JsProcessor, JsResult, JsRewriteInput, JsProcessorServer};

/// Type alias for our transport (SHM-based for zero-copy)
type PluginTransport = ShmTransport;

/// JS processor implementation
pub struct JsProcessorImpl;

impl JsProcessor for JsProcessorImpl {
    async fn rewrite_string_literals(&self, input: JsRewriteInput) -> JsResult {
        let js = &input.js;
        let path_map = &input.path_map;

        // Parse the JavaScript
        let allocator = Allocator::default();
        let source_type = SourceType::mjs(); // Treat as ES module
        let parser_result = Parser::new(&allocator, js, source_type).parse();

        if parser_result.panicked || !parser_result.errors.is_empty() {
            // If parsing fails, return unchanged (could be a snippet or invalid JS)
            return JsResult::Success { js: js.to_string() };
        }

        // Collect string literal positions and their replacement values
        let mut replacements: Vec<(u32, u32, String)> = Vec::new(); // (start, end, new_value)
        let mut collector = StringCollector {
            source: js,
            path_map,
            replacements: &mut replacements,
        };
        collector.visit_program(&parser_result.program);

        // Apply replacements in reverse order (so offsets stay valid)
        if replacements.is_empty() {
            return JsResult::Success { js: js.to_string() };
        }

        replacements.sort_by(|a, b| b.0.cmp(&a.0)); // Sort by start position, descending

        let mut result = js.to_string();
        for (start, end, new_value) in replacements {
            result.replace_range(start as usize..end as usize, &new_value);
        }

        JsResult::Success { js: result }
    }
}

/// Visitor that collects string literals for replacement
struct StringCollector<'a> {
    source: &'a str,
    path_map: &'a HashMap<String, String>,
    replacements: &'a mut Vec<(u32, u32, String)>,
}

impl<'a> Visit<'_> for StringCollector<'a> {
    fn visit_string_literal(&mut self, lit: &StringLiteral<'_>) {
        let value = lit.value.as_str();
        let mut new_value = value.to_string();
        let mut changed = false;

        for (old_path, new_path) in self.path_map.iter() {
            if new_value.contains(old_path.as_str()) {
                new_value = new_value.replace(old_path, new_path);
                changed = true;
            }
        }

        if changed {
            // Get the original source including quotes
            let start = lit.span.start;
            let end = lit.span.end;
            let original = &self.source[start as usize..end as usize];
            let quote = original.chars().next().unwrap_or('"');
            self.replacements
                .push((start, end, format!("{quote}{new_value}{quote}")));
        }
    }

    fn visit_template_literal(&mut self, lit: &TemplateLiteral<'_>) {
        // Handle template literal quasi strings
        for quasi in &lit.quasis {
            let value = quasi.value.raw.as_str();
            let mut new_value = value.to_string();
            let mut changed = false;

            for (old_path, new_path) in self.path_map.iter() {
                if new_value.contains(old_path.as_str()) {
                    new_value = new_value.replace(old_path, new_path);
                    changed = true;
                }
            }

            if changed {
                // For template literals, we only replace the quasi part
                let start = quasi.span.start;
                let end = quasi.span.end;
                self.replacements.push((start, end, new_value));
            }
        }

        // Continue visiting expressions inside template literal
        for expr in &lit.expressions {
            self.visit_expression(expr);
        }
    }
}

/// Service wrapper for JsProcessor to satisfy ServiceDispatch
struct JsProcessorService(Arc<JsProcessorServer<JsProcessorImpl>>);

impl ServiceDispatch for JsProcessorService {
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
    slot_size: 65536,   // 64KB per slot (fits most JS files)
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
    let dispatcher = dispatcher.add_service(JsProcessorService(Arc::new(JsProcessorServer::new(
        JsProcessorImpl,
    ))));
    let dispatcher = dispatcher.build();

    session.set_dispatcher(dispatcher);

    // Run the RPC session demux loop (this is the main event loop now)
    tracing::info!("JS plugin ready, waiting for requests");
    if let Err(e) = session.run().await {
        tracing::error!(error = ?e, "RPC session error - host connection lost");
    }

    Ok(())
}