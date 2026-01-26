//! RPC protocol for dodeca config cell
//!
//! Parses configuration files using facet-styx.

pub use dodeca_config::DodecaConfig;
use facet::Facet;

/// Result of config parsing
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum ParseConfigResult {
    /// Successfully parsed config
    Success { config: Box<DodecaConfig> },
    /// Error during parsing
    Error { message: String },
}

/// Config parsing service
#[allow(async_fn_in_trait)]
#[roam::service]
pub trait ConfigParser {
    /// Parse a styx config file
    async fn parse_styx(&self, content: String) -> ParseConfigResult;
}
