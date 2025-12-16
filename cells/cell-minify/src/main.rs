//! Dodeca minify cell (cell-minify)
//!
//! This cell handles HTML minification.

use cell_minify_proto::{Minifier, MinifierServer, MinifyResult};

/// Minifier implementation
pub struct MinifierImpl;

impl Minifier for MinifierImpl {
    async fn minify_html(&self, html: String) -> MinifyResult {
        let cfg = minify_html::Cfg {
            minify_css: true,
            minify_js: true,
            // Preserve template syntax for compatibility
            preserve_brace_template_syntax: true,
            ..minify_html::Cfg::default()
        };

        let result = minify_html::minify(html.as_bytes(), &cfg);
        match String::from_utf8(result) {
            Ok(minified) => MinifyResult::Success { content: minified },
            Err(_) => MinifyResult::Error {
                message: "minification produced invalid UTF-8".to_string(),
            },
        }
    }
}

use std::sync::Arc;

struct CellService(Arc<MinifierServer<MinifierImpl>>);

impl rapace_cell::ServiceDispatch for CellService {
    fn dispatch(
        &self,
        method_id: u32,
        payload: &[u8],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<rapace::Frame, rapace::RpcError>>
                + Send
                + 'static,
        >,
    > {
        let server = self.0.clone();
        let payload = payload.to_vec();
        Box::pin(async move { server.dispatch(method_id, &payload).await })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = MinifierServer::new(MinifierImpl);
    rapace_cell::run(CellService(Arc::new(server))).await?;
    Ok(())
}
