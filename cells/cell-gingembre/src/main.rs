//! Template rendering cell using gingembre.
//!
//! This cell handles template rendering with bidirectional RPC:
//! - Receives render requests from the host
//! - Calls back to host for template loading and data resolution

use cell_gingembre_proto::{
    ContextId, EvalResult, RenderResult, TemplateHostClient, TemplateRenderer,
    TemplateRendererServer,
};
use rapace_cell::RpcSession;
use std::sync::Arc;

/// Cell context holding the RPC session for callbacks
pub struct CellContext {
    pub session: Arc<RpcSession>,
}

impl CellContext {
    /// Create a client for calling back to the host
    pub fn host_client(&self) -> TemplateHostClient {
        TemplateHostClient::new(self.session.clone())
    }
}

/// Template renderer implementation
pub struct TemplateRendererImpl {
    #[allow(dead_code)] // Will be used when implementation is complete
    ctx: Arc<CellContext>,
}

impl TemplateRendererImpl {
    pub fn new(ctx: Arc<CellContext>) -> Self {
        Self { ctx }
    }
}

impl TemplateRenderer for TemplateRendererImpl {
    async fn render(
        &self,
        _context_id: ContextId,
        _template_name: String,
        _initial_context_json: String,
    ) -> RenderResult {
        // TODO: Implement with async gingembre
        RenderResult::Error {
            message: "Not yet implemented".to_string(),
        }
    }

    async fn eval_expression(
        &self,
        _context_id: ContextId,
        _expression: String,
        _context_json: String,
    ) -> EvalResult {
        // TODO: Implement with async gingembre
        EvalResult::Error {
            message: "Not yet implemented".to_string(),
        }
    }
}

rapace_cell::cell_service!(
    TemplateRendererServer<TemplateRendererImpl>,
    TemplateRendererImpl
);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run_with_session(|session: Arc<RpcSession>| {
        let ctx = Arc::new(CellContext { session });
        let renderer = TemplateRendererImpl::new(ctx);
        CellService::from(renderer)
    })
    .await?;
    Ok(())
}
