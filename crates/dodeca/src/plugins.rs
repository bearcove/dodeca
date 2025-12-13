//! Plugin loading and management for dodeca.
//!
//! Plugins are loaded from dynamic libraries (.so on Linux, .dylib on macOS).
//! Currently supports image encoding/decoding plugins (WebP, JXL).

use crate::plugin_server::ForwardingTracingSink;

use mod_arborium_proto::{HighlightResult, SyntaxHighlightServiceClient};
use mod_code_execution_proto::{
    CodeExecutionResult, CodeExecutorClient, ExecuteSamplesInput, ExtractSamplesInput,
};
use mod_css_proto::{CssProcessorClient, CssResult};
use mod_fonts_proto::{FontAnalysis, FontProcessorClient, FontResult, SubsetFontInput};
use mod_html_diff_proto::{DiffInput, DiffResult, HtmlDiffResult, HtmlDifferClient};
use mod_html_proto::HtmlProcessorClient;
use mod_image_proto::{ImageProcessorClient, ImageResult, ResizeInput, ThumbhashInput};
use mod_js_proto::{JsProcessorClient, JsResult, JsRewriteInput};
use mod_jxl_proto::{JXLEncodeInput, JXLProcessorClient, JXLResult};
use mod_linkcheck_proto::{LinkCheckInput, LinkCheckResult, LinkCheckerClient, LinkStatus};
use mod_minify_proto::{MinifierClient, MinifyResult};
use mod_pagefind_proto::{
    SearchFile, SearchIndexInput, SearchIndexResult, SearchIndexerClient, SearchPage,
};
use mod_sass_proto::{SassCompilerClient, SassInput, SassResult};
use mod_svgo_proto::{SvgoOptimizerClient, SvgoResult};
use mod_webp_proto::{WebPEncodeInput, WebPProcessorClient, WebPResult};
use rapace::RpcSession;
use rapace::transport::shm::{ShmSession, ShmSessionConfig, ShmTransport};
use rapace_tracing::{TracingConfigClient, TracingSinkServer};
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::process::Command;
use tracing::{debug, info, warn};

/// SHM configuration used for rapace plugins.
/// This must match the SHM_CONFIG in dodeca-plugin-runtime for plugins to connect.
const PLUGIN_SHM_CONFIG: ShmSessionConfig = ShmSessionConfig {
    ring_capacity: 1024, // 1024 descriptors in flight
    slot_size: 65536,    // 64KB per slot
    slot_count: 512,     // 512 slots = 32MB total
};

/// Decoded image data returned by plugins
pub type DecodedImage = mod_image_proto::DecodedImage;

/// Global plugin registry, initialized once.
static PLUGINS: OnceLock<PluginRegistry> = OnceLock::new();

macro_rules! define_plugins {
    ( $( $key:ident => $Client:ident ),* $(,)? ) => {
        #[derive(Default)]
        pub struct PluginRegistry {
            $(
                pub $key: Option<Arc<$Client<ShmTransport>>>,
            )*
        }

        impl PluginRegistry {
            /// Load plugins from a directory.
            fn load_from_dir(dir: &Path) -> Self {
                $(
                    let plugin_name = match stringify!($key) {
                        "syntax_highlight" => "dodeca-mod-arborium".to_string(),
                        other => format!("dodeca-mod-{}", other.replace('_', "-")),
                    };
                    let $key = Self::try_load_mod(dir, &plugin_name)
                        .map(|s| Arc::new($Client::<ShmTransport>::new(s)));
                )*

                PluginRegistry {
                    $($key),*
                }
            }
        }
    };
}

define_plugins! {
    webp            => WebPProcessorClient,
    jxl             => JXLProcessorClient,
    minify          => MinifierClient,
    svgo            => SvgoOptimizerClient,
    sass            => SassCompilerClient,
    css             => CssProcessorClient,
    js              => JsProcessorClient,
    pagefind        => SearchIndexerClient,
    image           => ImageProcessorClient,
    fonts           => FontProcessorClient,
    linkcheck       => LinkCheckerClient,
    code_execution  => CodeExecutorClient,
    html_diff       => HtmlDifferClient,
    html            => HtmlProcessorClient,
    syntax_highlight=> SyntaxHighlightServiceClient,
}

impl PluginRegistry {
    /// Try to load a rapace service from the given directory.
    fn try_load_mod(dir: &Path, binary_name: &str) -> Option<Arc<RpcSession<ShmTransport>>> {
        #[cfg(target_os = "windows")]
        let executable = format!("{binary_name}.exe");
        #[cfg(not(target_os = "windows"))]
        let executable = binary_name.to_string();

        let path = dir.join(&executable);
        if !path.exists() {
            debug!("rapace plugin not found: {}", path.display());
            return None;
        }

        let shm_path = env::temp_dir().join(format!("{binary_name}-{}.shm", std::process::id()));
        let _ = std::fs::remove_file(&shm_path);

        let session = match ShmSession::create_file(&shm_path, PLUGIN_SHM_CONFIG) {
            Ok(sess) => sess,
            Err(e) => {
                warn!("failed to create SHM for {} plugin: {}", binary_name, e);
                return None;
            }
        };

        let mut cmd = Command::new(&path);
        cmd.arg(format!("--shm-path={}", shm_path.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let mut child = match ur_taking_me_with_you::spawn_dying_with_parent(cmd) {
            Ok(child) => child,
            Err(e) => {
                warn!("failed to spawn {}: {}", executable, e);
                let _ = std::fs::remove_file(&shm_path);
                return None;
            }
        };

        let transport = Arc::new(ShmTransport::new(session));
        let rpc_session = Arc::new(RpcSession::with_channel_start(transport, 1));

        // Set up tracing sink so plugin logs are forwarded to host tracing
        let tracing_sink = ForwardingTracingSink::new();
        rpc_session.set_dispatcher(move |_channel_id, method_id, payload| {
            let tracing_sink = tracing_sink.clone();
            Box::pin(async move {
                let server = TracingSinkServer::new(tracing_sink);
                server.dispatch(method_id, &payload).await
            })
        });

        // Push the current RUST_LOG filter to the plugin
        {
            let rpc_session = rpc_session.clone();
            let plugin_label = binary_name.to_string();
            tokio::spawn(async move {
                let tracing_config_client = TracingConfigClient::new(rpc_session.clone());
                let filter_str = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
                if let Err(e) = tracing_config_client.set_filter(filter_str.clone()).await {
                    warn!("Failed to push filter to {} plugin: {:?}", plugin_label, e);
                } else {
                    debug!("Pushed filter to {} plugin: {}", plugin_label, filter_str);
                }
            });
        }

        {
            let session_runner = rpc_session.clone();
            let plugin_label = binary_name.to_string();
            tokio::spawn(async move {
                if let Err(e) = session_runner.run().await {
                    warn!("{} plugin RPC session error: {}", plugin_label, e);
                }
            });
        }

        let shm_cleanup = shm_path.clone();
        let plugin_label = binary_name.to_string();
        // Wait on the child in a blocking task (std::process::Child::wait is blocking)
        tokio::task::spawn_blocking(move || {
            if let Err(e) = child.wait() {
                warn!("{} plugin exited with error: {}", plugin_label, e);
            }
            let _ = std::fs::remove_file(&shm_cleanup);
        });

        info!("launched {} plugin from {}", binary_name, path.display());

        Some(rpc_session)
    }
}

/// Get the global plugin registry, initializing it if needed.
pub fn plugins() -> &'static PluginRegistry {
    PLUGINS.get_or_init(|| {
        // Look for plugins in several locations:
        // 1. DODECA_PLUGIN_PATH environment variable (highest priority)
        // 2. Next to the executable
        // 3. In plugins/ subdirectory next to executable (for installed releases)
        // 4. In target/debug (for development)
        // 5. In target/release

        let env_plugin_path = std::env::var("DODECA_PLUGIN_PATH").ok().map(PathBuf::from);

        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));

        let plugins_dir = exe_dir.as_ref().map(|p| p.join("plugins"));

        #[cfg(debug_assertions)]
        let profile_dir = PathBuf::from("target/debug");
        #[cfg(not(debug_assertions))]
        let profile_dir = PathBuf::from("target/release");

        let search_paths: Vec<PathBuf> = [env_plugin_path, exe_dir, plugins_dir, Some(profile_dir)]
            .into_iter()
            .flatten()
            .collect();

        for dir in &search_paths {
            let registry = PluginRegistry::load_from_dir(dir);
            // Consider the directory "active" if at least one plugin binary exists and loaded.
            if !matches!(
                registry,
                PluginRegistry {
                    webp: None,
                    jxl: None,
                    minify: None,
                    svgo: None,
                    sass: None,
                    css: None,
                    js: None,
                    pagefind: None,
                    image: None,
                    fonts: None,
                    linkcheck: None,
                    code_execution: None,
                    html_diff: None,
                    html: None,
                    syntax_highlight: None,
                }
            ) {
                info!("loaded plugins from {}", dir.display());
                return registry;
            }
        }

        debug!("no plugins found in search paths: {:?}", search_paths);
        Default::default()
    })
}

/// Encode RGBA pixels to WebP using the plugin if available, otherwise return None.
#[tracing::instrument(level = "debug", skip(pixels), fields(pixels_len = pixels.len()))]
pub async fn encode_webp_plugin(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Option<Vec<u8>> {
    let plugin = plugins().webp.as_ref()?;

    let input = WebPEncodeInput {
        pixels: pixels.to_vec(),
        width,
        height,
        quality,
    };

    match plugin.encode_webp(input).await {
        Ok(WebPResult::EncodeSuccess { data }) => Some(data),
        Ok(WebPResult::Error { message }) => {
            warn!("webp plugin error: {}", message);
            None
        }
        Ok(WebPResult::DecodeSuccess { .. }) => {
            warn!("webp plugin returned decode result for encode operation");
            None
        }
        Err(e) => {
            warn!("webp plugin call failed: {:?}", e);
            None
        }
    }
}

/// Encode RGBA pixels to JXL using the plugin if available, otherwise return None.
#[tracing::instrument(level = "debug", skip(pixels), fields(pixels_len = pixels.len()))]
pub async fn encode_jxl_plugin(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Option<Vec<u8>> {
    let plugin = plugins().jxl.as_ref()?;

    let input = JXLEncodeInput {
        pixels: pixels.to_vec(),
        width,
        height,
        quality,
    };

    match plugin.encode_jxl(input).await {
        Ok(JXLResult::EncodeSuccess { data }) => Some(data),
        Ok(JXLResult::Error { message }) => {
            warn!("jxl plugin error: {}", message);
            None
        }
        Ok(JXLResult::DecodeSuccess { .. }) => {
            warn!("jxl plugin returned decode result for encode operation");
            None
        }
        Err(e) => {
            warn!("jxl plugin call failed: {:?}", e);
            None
        }
    }
}

/// Decode WebP to pixels using the plugin.
#[tracing::instrument(level = "debug", skip(data), fields(data_len = data.len()))]
pub async fn decode_webp_plugin(data: &[u8]) -> Option<DecodedImage> {
    let plugin = plugins().webp.as_ref()?;

    match plugin.decode_webp(data.to_vec()).await {
        Ok(WebPResult::DecodeSuccess {
            pixels,
            width,
            height,
            channels,
        }) => Some(DecodedImage {
            pixels,
            width,
            height,
            channels,
        }),
        Ok(WebPResult::Error { message }) => {
            warn!("webp decode plugin error: {}", message);
            None
        }
        Ok(WebPResult::EncodeSuccess { .. }) => {
            warn!("webp plugin returned encode result for decode operation");
            None
        }
        Err(e) => {
            warn!("webp decode plugin call failed: {:?}", e);
            None
        }
    }
}

/// Decode JXL to pixels using the plugin.
#[tracing::instrument(level = "debug", skip(data), fields(data_len = data.len()))]
pub async fn decode_jxl_plugin(data: &[u8]) -> Option<DecodedImage> {
    let plugin = plugins().jxl.as_ref()?;

    match plugin.decode_jxl(data.to_vec()).await {
        Ok(JXLResult::DecodeSuccess {
            pixels,
            width,
            height,
            channels,
        }) => Some(DecodedImage {
            pixels,
            width,
            height,
            channels,
        }),
        Ok(JXLResult::Error { message }) => {
            warn!("jxl decode plugin error: {}", message);
            None
        }
        Ok(JXLResult::EncodeSuccess { .. }) => {
            warn!("jxl plugin returned encode result for decode operation");
            None
        }
        Err(e) => {
            warn!("jxl decode plugin call failed: {:?}", e);
            None
        }
    }
}

/// Minify HTML using the plugin.
///
/// # Panics
/// Panics if the minify plugin is not loaded.
#[tracing::instrument(level = "debug", skip(html), fields(html_len = html.len()))]
pub async fn minify_html_plugin(html: &str) -> Result<String, String> {
    let plugin = plugins()
        .minify
        .as_ref()
        .expect("dodeca-minify plugin not loaded");

    match plugin.minify_html(html.to_string()).await {
        Ok(MinifyResult::Success { content }) => Ok(content),
        Ok(MinifyResult::Error { message }) => Err(message),
        Err(e) => Err(format!("plugin call failed: {:?}", e)),
    }
}

/// Optimize SVG using the plugin.
///
/// # Panics
/// Panics if the svgo plugin is not loaded.
#[tracing::instrument(level = "debug", skip(svg), fields(svg_len = svg.len()))]
pub async fn optimize_svg_plugin(svg: &str) -> Result<String, String> {
    let plugin = plugins()
        .svgo
        .as_ref()
        .expect("dodeca-svgo plugin not loaded");

    match plugin.optimize_svg(svg.to_string()).await {
        Ok(SvgoResult::Success { svg }) => Ok(svg),
        Ok(SvgoResult::Error { message }) => Err(message),
        Err(e) => Err(format!("plugin call failed: {:?}", e)),
    }
}

/// Compile SASS/SCSS using the plugin.
///
/// # Panics
/// Panics if the sass plugin is not loaded.
#[tracing::instrument(level = "debug", skip(files), fields(num_files = files.len()))]
pub async fn compile_sass_plugin(
    files: &std::collections::HashMap<String, String>,
) -> Result<String, String> {
    let plugin = plugins()
        .sass
        .as_ref()
        .expect("dodeca-sass plugin not loaded");

    let input = SassInput {
        files: files.clone(),
    };

    match plugin.compile_sass(input).await {
        Ok(SassResult::Success { css }) => Ok(css),
        Ok(SassResult::Error { message }) => Err(message),
        Err(e) => Err(format!("plugin call failed: {:?}", e)),
    }
}

/// Rewrite URLs in CSS and minify using the plugin.
///
/// # Panics
/// Panics if the css plugin is not loaded.
#[tracing::instrument(level = "debug", skip(css, path_map), fields(css_len = css.len(), path_map_len = path_map.len()))]
pub async fn rewrite_urls_in_css_plugin(
    css: &str,
    path_map: &HashMap<String, String>,
) -> Result<String, String> {
    let plugin = plugins()
        .css
        .as_ref()
        .expect("dodeca-css plugin not loaded");

    match plugin
        .rewrite_and_minify(css.to_string(), path_map.clone())
        .await
    {
        Ok(CssResult::Success { css }) => Ok(css),
        Ok(CssResult::Error { message }) => Err(message),
        Err(e) => Err(format!("plugin call failed: {:?}", e)),
    }
}

/// Rewrite string literals in JS using the plugin.
///
/// # Panics
/// Panics if the js plugin is not loaded.
#[tracing::instrument(level = "debug", skip(js, path_map), fields(js_len = js.len(), path_map_len = path_map.len()))]
pub async fn rewrite_string_literals_in_js_plugin(
    js: &str,
    path_map: &HashMap<String, String>,
) -> Result<String, String> {
    let plugin = plugins().js.as_ref().expect("dodeca-js plugin not loaded");

    let input = JsRewriteInput {
        js: js.to_string(),
        path_map: path_map.clone(),
    };

    match plugin.rewrite_string_literals(input).await {
        Ok(JsResult::Success { js }) => Ok(js),
        Ok(JsResult::Error { message }) => Err(message),
        Err(e) => Err(format!("plugin call failed: {:?}", e)),
    }
}

/// Build a search index from HTML pages using the plugin.
///
/// # Panics
/// Panics if the pagefind plugin is not loaded.
#[tracing::instrument(level = "debug", skip(pages), fields(num_pages = pages.len()))]
pub async fn build_search_index_plugin(pages: Vec<SearchPage>) -> Result<Vec<SearchFile>, String> {
    let plugin = plugins()
        .pagefind
        .as_ref()
        .expect("dodeca-pagefind plugin not loaded");

    let input = SearchIndexInput { pages };

    match plugin.build_search_index(input).await {
        Ok(SearchIndexResult::Success { output }) => Ok(output.files),
        Ok(SearchIndexResult::Error { message }) => Err(message),
        Err(e) => Err(format!("plugin call failed: {:?}", e)),
    }
}

// ============================================================================
// Image processing plugin functions
// ============================================================================

/// Decode a PNG image using the plugin.
#[tracing::instrument(level = "debug", skip(data), fields(data_len = data.len()))]
pub async fn decode_png_plugin(data: &[u8]) -> Option<DecodedImage> {
    let plugin = plugins().image.as_ref()?;

    match plugin.decode_png(data.to_vec()).await {
        Ok(ImageResult::Success { image }) => Some(DecodedImage {
            pixels: image.pixels,
            width: image.width,
            height: image.height,
            channels: image.channels,
        }),
        Ok(ImageResult::Error { message }) => {
            warn!("png decode plugin error: {}", message);
            None
        }
        Ok(_) => {
            warn!("png plugin returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("png decode plugin call failed: {:?}", e);
            None
        }
    }
}

/// Decode a JPEG image using the plugin.
#[tracing::instrument(level = "debug", skip(data), fields(data_len = data.len()))]
pub async fn decode_jpeg_plugin(data: &[u8]) -> Option<DecodedImage> {
    let plugin = plugins().image.as_ref()?;

    match plugin.decode_jpeg(data.to_vec()).await {
        Ok(ImageResult::Success { image }) => Some(DecodedImage {
            pixels: image.pixels,
            width: image.width,
            height: image.height,
            channels: image.channels,
        }),
        Ok(ImageResult::Error { message }) => {
            warn!("jpeg decode plugin error: {}", message);
            None
        }
        Ok(_) => {
            warn!("jpeg plugin returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("jpeg decode plugin call failed: {:?}", e);
            None
        }
    }
}

/// Decode a GIF image using the plugin.
#[tracing::instrument(level = "debug", skip(data), fields(data_len = data.len()))]
pub async fn decode_gif_plugin(data: &[u8]) -> Option<DecodedImage> {
    let plugin = plugins().image.as_ref()?;

    match plugin.decode_gif(data.to_vec()).await {
        Ok(ImageResult::Success { image }) => Some(DecodedImage {
            pixels: image.pixels,
            width: image.width,
            height: image.height,
            channels: image.channels,
        }),
        Ok(ImageResult::Error { message }) => {
            warn!("gif decode plugin error: {}", message);
            None
        }
        Ok(_) => {
            warn!("gif plugin returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("gif decode plugin call failed: {:?}", e);
            None
        }
    }
}

/// Resize an image using the plugin.
#[tracing::instrument(level = "debug", skip(pixels), fields(pixels_len = pixels.len()))]
pub async fn resize_image_plugin(
    pixels: &[u8],
    width: u32,
    height: u32,
    channels: u8,
    target_width: u32,
) -> Option<DecodedImage> {
    let plugin = plugins().image.as_ref()?;

    let input = ResizeInput {
        pixels: pixels.to_vec(),
        width,
        height,
        channels,
        target_width,
    };

    match plugin.resize_image(input).await {
        Ok(ImageResult::Success { image }) => Some(DecodedImage {
            pixels: image.pixels,
            width: image.width,
            height: image.height,
            channels: image.channels,
        }),
        Ok(ImageResult::Error { message }) => {
            warn!("resize plugin error: {}", message);
            None
        }
        Ok(_) => {
            warn!("resize plugin returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("resize plugin call failed: {:?}", e);
            None
        }
    }
}

/// Generate a thumbhash data URL using the plugin.
#[tracing::instrument(level = "debug", skip(pixels), fields(pixels_len = pixels.len()))]
pub async fn generate_thumbhash_plugin(pixels: &[u8], width: u32, height: u32) -> Option<String> {
    let plugin = plugins().image.as_ref()?;

    let input = ThumbhashInput {
        pixels: pixels.to_vec(),
        width,
        height,
    };

    match plugin.generate_thumbhash_data_url(input).await {
        Ok(ImageResult::ThumbhashSuccess { data_url }) => Some(data_url),
        Ok(ImageResult::Error { message }) => {
            warn!("thumbhash plugin error: {}", message);
            None
        }
        Ok(_) => {
            warn!("thumbhash plugin returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("thumbhash plugin call failed: {:?}", e);
            None
        }
    }
}

// ============================================================================
// Font processing plugin functions
// ============================================================================

/// Analyze HTML and CSS to collect font usage information.
///
/// # Panics
/// Panics if the fonts plugin is not loaded.
pub async fn analyze_fonts_plugin(html: &str, css: &str) -> FontAnalysis {
    let plugin = plugins()
        .fonts
        .as_ref()
        .expect("dodeca-fonts plugin not loaded");

    match plugin
        .analyze_fonts(html.to_string(), css.to_string())
        .await
    {
        Ok(FontResult::AnalysisSuccess { analysis }) => analysis,
        Ok(FontResult::Error { message }) => panic!("font analysis plugin error: {}", message),
        Ok(_) => panic!("font analysis plugin returned unexpected result type"),
        Err(e) => panic!("font analysis plugin call failed: {:?}", e),
    }
}

/// Extract inline CSS from HTML (from <style> tags).
///
/// # Panics
/// Panics if the fonts plugin is not loaded.
pub async fn extract_css_from_html_plugin(html: &str) -> String {
    let plugin = plugins()
        .fonts
        .as_ref()
        .expect("dodeca-fonts plugin not loaded");

    match plugin.extract_css_from_html(html.to_string()).await {
        Ok(FontResult::CssSuccess { css }) => css,
        Ok(FontResult::Error { message }) => panic!("extract css plugin error: {}", message),
        Ok(_) => panic!("extract css plugin returned unexpected result type"),
        Err(e) => panic!("extract css plugin call failed: {:?}", e),
    }
}

/// Decompress a WOFF2/WOFF font to TTF.
pub async fn decompress_font_plugin(data: &[u8]) -> Option<Vec<u8>> {
    let plugin = plugins().fonts.as_ref()?;

    match plugin.decompress_font(data.to_vec()).await {
        Ok(FontResult::DecompressSuccess { data }) => Some(data),
        Ok(FontResult::Error { message }) => {
            warn!("decompress font plugin error: {}", message);
            None
        }
        Ok(_) => {
            warn!("decompress font plugin returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("decompress font plugin call failed: {:?}", e);
            None
        }
    }
}

/// Subset a font to only include specified characters.
pub async fn subset_font_plugin(data: &[u8], chars: &[char]) -> Option<Vec<u8>> {
    let plugin = plugins().fonts.as_ref()?;

    let input = SubsetFontInput {
        data: data.to_vec(),
        chars: chars.to_vec(),
    };

    match plugin.subset_font(input).await {
        Ok(FontResult::SubsetSuccess { data }) => Some(data),
        Ok(FontResult::Error { message }) => {
            warn!("subset font plugin error: {}", message);
            None
        }
        Ok(_) => {
            warn!("subset font plugin returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("subset font plugin call failed: {:?}", e);
            None
        }
    }
}

/// Compress TTF font data to WOFF2.
pub async fn compress_to_woff2_plugin(data: &[u8]) -> Option<Vec<u8>> {
    let plugin = plugins().fonts.as_ref()?;

    match plugin.compress_to_woff2(data.to_vec()).await {
        Ok(FontResult::CompressSuccess { data }) => Some(data),
        Ok(FontResult::Error { message }) => {
            warn!("compress to woff2 plugin error: {}", message);
            None
        }
        Ok(_) => {
            warn!("compress to woff2 plugin returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("compress to woff2 plugin call failed: {:?}", e);
            None
        }
    }
}

// ============================================================================
// Link checking plugin functions
// ============================================================================

/// Status of an external link check (from plugin)
/// Options for link checking
#[derive(Debug, Clone)]
pub struct CheckOptions {
    /// Domains to skip (e.g., ["localhost", "127.0.0.1"])
    #[allow(dead_code)] // TODO: pass to plugin
    pub skip_domains: Vec<String>,
    /// Rate limiting between requests (milliseconds)
    pub rate_limit_ms: u64,
    /// Timeout for each request (seconds)
    pub timeout_secs: u64,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            skip_domains: vec!["localhost".to_string(), "127.0.0.1".to_string()],
            rate_limit_ms: 1000,
            timeout_secs: 10,
        }
    }
}

/// Result of link checking
#[derive(Debug, Clone)]
pub struct CheckResult {
    /// Status for each URL (in same order as input)
    pub statuses: Vec<LinkStatus>,
    /// Number of URLs that were actually checked (not skipped)
    pub checked_count: u32,
}

/// Check external URLs using the linkcheck plugin.
///
/// Returns None if the plugin is not loaded.
pub async fn check_urls_plugin(urls: Vec<String>, options: CheckOptions) -> Option<CheckResult> {
    let plugin = plugins().linkcheck.as_ref()?;

    let input = LinkCheckInput {
        urls: urls.clone(),
        delay_ms: options.rate_limit_ms,
        timeout_secs: options.timeout_secs,
    };

    match plugin.check_links(input).await {
        Ok(LinkCheckResult::Success { output }) => {
            // Convert the HashMap results back to the expected format
            let statuses: Vec<LinkStatus> = urls
                .into_iter()
                .map(|url| {
                    output.results.get(&url).cloned().unwrap_or(LinkStatus {
                        status: "skipped".to_string(),
                        code: None,
                        message: Some("URL not in results".to_string()),
                    })
                })
                .collect();

            let checked_count = output.results.len() as u32;

            Some(CheckResult {
                statuses,
                checked_count,
            })
        }
        Ok(LinkCheckResult::Error { message }) => {
            warn!("linkcheck plugin error: {}", message);
            None
        }
        Err(e) => {
            warn!("linkcheck plugin call failed: {:?}", e);
            None
        }
    }
}

/// Check if the linkcheck plugin is available.
pub fn has_linkcheck_plugin() -> bool {
    plugins().linkcheck.is_some()
}

// ============================================================================
// Code execution plugin functions
// ============================================================================

/// Extract code samples from markdown using plugin.
///
/// Returns None if plugin is not loaded.
pub async fn extract_code_samples_plugin(
    content: &str,
    source_path: &str,
) -> Option<Vec<dodeca_code_execution_types::CodeSample>> {
    let plugin = plugins().code_execution.as_ref()?;

    let input = ExtractSamplesInput {
        source_path: source_path.to_string(),
        content: content.to_string(),
    };

    match plugin.extract_code_samples(input).await {
        Ok(CodeExecutionResult::ExtractSuccess { output }) => Some(output.samples),
        Ok(CodeExecutionResult::Error { message }) => {
            warn!("code execution plugin error: {}", message);
            None
        }
        Ok(_) => {
            warn!("code execution plugin returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("code execution plugin call failed: {:?}", e);
            None
        }
    }
}

/// Execute code samples using plugin.
///
/// Returns None if plugin is not loaded.
pub async fn execute_code_samples_plugin(
    samples: Vec<dodeca_code_execution_types::CodeSample>,
    config: dodeca_code_execution_types::CodeExecutionConfig,
) -> Option<
    Vec<(
        dodeca_code_execution_types::CodeSample,
        dodeca_code_execution_types::ExecutionResult,
    )>,
> {
    let plugin = plugins().code_execution.as_ref()?;

    let input = ExecuteSamplesInput { samples, config };

    match plugin.execute_code_samples(input).await {
        Ok(CodeExecutionResult::ExecuteSuccess { output }) => Some(output.results),
        Ok(CodeExecutionResult::Error { message }) => {
            warn!("code execution plugin error: {}", message);
            None
        }
        Ok(_) => {
            warn!("code execution plugin returned unexpected result type");
            None
        }
        Err(e) => {
            warn!("code execution plugin call failed: {:?}", e);
            None
        }
    }
}

/// Diff two HTML documents and produce patches using the plugin.
/// Returns None if the plugin is not loaded.
pub async fn diff_html_plugin(old_html: &str, new_html: &str) -> Option<DiffResult> {
    let plugin = plugins().html_diff.as_ref()?;

    let input = DiffInput {
        old_html: old_html.to_string(),
        new_html: new_html.to_string(),
    };

    match plugin.diff_html(input).await {
        Ok(HtmlDiffResult::Success { result }) => Some(result),
        Ok(HtmlDiffResult::Error { message }) => {
            warn!("html diff plugin error: {}", message);
            None
        }
        Err(e) => {
            warn!("html diff plugin call failed: {:?}", e);
            None
        }
    }
}

// ============================================================================
// HTML processing plugin functions
// ============================================================================

/// Rewrite URLs in HTML attributes (href, src, srcset) using the plugin.
///
/// Returns None if the html plugin is not loaded.
#[tracing::instrument(level = "debug", skip(html, path_map), fields(html_len = html.len(), path_map_len = path_map.len()))]
pub async fn rewrite_urls_in_html_plugin(
    html: &str,
    path_map: &HashMap<String, String>,
) -> Option<String> {
    let plugin = plugins().html.as_ref()?;

    match plugin.rewrite_urls(html.to_string(), path_map.clone()).await {
        Ok(mod_html_proto::HtmlResult::Success { html }) => Some(html),
        Ok(mod_html_proto::HtmlResult::SuccessWithFlag { html, .. }) => Some(html),
        Ok(mod_html_proto::HtmlResult::Error { message }) => {
            warn!("html rewrite_urls plugin error: {}", message);
            None
        }
        Err(e) => {
            warn!("html rewrite_urls plugin call failed: {:?}", e);
            None
        }
    }
}

/// Mark dead internal links in HTML using the plugin.
///
/// Returns (modified_html, had_dead_links) or None if plugin not loaded.
#[tracing::instrument(level = "debug", skip(html, known_routes), fields(html_len = html.len(), routes_count = known_routes.len()))]
pub async fn mark_dead_links_plugin(
    html: &str,
    known_routes: &std::collections::HashSet<String>,
) -> Option<(String, bool)> {
    let plugin = plugins().html.as_ref()?;

    match plugin.mark_dead_links(html.to_string(), known_routes.clone()).await {
        Ok(mod_html_proto::HtmlResult::SuccessWithFlag { html, flag }) => Some((html, flag)),
        Ok(mod_html_proto::HtmlResult::Success { html }) => Some((html, false)),
        Ok(mod_html_proto::HtmlResult::Error { message }) => {
            warn!("html mark_dead_links plugin error: {}", message);
            None
        }
        Err(e) => {
            warn!("html mark_dead_links plugin call failed: {:?}", e);
            None
        }
    }
}

/// Inject build info buttons into code blocks using the plugin.
///
/// Returns (modified_html, had_buttons) or None if plugin not loaded.
#[tracing::instrument(level = "debug", skip(html, code_metadata), fields(html_len = html.len(), metadata_count = code_metadata.len()))]
pub async fn inject_build_info_plugin(
    html: &str,
    code_metadata: &HashMap<String, mod_html_proto::CodeExecutionMetadata>,
) -> Option<(String, bool)> {
    let plugin = plugins().html.as_ref()?;

    match plugin.inject_build_info(html.to_string(), code_metadata.clone()).await {
        Ok(mod_html_proto::HtmlResult::SuccessWithFlag { html, flag }) => Some((html, flag)),
        Ok(mod_html_proto::HtmlResult::Success { html }) => Some((html, false)),
        Ok(mod_html_proto::HtmlResult::Error { message }) => {
            warn!("html inject_build_info plugin error: {}", message);
            None
        }
        Err(e) => {
            warn!("html inject_build_info plugin call failed: {:?}", e);
            None
        }
    }
}

// ============================================================================
// Syntax highlighting plugin functions
// ============================================================================

/// Highlight source code using the rapace syntax highlight service.
///
/// Returns the code with syntax highlighting applied as HTML, or None if no service is available.
pub async fn highlight_code(code: &str, language: &str) -> Option<HighlightResult> {
    let client = syntax_highlight_client()?;
    let code_len = code.len();
    tracing::debug!(
        language = language,
        code_len = code_len,
        "highlight_code: sending RPC request"
    );
    match client
        .highlight_code(code.to_string(), language.to_string())
        .await
    {
        Ok(result) => {
            tracing::debug!(
                language = language,
                code_len = code_len,
                html_len = result.html.len(),
                "highlight_code: RPC success"
            );
            Some(result)
        }
        Err(e) => {
            warn!(
                language = language,
                code_len = code_len,
                error = %e,
                "highlight_code: RPC failed"
            );
            None
        }
    }
}

/// Get the syntax highlight service client, if available
fn syntax_highlight_client() -> Option<Arc<SyntaxHighlightServiceClient<ShmTransport>>> {
    plugins().syntax_highlight.clone()
}
