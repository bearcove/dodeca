//! Transitional facade for former dodeca cells.
//!
//! The monolith migration is replacing internal Vox/dynamic-cell calls with
//! direct Rust calls while preserving this module's helper names for the rest
//! of the codebase. Callback-heavy modules still use the old client path until
//! their host interactions are made local.

use cell_code_execution_proto::{
    CodeExecutionResult, CodeExecutor, CodeExecutorClient, ExecuteSamplesInput, ExtractSamplesInput,
};
use cell_css_proto::{CssProcessor, CssProcessorClient, CssResult};
use cell_data_proto::{DataFormat, DataLoader, DataLoaderClient, LoadDataResult};
use cell_dialoguer_proto::{Dialoguer, DialoguerClient, SelectResult};
use cell_fonts_proto::{FontProcessor, FontProcessorClient, FontResult, SubsetFontInput};
use cell_gingembre_proto::{ContextId, RenderResult, TemplateRendererClient};
use cell_host_proto::{
    CallFunctionResult, CommandResult, HostService, KeysAtResult, LoadTemplateResult, ReadyAck,
    ReadyMsg, ResolveDataResult, ServeContent, ServerCommand, Value,
};
use cell_html_diff_proto::{DiffError, DiffInput, DiffOutcome, HtmlDiffer, HtmlDifferClient};
use cell_html_proto::{
    HtmlProcessInput, HtmlProcessResult, HtmlProcessor, HtmlProcessorClient, HtmlResult,
};
use cell_http_proto::{ScopeEntry, TcpTunnelClient};
use cell_image_proto::{
    ImageProcessor, ImageProcessorClient, ImageResult, ResizeInput, ThumbhashInput,
};
use cell_js_proto::{JsProcessor, JsProcessorClient, JsRewriteInput};
use cell_jxl_proto::{JXLEncodeInput, JXLProcessor, JXLProcessorClient, JXLResult};
use cell_lifecycle_proto::CellLifecycle;
use cell_linkcheck_proto::{
    LinkCheckInput, LinkCheckResult, LinkChecker, LinkCheckerClient, LinkStatus,
};
use cell_markdown_proto::{MarkdownProcessor, MarkdownProcessorClient};
use cell_minify_proto::{Minifier, MinifyResult};
use cell_sass_proto::{SassCompiler, SassCompilerClient, SassResult};
use cell_search_proto::{
    SearchFile, SearchIndexResult, SearchIndexer, SearchIndexerClient, SearchPage,
};
use cell_svgo_proto::{SvgoOptimizer, SvgoOptimizerClient, SvgoResult};
use cell_term_proto::{RecordConfig, TermRecorder, TermRecorderClient, TermResult};
use cell_tui_proto::TuiDisplayClient;
use cell_vite_proto::{RunBuildResult, StartDevServerResult, ViteManager, ViteManagerClient};
use cell_webp_proto::{WebPEncodeInput, WebPProcessor, WebPProcessorClient, WebPResult};
use dashmap::DashMap;
use facet::Facet;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use std::time::SystemTime;
use tracing::debug;

use crate::serve::SiteServer;

#[derive(Clone)]
pub(crate) struct DodecaHtmlCallbacks;

type HtmlCallbackFuture<'a> = Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;

impl ddc_cell_html::HtmlCallbacks for DodecaHtmlCallbacks {
    fn process_inline_css<'a>(
        &'a self,
        css: String,
        path_map: HashMap<String, String>,
    ) -> HtmlCallbackFuture<'a> {
        Box::pin(async move {
            rewrite_urls_in_css_cell(css, path_map)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn process_inline_js<'a>(
        &'a self,
        js: String,
        path_map: HashMap<String, String>,
    ) -> HtmlCallbackFuture<'a> {
        Box::pin(async move {
            rewrite_string_literals_in_js_cell(js, path_map)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn minify_css<'a>(&'a self, css: String) -> HtmlCallbackFuture<'a> {
        Box::pin(async move {
            rewrite_urls_in_css_cell(css, HashMap::new())
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn minify_js<'a>(&'a self, js: String) -> HtmlCallbackFuture<'a> {
        Box::pin(async move {
            rewrite_string_literals_in_js_cell(js, HashMap::new())
                .await
                .map_err(|e| e.to_string())
        })
    }
}

pub(crate) fn html_processor() -> ddc_cell_html::HtmlProcessorImpl<DodecaHtmlCallbacks> {
    ddc_cell_html::HtmlProcessorImpl::with_callbacks(DodecaHtmlCallbacks)
}

// ============================================================================
// Global State
// ============================================================================

static CELL_RPC_ID: AtomicU64 = AtomicU64::new(1);

fn next_cell_rpc_id() -> u64 {
    CELL_RPC_ID.fetch_add(1, Ordering::Relaxed)
}

// Note: Most globals have been moved to Host singleton:
// - Site server: Host::get().site_server()
// - Cell handles: Host::get().get_cell_handle(name)

/// Provide the SiteServer for HTTP cell initialization.
/// This must be called before cells are initialized when the HTTP cell needs to serve content.
/// For build-only commands, this can be skipped.
pub fn provide_site_server(server: Arc<SiteServer>) {
    crate::host::Host::get().provide_site_server(server);
}

// Note: TUI command forwarding now goes through Host::get().handle_tui_command()
// The old TUI_HOST_FOR_INIT global has been removed.
// Exit signaling now goes through Host::get().signal_exit() / wait_for_exit().
// ============================================================================
// Cell Readiness Registry
// ============================================================================

/// Registry for tracking cell readiness (RPC-ready state).
/// Tracks by cell name (logical name like "gingembre", "sass").
#[derive(Clone)]
pub struct CellReadyRegistry {
    ready: Arc<DashMap<String, ReadyMsg>>,
}

impl CellReadyRegistry {
    fn new() -> Self {
        Self {
            ready: Arc::new(DashMap::new()),
        }
    }

    fn mark_ready(&self, msg: ReadyMsg) {
        // Normalize: cells report with underscores (code_execution) but we use hyphens (code-execution)
        let cell_name = msg.cell_name.replace('_', "-");
        debug!(
            cell_name = %cell_name,
            peer_id = msg.peer_id,
            "CellReadyRegistry::mark_ready: marking cell as ready"
        );
        self.ready.insert(cell_name.clone(), msg);
        debug!(
            cell_name = %cell_name,
            "CellReadyRegistry::mark_ready: cell marked ready, registry now has {} cells",
            self.ready.len()
        );
    }
}

static CELL_READY_REGISTRY: OnceLock<CellReadyRegistry> = OnceLock::new();

pub fn cell_ready_registry() -> &'static CellReadyRegistry {
    CELL_READY_REGISTRY.get_or_init(CellReadyRegistry::new)
}

/// Host implementation of CellLifecycle service
#[derive(Clone)]
pub struct HostCellLifecycle {
    registry: CellReadyRegistry,
}

impl HostCellLifecycle {
    pub fn new(registry: CellReadyRegistry) -> Self {
        Self { registry }
    }
}

impl CellLifecycle for HostCellLifecycle {
    async fn ready(&self, msg: ReadyMsg) -> ReadyAck {
        let peer_id = msg.peer_id;
        let cell_name = msg.cell_name.clone();
        debug!("Cell {} (peer_id={}) is ready", cell_name, peer_id);
        self.registry.mark_ready(msg);

        let host_time_unix_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as u64);

        ReadyAck {
            ok: true,
            host_time_unix_ms,
        }
    }
}

// ============================================================================
// Unified Host Service Implementation
// ============================================================================

/// Unified host service that all cells connect to.
///
/// This implements `HostService` by delegating to the specialized implementations.
/// TUI command forwarding goes through `Host::get()`.
#[derive(Clone)]
pub struct HostServiceImpl {
    lifecycle: HostCellLifecycle,
    template_host: crate::template_host::TemplateHostImpl,
    site_server: Option<Arc<SiteServer>>,
}

impl HostServiceImpl {
    pub fn new(
        lifecycle: HostCellLifecycle,
        template_host: crate::template_host::TemplateHostImpl,
        site_server: Option<Arc<SiteServer>>,
    ) -> Self {
        Self {
            lifecycle,
            template_host,
            site_server,
        }
    }
}

/// Build the unified host service the cell loader serves back to every cell.
pub fn make_host_service() -> HostServiceImpl {
    HostServiceImpl::new(
        HostCellLifecycle::new(cell_ready_registry().clone()),
        crate::template_host::TemplateHostImpl::new(),
        crate::host::Host::get().site_server().cloned(),
    )
}

impl HostService for HostServiceImpl {
    // Cell Lifecycle
    async fn ready(&self, msg: ReadyMsg) -> ReadyAck {
        let peer_id = msg.peer_id;
        let cell_name = msg.cell_name.clone();
        debug!("Cell {} (peer_id={}) is ready", cell_name, peer_id);
        self.lifecycle.registry.mark_ready(msg);

        let host_time_unix_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as u64);

        ReadyAck {
            ok: true,
            host_time_unix_ms,
        }
    }

    // Template Host
    async fn load_template(&self, context_id: ContextId, name: String) -> LoadTemplateResult {
        use cell_gingembre_proto::TemplateHost;
        self.template_host.load_template(context_id, name).await
    }

    async fn resolve_data(&self, context_id: ContextId, path: Vec<String>) -> ResolveDataResult {
        use cell_gingembre_proto::TemplateHost;
        self.template_host.resolve_data(context_id, path).await
    }

    async fn keys_at(&self, context_id: ContextId, path: Vec<String>) -> KeysAtResult {
        use cell_gingembre_proto::TemplateHost;
        self.template_host.keys_at(context_id, path).await
    }

    async fn call_function(
        &self,
        context_id: ContextId,
        name: String,
        args: Vec<Value>,
        kwargs: Vec<(String, Value)>,
    ) -> CallFunctionResult {
        use cell_gingembre_proto::TemplateHost;
        self.template_host
            .call_function(context_id, name, args, kwargs)
            .await
    }

    // Content Service
    async fn find_content(
        &self,
        path: String,
        identity: Option<cell_http_proto::Identity>,
    ) -> ServeContent {
        if let Some(server) = &self.site_server {
            use cell_http_proto::ContentService;
            let content_service = crate::content_service::HostContentService::new(server.clone());
            content_service.find_content(path, identity).await
        } else {
            ServeContent::NotFound {
                html: "Not in serve mode".to_string(),
                generation: 0,
            }
        }
    }

    async fn get_scope(&self, route: String, path: Vec<String>) -> Vec<ScopeEntry> {
        if let Some(server) = &self.site_server {
            use cell_http_proto::ContentService;
            let content_service = crate::content_service::HostContentService::new(server.clone());
            content_service.get_scope(route, path).await
        } else {
            vec![]
        }
    }

    async fn eval_expression(
        &self,
        route: String,
        expression: String,
    ) -> cell_host_proto::EvalResult {
        if let Some(server) = &self.site_server {
            use cell_http_proto::ContentService;
            let content_service = crate::content_service::HostContentService::new(server.clone());
            content_service.eval_expression(route, expression).await
        } else {
            cell_host_proto::EvalResult::Err("Not in serve mode".to_string())
        }
    }

    // TUI Commands (TUI → Host)
    async fn send_command(&self, command: ServerCommand) -> CommandResult {
        // Forward to Host singleton
        crate::host::Host::get().handle_tui_command(command)
    }

    async fn quit(&self) {
        crate::host::Host::get().signal_exit();
    }

    // Vite Integration
    async fn get_vite_port(&self) -> Option<u16> {
        crate::host::Host::get().get_vite_port()
    }

    // HTML Host callbacks
    async fn minify_css(&self, css: String) -> cell_host_proto::MinifyCssResult {
        match rewrite_urls_in_css_cell(css, HashMap::new()).await {
            Ok(css) => cell_host_proto::MinifyCssResult::Success { css },
            Err(e) => cell_host_proto::MinifyCssResult::Error {
                message: e.to_string(),
            },
        }
    }

    async fn minify_js(&self, js: String) -> cell_host_proto::MinifyJsResult {
        match rewrite_string_literals_in_js_cell(js, HashMap::new()).await {
            Ok(js) => cell_host_proto::MinifyJsResult::Success { js },
            Err(e) => cell_host_proto::MinifyJsResult::Error {
                message: e.to_string(),
            },
        }
    }

    async fn process_inline_css(
        &self,
        css: String,
        path_map: HashMap<String, String>,
    ) -> cell_host_proto::ProcessCssResult {
        match rewrite_urls_in_css_cell(css, path_map).await {
            Ok(css) => cell_host_proto::ProcessCssResult::Success { css },
            Err(e) => cell_host_proto::ProcessCssResult::Error {
                message: e.to_string(),
            },
        }
    }

    async fn process_inline_js(
        &self,
        js: String,
        path_map: HashMap<String, String>,
    ) -> cell_host_proto::ProcessJsResult {
        match rewrite_string_literals_in_js_cell(js, path_map).await {
            Ok(js) => cell_host_proto::ProcessJsResult::Success { js },
            Err(e) => cell_host_proto::ProcessJsResult::Error {
                message: e.to_string(),
            },
        }
    }
}

/// Get the TUI display client for pushing updates to the TUI cell.
/// This will spawn the TUI cell if it hasn't been spawned yet.
pub async fn get_tui_display_client() -> Option<TuiDisplayClient> {
    crate::host::Host::get()
        .client_async::<TuiDisplayClient>()
        .await
}

// ============================================================================
// Decoded Image Type (re-export)
// ============================================================================

pub type DecodedImage = cell_image_proto::DecodedImage;

// ============================================================================
// Template Rendering
// ============================================================================

pub async fn render_template(
    context_id: ContextId,
    template_name: &str,
    initial_context: Value,
) -> eyre::Result<RenderResult> {
    let rpc_id = next_cell_rpc_id();
    let started_at = Instant::now();
    tracing::debug!(
        rpc_id,
        cell = "gingembre",
        method = "render",
        context_id = context_id.0,
        template_name,
        "cell rpc client lookup starting"
    );
    let cell = crate::host::Host::get()
        .client_async::<TemplateRendererClient>()
        .await
        .ok_or_else(|| eyre::eyre!("Gingembre cell not available"))?;
    tracing::debug!(
        rpc_id,
        cell = "gingembre",
        method = "render",
        elapsed_ms = started_at.elapsed().as_millis(),
        "cell rpc dispatch starting"
    );
    let result = cell
        .render(context_id, template_name.to_string(), initial_context)
        .await
        .map_err(|e| {
            tracing::error!(
                rpc_id,
                cell = "gingembre",
                method = "render",
                elapsed_ms = started_at.elapsed().as_millis(),
                error = ?e,
                "cell rpc failed"
            );
            eyre::eyre!("RPC call error: {:?}", e)
        })?;
    tracing::debug!(
        rpc_id,
        cell = "gingembre",
        method = "render",
        elapsed_ms = started_at.elapsed().as_millis(),
        "cell rpc complete"
    );
    Ok(result)
}

// ============================================================================
// Cell Client Accessor Functions
// ============================================================================

/// Create a client for the given cell if available.
///
/// Uses Host for handle lookup. With lazy spawning, will spawn cell on first access.
macro_rules! cell_client_accessor {
    ($name:ident, $suffix:expr, $client:ty) => {
        #[allow(unused)]
        pub async fn $name() -> Option<Arc<$client>> {
            // Use Host for handle lookup with lazy spawning support
            crate::host::Host::get()
                .client_async::<$client>()
                .await
                .map(Arc::new)
        }
    };
}

// Image processing
cell_client_accessor!(image_cell, "image", ImageProcessorClient);
cell_client_accessor!(webp_cell, "webp", WebPProcessorClient);
cell_client_accessor!(jxl_cell, "jxl", JXLProcessorClient);

// Text processing
cell_client_accessor!(markdown_cell, "markdown", MarkdownProcessorClient);
cell_client_accessor!(html_cell, "html", HtmlProcessorClient);
cell_client_accessor!(css_cell, "css", CssProcessorClient);
cell_client_accessor!(sass_cell, "sass", SassCompilerClient);
cell_client_accessor!(js_cell, "js", JsProcessorClient);
cell_client_accessor!(svgo_cell, "svgo", SvgoOptimizerClient);

// Template rendering
cell_client_accessor!(gingembre_cell, "gingembre", TemplateRendererClient);

// Data processing
cell_client_accessor!(data_cell, "data", DataLoaderClient);

// Vite management
cell_client_accessor!(vite_cell, "vite", ViteManagerClient);

// Other cells
cell_client_accessor!(font_cell, "fonts", FontProcessorClient);
cell_client_accessor!(linkcheck_cell, "linkcheck", LinkCheckerClient);
cell_client_accessor!(search_cell, "search", SearchIndexerClient);
cell_client_accessor!(html_diff_cell, "html_diff", HtmlDifferClient);
cell_client_accessor!(dialoguer_cell, "dialoguer", DialoguerClient);
cell_client_accessor!(code_execution_cell, "code_execution", CodeExecutorClient);
cell_client_accessor!(http_cell, "http", TcpTunnelClient);
cell_client_accessor!(term_cell, "term", TermRecorderClient);

/// Record a terminal session interactively
pub async fn record_term_interactive(config: RecordConfig) -> Result<TermResult, eyre::Error> {
    Ok(ddc_cell_term::TermRecorderImpl
        .record_interactive(config)
        .await)
}

/// Record a terminal session with an auto-executed command
pub async fn record_term_command(
    command: String,
    config: RecordConfig,
) -> Result<TermResult, eyre::Error> {
    Ok(ddc_cell_term::TermRecorderImpl
        .record_command(command, config)
        .await)
}

pub async fn optimize_svg(svg: String) -> Result<SvgoResult, eyre::Error> {
    Ok(ddc_cell_svgo::SvgoOptimizerImpl.optimize_svg(svg).await)
}

pub async fn subset_font(input: SubsetFontInput) -> Result<FontResult, eyre::Error> {
    Ok(ddc_cell_fonts::FontProcessorImpl.subset_font(input).await)
}

/// Build the full-text search index from rendered pages via the search cell.
pub async fn build_search_index_cell(
    pages: Vec<SearchPage>,
) -> Result<Vec<SearchFile>, eyre::Error> {
    match ddc_cell_search::SearchIndexerImpl.build_index(pages).await {
        SearchIndexResult::Success { files } => Ok(files),
        SearchIndexResult::Error { message } => {
            Err(eyre::eyre!("search indexing failed: {message}"))
        }
    }
}

pub async fn execute_code_samples(
    input: ExecuteSamplesInput,
) -> Result<CodeExecutionResult, eyre::Error> {
    Ok(ddc_cell_code_execution::CodeExecutorImpl
        .execute_code_samples(input)
        .await)
}

pub async fn extract_code_samples(
    input: ExtractSamplesInput,
) -> Result<CodeExecutionResult, eyre::Error> {
    Ok(ddc_cell_code_execution::CodeExecutorImpl
        .extract_code_samples(input)
        .await)
}

// ============================================================================
// Additional Function Aliases (for compatibility with other modules)
// ============================================================================

// These are wrapper functions that other modules expect while the facade is
// still being collapsed into direct calls.

pub async fn select_dialog(prompt: String, items: Vec<String>) -> SelectResult {
    ddc_cell_dialoguer::DialoguerImpl
        .select(prompt, items)
        .await
}

pub async fn start_vite_dev_server_cell(project_dir: String) -> StartDevServerResult {
    ddc_cell_vite::ViteManagerImpl
        .start_dev_server(project_dir)
        .await
}

pub async fn run_vite_build_cell(project_dir: String) -> RunBuildResult {
    ddc_cell_vite::ViteManagerImpl.run_build(project_dir).await
}

pub async fn diff_html_cell(input: DiffInput) -> Result<DiffOutcome, DiffError> {
    ddc_cell_html_diff::HtmlDifferImpl.diff_html(input).await
}

pub async fn minify_html_cell(html: String) -> MinifyResult {
    ddc_cell_minify::MinifierImpl.minify_html(html).await
}

pub async fn process_html_cell(input: HtmlProcessInput) -> HtmlProcessResult {
    html_processor().process(input).await
}

/// Result of link checking - wrapper for internal use
#[derive(Debug, Clone)]
pub struct UrlCheckResult {
    pub statuses: std::collections::HashMap<String, LinkStatus>,
}

pub async fn check_urls_cell(urls: Vec<String>, options: CheckOptions) -> Option<UrlCheckResult> {
    let input = LinkCheckInput {
        urls,
        delay_ms: options.rate_limit_ms,
        timeout_secs: options.timeout_secs,
    };
    match ddc_cell_linkcheck::LinkCheckerImpl::new()
        .check_links(input)
        .await
    {
        LinkCheckResult::Success { output } => Some(UrlCheckResult {
            statuses: output.results,
        }),
        LinkCheckResult::Error { message } => {
            tracing::warn!("Link check error: {}", message);
            None
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CheckOptions {
    pub timeout_secs: u64,
    pub rate_limit_ms: u64,
}

pub async fn highlight_code_cell(lang: &str, code: &str) -> Result<String, eyre::Error> {
    match ddc_cell_markdown::MarkdownProcessorImpl::new()
        .highlight_code(lang.to_string(), code.to_string())
        .await
    {
        cell_markdown_proto::HighlightResult::Success { html } => Ok(html),
        cell_markdown_proto::HighlightResult::Error { message } => {
            Err(eyre::eyre!("Highlight error: {}", message))
        }
    }
}

pub async fn parse_and_render_markdown_cell(
    source_path: &str,
    content: &str,
    source_map: bool,
) -> Result<cell_markdown_proto::ParseResult, MarkdownParseError> {
    let rpc_id = next_cell_rpc_id();
    let started_at = Instant::now();
    tracing::debug!(
        rpc_id,
        cell = "markdown",
        method = "parse_and_render",
        source_path,
        source_len = content.len(),
        source_map,
        "markdown render starting"
    );
    let result = ddc_cell_markdown::MarkdownProcessorImpl::new()
        .parse_and_render(source_path.to_string(), content.to_string(), source_map)
        .await;
    match result {
        result @ cell_markdown_proto::ParseResult::Success { .. } => {
            tracing::debug!(
                rpc_id,
                cell = "markdown",
                method = "parse_and_render",
                elapsed_ms = started_at.elapsed().as_millis(),
                "markdown render complete"
            );
            Ok(result)
        }
        cell_markdown_proto::ParseResult::Error { message } => {
            tracing::error!(
                rpc_id,
                cell = "markdown",
                method = "parse_and_render",
                elapsed_ms = started_at.elapsed().as_millis(),
                %message,
                "markdown render failed"
            );
            Err(MarkdownParseError { message })
        }
    }
}

pub async fn execute_code_samples_cell(
    input: ExecuteSamplesInput,
) -> Result<CodeExecutionResult, eyre::Error> {
    execute_code_samples(input).await
}

pub async fn extract_code_samples_cell(
    input: ExtractSamplesInput,
) -> Result<CodeExecutionResult, eyre::Error> {
    extract_code_samples(input).await
}

pub async fn inject_code_buttons_cell(
    html: String,
    code_metadata: HashMap<String, cell_html_proto::CodeExecutionMetadata>,
) -> Result<(String, bool), eyre::Error> {
    let rpc_id = next_cell_rpc_id();
    let started_at = Instant::now();
    tracing::debug!(
        rpc_id,
        cell = "html",
        method = "inject_code_buttons",
        html_len = html.len(),
        metadata_count = code_metadata.len(),
        "html injection starting"
    );
    match html_processor()
        .inject_code_buttons(html, code_metadata)
        .await
    {
        HtmlResult::SuccessWithFlag { html, flag } => {
            tracing::debug!(
                rpc_id,
                cell = "html",
                method = "inject_code_buttons",
                elapsed_ms = started_at.elapsed().as_millis(),
                output_len = html.len(),
                had_buttons = flag,
                "html injection complete"
            );
            Ok((html, flag))
        }
        HtmlResult::Success { html } => {
            tracing::debug!(
                rpc_id,
                cell = "html",
                method = "inject_code_buttons",
                elapsed_ms = started_at.elapsed().as_millis(),
                output_len = html.len(),
                had_buttons = false,
                "html injection complete"
            );
            Ok((html, false))
        }
        HtmlResult::Error { message } => {
            tracing::error!(
                rpc_id,
                cell = "html",
                method = "inject_code_buttons",
                elapsed_ms = started_at.elapsed().as_millis(),
                %message,
                "html injection returned error"
            );
            Err(eyre::eyre!(message))
        }
    }
}

pub async fn render_template_cell(
    context_id: ContextId,
    template_name: &str,
    initial_context: Value,
) -> eyre::Result<RenderResult> {
    render_template(context_id, template_name, initial_context).await
}

pub async fn eval_expression_cell(
    context_id: ContextId,
    expression: &str,
    context: Value,
) -> eyre::Result<cell_gingembre_proto::EvalResult> {
    let cell = crate::host::Host::get()
        .client_async::<TemplateRendererClient>()
        .await
        .ok_or_else(|| eyre::eyre!("Gingembre cell not available"))?;
    let result = cell
        .eval_expression(context_id, expression.to_string(), context)
        .await
        .map_err(|e| eyre::eyre!("RPC call error: {:?}", e))?;
    Ok(result)
}

pub async fn optimize_svg_cell(input: String) -> Result<SvgoResult, eyre::Error> {
    optimize_svg(input).await
}

pub async fn load_data_cell(content: String, format: DataFormat) -> LoadDataResult {
    ddc_cell_data::DataLoaderImpl
        .load_data(content, format)
        .await
}

/// Extract links and element IDs from HTML using the HTML cell's parser.
/// This uses a proper HTML parser instead of regex.
pub async fn extract_links_from_html(
    html: String,
) -> Result<cell_html_proto::ExtractedLinks, eyre::Error> {
    Ok(html_processor().extract_links(html).await)
}

pub async fn rewrite_string_literals_in_js_cell(
    js: String,
    path_map: HashMap<String, String>,
) -> Result<String, eyre::Error> {
    let input = JsRewriteInput { js, path_map };
    ddc_cell_js::JsProcessorImpl
        .rewrite_string_literals(input)
        .await
        .map_err(|e| eyre::eyre!("JS rewrite error: {e}"))
}

pub async fn rewrite_urls_in_css_cell(
    css: String,
    path_map: HashMap<String, String>,
) -> Result<String, eyre::Error> {
    match ddc_cell_css::CssProcessorImpl
        .rewrite_and_minify(css, path_map)
        .await
    {
        CssResult::Success { css } => Ok(css),
        CssResult::Error { message } => Err(eyre::eyre!(message)),
    }
}

pub async fn decompress_font_cell(data: Vec<u8>) -> Result<Vec<u8>, eyre::Error> {
    match ddc_cell_fonts::FontProcessorImpl
        .decompress_font(data)
        .await
    {
        FontResult::DecompressSuccess { data } => Ok(data),
        FontResult::Error { message } => Err(eyre::eyre!(message)),
        other => Err(eyre::eyre!("Unexpected result: {:?}", other)),
    }
}

pub async fn compress_to_woff2_cell(data: Vec<u8>) -> Result<Vec<u8>, eyre::Error> {
    match ddc_cell_fonts::FontProcessorImpl
        .compress_to_woff2(data)
        .await
    {
        FontResult::CompressSuccess { data } => Ok(data),
        FontResult::Error { message } => Err(eyre::eyre!(message)),
        other => Err(eyre::eyre!("Unexpected result: {:?}", other)),
    }
}

pub async fn subset_font_cell(input: SubsetFontInput) -> Result<FontResult, eyre::Error> {
    subset_font(input).await
}

// Image decoding/encoding cell wrappers
// These return Option to match what image.rs expects
pub async fn decode_png_cell(data: &[u8]) -> Option<DecodedImage> {
    match ddc_cell_image::ImageProcessorImpl
        .decode_png(data.to_vec())
        .await
    {
        ImageResult::Success { image } => Some(image),
        ImageResult::Error { message } => {
            tracing::warn!("PNG decode error: {}", message);
            None
        }
        _ => None,
    }
}

pub async fn decode_jpeg_cell(data: &[u8]) -> Option<DecodedImage> {
    match ddc_cell_image::ImageProcessorImpl
        .decode_jpeg(data.to_vec())
        .await
    {
        ImageResult::Success { image } => Some(image),
        ImageResult::Error { message } => {
            tracing::warn!("JPEG decode error: {}", message);
            None
        }
        _ => None,
    }
}

pub async fn decode_gif_cell(data: &[u8]) -> Option<DecodedImage> {
    match ddc_cell_image::ImageProcessorImpl
        .decode_gif(data.to_vec())
        .await
    {
        ImageResult::Success { image } => Some(image),
        ImageResult::Error { message } => {
            tracing::warn!("GIF decode error: {}", message);
            None
        }
        _ => None,
    }
}

pub async fn decode_webp_cell(data: &[u8]) -> Option<DecodedImage> {
    match ddc_cell_webp::WebPProcessorImpl
        .decode_webp(data.to_vec())
        .await
    {
        WebPResult::DecodeSuccess {
            pixels,
            width,
            height,
            channels,
        } => Some(DecodedImage {
            pixels,
            width,
            height,
            channels,
        }),
        WebPResult::Error { message } => {
            tracing::warn!("WebP decode error: {}", message);
            None
        }
        _ => None,
    }
}

pub async fn decode_jxl_cell(data: &[u8]) -> Option<DecodedImage> {
    match ddc_cell_jxl::JXLProcessorImpl
        .decode_jxl(data.to_vec())
        .await
    {
        JXLResult::DecodeSuccess {
            pixels,
            width,
            height,
            channels,
        } => Some(DecodedImage {
            pixels,
            width,
            height,
            channels,
        }),
        JXLResult::Error { message } => {
            tracing::warn!("JXL decode error: {}", message);
            None
        }
        _ => None,
    }
}

pub async fn resize_image_cell(
    pixels: &[u8],
    width: u32,
    height: u32,
    channels: u8,
    target_width: u32,
) -> Option<DecodedImage> {
    let input = ResizeInput {
        pixels: pixels.to_vec(),
        width,
        height,
        channels,
        target_width,
    };
    match ddc_cell_image::ImageProcessorImpl.resize_image(input).await {
        ImageResult::Success { image } => Some(image),
        ImageResult::Error { message } => {
            tracing::warn!("Resize error: {}", message);
            None
        }
        _ => None,
    }
}

pub async fn generate_thumbhash_cell(pixels: &[u8], width: u32, height: u32) -> Option<String> {
    let input = ThumbhashInput {
        pixels: pixels.to_vec(),
        width,
        height,
    };
    match ddc_cell_image::ImageProcessorImpl
        .generate_thumbhash_data_url(input)
        .await
    {
        ImageResult::ThumbhashSuccess { data_url } => Some(data_url),
        ImageResult::Error { message } => {
            tracing::warn!("Thumbhash error: {}", message);
            None
        }
        _ => None,
    }
}

pub async fn encode_webp_cell(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Option<Vec<u8>> {
    let input = WebPEncodeInput {
        pixels: pixels.to_vec(),
        width,
        height,
        quality,
    };
    match ddc_cell_webp::WebPProcessorImpl.encode_webp(input).await {
        WebPResult::EncodeSuccess { data } => Some(data),
        WebPResult::Error { message } => {
            tracing::warn!("WebP encode error: {}", message);
            None
        }
        _ => None,
    }
}

pub async fn encode_jxl_cell(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Option<Vec<u8>> {
    let input = JXLEncodeInput {
        pixels: pixels.to_vec(),
        width,
        height,
        quality,
    };
    match ddc_cell_jxl::JXLProcessorImpl.encode_jxl(input).await {
        JXLResult::EncodeSuccess { data } => Some(data),
        JXLResult::Error { message } => {
            tracing::warn!("JXL encode error: {}", message);
            None
        }
        _ => None,
    }
}

// SASS/CSS cell wrappers
pub async fn compile_sass_cell(
    input: &HashMap<String, String>,
    load_paths: &[String],
) -> Result<SassResult, eyre::Error> {
    Ok(ddc_cell_sass::SassCompilerImpl
        .compile_sass("main.scss".to_string(), input.clone(), load_paths.to_vec())
        .await)
}

// Markdown error type
#[derive(Debug, Clone, Facet)]
pub struct MarkdownParseError {
    pub message: String,
}

impl std::fmt::Display for MarkdownParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for MarkdownParseError {}
