//! RPC protocol for dodeca code execution plugin
//!
//! Defines services for extracting and executing code samples from markdown.

use facet::Facet;
use std::collections::HashMap;

// Re-export types from the types crate
pub use dodeca_code_execution_types::{
    CodeSample, ExecutionResult, ExtractSamplesInput, ExtractSamplesOutput,
    ExecuteSamplesInput, ExecuteSamplesOutput, CodeExecutionConfig, LanguageConfig,
};

/// Result of code execution operations
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum CodeExecutionResult {
    /// Successfully extracted samples
    ExtractSuccess { output: ExtractSamplesOutput },
    /// Successfully executed samples
    ExecuteSuccess { output: ExecuteSamplesOutput },
    /// Error during processing
    Error { message: String },
}

/// Code execution service implemented by the plugin.
///
/// The host calls these methods to process code samples.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait CodeExecutor {
    /// Extract code samples from markdown content
    async fn extract_code_samples(&self, input: ExtractSamplesInput) -> CodeExecutionResult;

    /// Execute code samples
    async fn execute_code_samples(&self, input: ExecuteSamplesInput) -> CodeExecutionResult;
}