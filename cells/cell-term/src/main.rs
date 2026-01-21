//! Dodeca term cell (cell-term)
//!
//! This cell handles terminal session recording with ANSI color support.

use cell_term_proto::{RecordConfig, TermRecorder, TermRecorderDispatcher, TermResult};
use dodeca_cell_runtime::run_cell;

mod parser;
mod recorder;
mod renderer;

/// TermRecorder implementation
#[derive(Clone)]
pub struct TermRecorderImpl;

impl TermRecorder for TermRecorderImpl {
    async fn record_interactive(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        config: RecordConfig,
    ) -> TermResult {
        match recorder::record_session(None, config).await {
            Ok(html) => TermResult::Success { html },
            Err(e) => TermResult::Error {
                message: e.to_string(),
            },
        }
    }

    async fn record_command(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        command: String,
        config: RecordConfig,
    ) -> TermResult {
        match recorder::record_session(Some(command), config).await {
            Ok(html) => TermResult::Success { html },
            Err(e) => TermResult::Error {
                message: e.to_string(),
            },
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("term", |_handle| TermRecorderDispatcher::new(
        TermRecorderImpl
    ))
}
