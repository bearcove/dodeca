mod handler;
mod tools;

use handler::PikruServerHandler;
use rust_mcp_sdk::schema::{
    Implementation, InitializeResult, ServerCapabilities, ServerCapabilitiesTools,
    LATEST_PROTOCOL_VERSION,
};
use rust_mcp_sdk::{
    error::SdkResult,
    mcp_server::{server_runtime, ServerRuntime},
    McpServer, StdioTransport, TransportOptions,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> SdkResult<()> {
    // Initialize tracing to stderr (stdout is for MCP protocol)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let server_details = InitializeResult {
        server_info: Implementation {
            name: "pikru-test".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            title: Some("Pikru Compliance Test Server".to_string()),
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            ..Default::default()
        },
        meta: None,
        instructions: Some(
            "Run pikchr compliance tests comparing C and Rust implementations".to_string(),
        ),
        protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
    };

    let transport = StdioTransport::new(TransportOptions::default())?;
    let handler = PikruServerHandler::new().map_err(|e| {
        rust_mcp_sdk::error::McpSdkError::from(std::io::Error::other(e))
    })?;
    let server: Arc<ServerRuntime> =
        server_runtime::create_server(server_details, transport, handler);

    if let Err(start_error) = server.start().await {
        eprintln!(
            "{}",
            start_error
                .rpc_error_message()
                .unwrap_or(&start_error.to_string())
        );
    }
    Ok(())
}
