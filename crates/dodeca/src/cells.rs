//! In-process processing facade for the former dodeca cells.
//!
//! Internal processing uses direct Rust calls into the former cell crates. The
//! protocol crates still hold the shared typed inputs/results for each operation.

use cell_code_execution_proto::{
    CodeExecutionResult, CodeExecutor, ExecuteSamplesInput, ExtractSamplesInput,
};
use cell_css_proto::{CssProcessor, CssResult};
use cell_data_proto::{DataFormat, DataLoader, LoadDataResult};
use cell_dialoguer_proto::{Dialoguer, SelectResult};
use cell_fonts_proto::{FontProcessor, FontResult, SubsetFontInput};
use cell_gingembre_proto::{ContextId, RenderResult, TemplateRenderer};
use cell_html_diff_proto::{DiffError, DiffInput, DiffOutcome, HtmlDiffer};
use cell_html_proto::{HtmlProcessInput, HtmlProcessResult, HtmlProcessor, HtmlResult};
use cell_image_proto::{ImageProcessor, ImageResult, ResizeInput, ThumbhashInput};
use cell_js_proto::{JsProcessor, JsRewriteInput};
use cell_jxl_proto::{JXLEncodeInput, JXLProcessor, JXLResult};
use cell_linkcheck_proto::{LinkCheckInput, LinkCheckResult, LinkChecker, LinkStatus};
use cell_markdown_proto::MarkdownProcessor;
use cell_minify_proto::{Minifier, MinifyResult};
use cell_sass_proto::{SassCompiler, SassResult};
use cell_search_proto::{SearchFile, SearchIndexResult, SearchIndexer, SearchPage};
use cell_svgo_proto::{SvgoOptimizer, SvgoResult};
use cell_term_proto::{RecordConfig, TermRecorder, TermResult};
use cell_vite_proto::{RunBuildResult, StartDevServerResult, ViteManager};
use cell_webp_proto::{WebPEncodeInput, WebPProcessor, WebPResult};
use facet::Facet;
use facet_value::Value;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

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
            rewrite_urls_in_css(css, path_map)
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
            rewrite_string_literals_in_js(js, path_map)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn minify_css<'a>(&'a self, css: String) -> HtmlCallbackFuture<'a> {
        Box::pin(async move {
            rewrite_urls_in_css(css, HashMap::new())
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn minify_js<'a>(&'a self, js: String) -> HtmlCallbackFuture<'a> {
        Box::pin(async move {
            rewrite_string_literals_in_js(js, HashMap::new())
                .await
                .map_err(|e| e.to_string())
        })
    }
}

pub(crate) fn html_processor() -> ddc_cell_html::HtmlProcessorImpl<DodecaHtmlCallbacks> {
    ddc_cell_html::HtmlProcessorImpl::with_callbacks(DodecaHtmlCallbacks)
}

pub(crate) fn template_renderer()
-> ddc_cell_gingembre::TemplateRendererImpl<crate::template_host::TemplateHostImpl> {
    ddc_cell_gingembre::TemplateRendererImpl::new(crate::template_host::TemplateHostImpl::new())
}

static DIRECT_CALL_ID: AtomicU64 = AtomicU64::new(1);

fn next_direct_call_id() -> u64 {
    DIRECT_CALL_ID.fetch_add(1, Ordering::Relaxed)
}

/// Provide the SiteServer for local HTTP serving.
/// This must be called before the HTTP router needs to serve content.
/// For build-only commands, this can be skipped.
pub fn provide_site_server(server: Arc<SiteServer>) {
    crate::host::Host::get().provide_site_server(server);
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
    let call_id = next_direct_call_id();
    let started_at = Instant::now();
    tracing::debug!(
        call_id,
        cell = "gingembre",
        method = "render",
        context_id = context_id.0,
        template_name,
        "template render starting"
    );
    let result = template_renderer()
        .render(context_id, template_name.to_string(), initial_context)
        .await;
    tracing::debug!(
        call_id,
        cell = "gingembre",
        method = "render",
        elapsed_ms = started_at.elapsed().as_millis(),
        "template render complete"
    );
    Ok(result)
}

pub async fn eval_expression(
    context_id: ContextId,
    expression: &str,
    context: Value,
) -> eyre::Result<cell_gingembre_proto::EvalResult> {
    let call_id = next_direct_call_id();
    let started_at = Instant::now();
    tracing::debug!(
        call_id,
        cell = "gingembre",
        method = "eval_expression",
        context_id = context_id.0,
        expression,
        "template expression eval starting"
    );
    let result = template_renderer()
        .eval_expression(context_id, expression.to_string(), context)
        .await;
    tracing::debug!(
        call_id,
        cell = "gingembre",
        method = "eval_expression",
        elapsed_ms = started_at.elapsed().as_millis(),
        "template expression eval complete"
    );
    Ok(result)
}

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

/// Build the full-text search index from rendered pages.
pub async fn build_search_index(pages: Vec<SearchPage>) -> Result<Vec<SearchFile>, eyre::Error> {
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

pub async fn select_dialog(prompt: String, items: Vec<String>) -> SelectResult {
    ddc_cell_dialoguer::DialoguerImpl
        .select(prompt, items)
        .await
}

pub async fn start_vite_dev_server(project_dir: String) -> StartDevServerResult {
    ddc_cell_vite::ViteManagerImpl
        .start_dev_server(project_dir)
        .await
}

pub async fn run_vite_build(project_dir: String) -> RunBuildResult {
    ddc_cell_vite::ViteManagerImpl.run_build(project_dir).await
}

pub async fn diff_html(input: DiffInput) -> Result<DiffOutcome, DiffError> {
    ddc_cell_html_diff::HtmlDifferImpl.diff_html(input).await
}

pub async fn minify_html(html: String) -> MinifyResult {
    ddc_cell_minify::MinifierImpl.minify_html(html).await
}

/// Embed texts into unit vectors via the embed cell (Model2Vec; loads the model
/// once on first use). Output order matches input order.
pub async fn embed(texts: Vec<String>) -> cell_embed_proto::EmbedResult {
    use cell_embed_proto::Embedder;
    ddc_cell_embed::EmbedderImpl.embed(texts).await
}

pub async fn process_html(input: HtmlProcessInput) -> HtmlProcessResult {
    html_processor().process(input).await
}

/// Result of link checking - wrapper for internal use
#[derive(Debug, Clone)]
pub struct UrlCheckResult {
    pub statuses: std::collections::HashMap<String, LinkStatus>,
}

pub async fn check_urls(urls: Vec<String>, options: CheckOptions) -> Option<UrlCheckResult> {
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

pub async fn highlight_code(lang: &str, code: &str) -> Result<String, eyre::Error> {
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

pub async fn parse_and_render_markdown(
    source_path: &str,
    content: &str,
    source_map: bool,
    render_notes: bool,
) -> Result<cell_markdown_proto::ParseResult, MarkdownParseError> {
    let call_id = next_direct_call_id();
    let started_at = Instant::now();
    tracing::debug!(
        call_id,
        cell = "markdown",
        method = "parse_and_render",
        source_path,
        source_len = content.len(),
        source_map,
        render_notes,
        "markdown render starting"
    );
    let result = ddc_cell_markdown::MarkdownProcessorImpl::new()
        .parse_and_render(
            source_path.to_string(),
            content.to_string(),
            source_map,
            render_notes,
        )
        .await;
    match result {
        result @ cell_markdown_proto::ParseResult::Success { .. } => {
            tracing::debug!(
                call_id,
                cell = "markdown",
                method = "parse_and_render",
                elapsed_ms = started_at.elapsed().as_millis(),
                "markdown render complete"
            );
            Ok(result)
        }
        cell_markdown_proto::ParseResult::Error { message } => {
            tracing::error!(
                call_id,
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

pub async fn inject_code_buttons(
    html: String,
    code_metadata: HashMap<String, cell_html_proto::CodeExecutionMetadata>,
) -> Result<(String, bool), eyre::Error> {
    let call_id = next_direct_call_id();
    let started_at = Instant::now();
    tracing::debug!(
        call_id,
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
                call_id,
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
                call_id,
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
                call_id,
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

pub async fn load_data(content: String, format: DataFormat) -> LoadDataResult {
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

pub async fn rewrite_string_literals_in_js(
    js: String,
    path_map: HashMap<String, String>,
) -> Result<String, eyre::Error> {
    let input = JsRewriteInput { js, path_map };
    ddc_cell_js::JsProcessorImpl
        .rewrite_string_literals(input)
        .await
        .map_err(|e| eyre::eyre!("JS rewrite error: {e}"))
}

pub async fn rewrite_urls_in_css(
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

pub async fn decompress_font(data: Vec<u8>) -> Result<Vec<u8>, eyre::Error> {
    match ddc_cell_fonts::FontProcessorImpl
        .decompress_font(data)
        .await
    {
        FontResult::DecompressSuccess { data } => Ok(data),
        FontResult::Error { message } => Err(eyre::eyre!(message)),
        other => Err(eyre::eyre!("Unexpected result: {:?}", other)),
    }
}

pub async fn compress_to_woff2(data: Vec<u8>) -> Result<Vec<u8>, eyre::Error> {
    match ddc_cell_fonts::FontProcessorImpl
        .compress_to_woff2(data)
        .await
    {
        FontResult::CompressSuccess { data } => Ok(data),
        FontResult::Error { message } => Err(eyre::eyre!(message)),
        other => Err(eyre::eyre!("Unexpected result: {:?}", other)),
    }
}

// Image decoding/encoding helpers.
// These return Option to match what image.rs expects
pub async fn decode_png(data: &[u8]) -> Option<DecodedImage> {
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

pub async fn decode_jpeg(data: &[u8]) -> Option<DecodedImage> {
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

pub async fn decode_gif(data: &[u8]) -> Option<DecodedImage> {
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

pub async fn decode_webp(data: &[u8]) -> Option<DecodedImage> {
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

pub async fn decode_jxl(data: &[u8]) -> Option<DecodedImage> {
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

pub async fn resize_image(
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

pub async fn generate_thumbhash(pixels: &[u8], width: u32, height: u32) -> Option<String> {
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

pub async fn encode_webp(pixels: &[u8], width: u32, height: u32, quality: u8) -> Option<Vec<u8>> {
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

pub async fn encode_jxl(pixels: &[u8], width: u32, height: u32, quality: u8) -> Option<Vec<u8>> {
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

pub async fn compile_sass(
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
