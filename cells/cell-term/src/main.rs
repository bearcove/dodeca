//! Dodeca term cell (cell-term)
//!
//! This cell handles terminal session recording with ANSI color support.

#[cfg(feature = "dynamic-cell")]
use cell_term_proto::TermRecorderDispatcher;
use cell_term_proto::{RecordConfig, TermRecorder, TermResult};

mod parser;
mod recorder;
mod renderer;

/// TermRecorder implementation
#[derive(Clone)]
pub struct TermRecorderImpl;

impl TermRecorder for TermRecorderImpl {
    async fn record_interactive(&self, config: RecordConfig) -> TermResult {
        match recorder::record_session(None, config).await {
            Ok(html) => TermResult::Success { html },
            Err(e) => TermResult::Error {
                message: e.to_string(),
            },
        }
    }

    async fn record_command(&self, command: String, config: RecordConfig) -> TermResult {
        match recorder::record_session(Some(command), config).await {
            Ok(html) => TermResult::Success { html },
            Err(e) => TermResult::Error {
                message: e.to_string(),
            },
        }
    }
}

#[cfg(feature = "dynamic-cell")]
dodeca_cell_runtime::declare_cell!("term", |_host| TermRecorderDispatcher::new(
    TermRecorderImpl
));
