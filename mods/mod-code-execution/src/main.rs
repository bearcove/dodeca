//! Dodeca code execution plugin (dodeca-mod-code-execution)
//!
//! This plugin handles extracting and executing code samples from markdown.

use mod_code_execution_proto::{CodeExecutor, CodeExecutionResult, CodeExecutorServer};

// Include implementation code directly
include!("impl.rs");

dodeca_plugin_runtime::plugin_service!(
    CodeExecutorServer<CodeExecutorImpl>,
    CodeExecutorImpl
);

dodeca_plugin_runtime::run_plugin!(CodeExecutorImpl);
