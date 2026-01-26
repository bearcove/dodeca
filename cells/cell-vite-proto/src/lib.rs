//! RPC protocol for dodeca vite cell
//!
//! Manages Vite dev server and production builds.

use facet::Facet;

/// Result of starting a Vite dev server
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum StartDevServerResult {
    /// Server started successfully
    Success { port: u16 },
    /// Error starting server
    Error { message: String },
}

/// Result of running a Vite production build
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum RunBuildResult {
    /// Build completed successfully
    Success,
    /// Error during build
    Error { message: String },
}

/// Vite management service
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait ViteManager {
    /// Start a Vite dev server in the given project directory.
    /// Returns the port the server is listening on.
    async fn start_dev_server(&self, project_dir: String) -> StartDevServerResult;

    /// Run a Vite production build in the given project directory.
    async fn run_build(&self, project_dir: String) -> RunBuildResult;
}
