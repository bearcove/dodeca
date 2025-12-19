mod tools;

use rmcp::{ServiceExt, transport::stdio};
use tools::PikruServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing to stderr (stdout is for MCP protocol)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let server = PikruServer::new()?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;

    Ok(())
}
