use async_trait::async_trait;
use rust_mcp_sdk::schema::{
    CallToolRequest, CallToolResult, ListToolsRequest, ListToolsResult, RpcError,
    schema_utils::CallToolError,
};
use rust_mcp_sdk::{McpServer, mcp_server::ServerHandler};
use std::path::PathBuf;
use std::sync::Arc;

use crate::tools::PikruTools;

/// Paths to project resources
pub struct PikruPaths {
    pub project_root: PathBuf,
    pub tests_dir: PathBuf,
    pub c_pikchr: PathBuf,
}

pub struct PikruServerHandler {
    paths: PikruPaths,
}

impl PikruServerHandler {
    pub fn new() -> Result<Self, String> {
        // Find project root by looking for Cargo.toml
        let exe_path = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut project_root = exe_path.parent().map(|p| p.to_path_buf());

        // Walk up looking for Cargo.toml with pikru
        while let Some(ref path) = project_root {
            let cargo_toml = path.join("Cargo.toml");
            if cargo_toml.exists() {
                if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                    if content.contains("name = \"pikru\"") {
                        break;
                    }
                }
            }
            project_root = path.parent().map(|p| p.to_path_buf());
        }

        // Fallback: use PIKRU_ROOT env var or current dir
        let project_root = project_root
            .or_else(|| std::env::var("PIKRU_ROOT").ok().map(PathBuf::from))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let tests_dir = project_root.join("vendor/pikchr-c/tests");
        let c_pikchr = project_root.join("vendor/pikchr-c/pikchr");

        if !tests_dir.exists() {
            return Err(format!(
                "Tests directory not found: {}",
                tests_dir.display()
            ));
        }
        if !c_pikchr.exists() {
            return Err(format!("C pikchr binary not found: {}", c_pikchr.display()));
        }

        Ok(Self {
            paths: PikruPaths {
                project_root,
                tests_dir,
                c_pikchr,
            },
        })
    }
}

#[async_trait]
impl ServerHandler for PikruServerHandler {
    async fn handle_list_tools_request(
        &self,
        _request: ListToolsRequest,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult {
            meta: None,
            next_cursor: None,
            tools: PikruTools::tools(),
        })
    }

    async fn handle_call_tool_request(
        &self,
        request: CallToolRequest,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        let tool_params: PikruTools =
            PikruTools::try_from(request.params).map_err(CallToolError::new)?;

        match tool_params {
            PikruTools::ListPikruTestsTool(tool) => tool.call_tool(&self.paths),
            PikruTools::RunPikruTestTool(tool) => tool.call_tool(&self.paths),
        }
    }
}
