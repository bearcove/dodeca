//! Dodeca config cell (cell-config)
//!
//! Parses configuration files using facet-styx.

use cell_config_proto::{ConfigParser, ConfigParserDispatcher, DodecaConfig, ParseConfigResult};
use dodeca_cell_runtime::run_cell;

/// Config parser implementation
#[derive(Clone)]
pub struct ConfigParserImpl;

impl ConfigParser for ConfigParserImpl {
    async fn parse_styx(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        content: String,
    ) -> ParseConfigResult {
        match facet_styx::from_str::<DodecaConfig>(&content) {
            Ok(config) => ParseConfigResult::Success {
                config: Box::new(config),
            },
            Err(e) => ParseConfigResult::Error {
                message: format!("{}", e),
            },
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("config", |_handle| ConfigParserDispatcher::new(
        ConfigParserImpl
    ))
}
