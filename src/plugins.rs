//! Plugin loading and management for dodeca.
//!
//! Plugins are loaded from dynamic libraries (.so on Linux, .dylib on macOS).
//! Currently supports image encoding/decoding plugins (WebP, JXL).

use facet::Facet;
use plugcard::{LoadError, Plugin, PlugResult};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing::{debug, info, warn};

/// Decoded image data returned by plugins
#[derive(Facet)]
pub struct DecodedImage {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub channels: u8,
}

/// Global plugin registry, initialized once.
static PLUGINS: OnceLock<PluginRegistry> = OnceLock::new();

/// Registry of loaded plugins.
pub struct PluginRegistry {
    /// WebP encoder plugin
    pub webp: Option<Plugin>,
    /// JPEG XL encoder plugin
    pub jxl: Option<Plugin>,
    /// HTML minification plugin
    pub minify: Option<Plugin>,
    /// SVG optimization plugin
    pub svgo: Option<Plugin>,
    /// SASS/SCSS compilation plugin
    pub sass: Option<Plugin>,
    /// CSS URL rewriting plugin
    pub css: Option<Plugin>,
    /// JS string literal rewriting plugin
    pub js: Option<Plugin>,
    /// Search indexing plugin
    pub pagefind: Option<Plugin>,
    /// Image processing plugin (decode, resize, thumbhash)
    pub image: Option<Plugin>,
    /// Font processing plugin (analysis, subsetting, compression)
    pub fonts: Option<Plugin>,
}

impl PluginRegistry {
    /// Load plugins from a directory.
    fn load_from_dir(dir: &Path) -> Self {
        let webp = Self::try_load_plugin(dir, "dodeca_webp");
        let jxl = Self::try_load_plugin(dir, "dodeca_jxl");
        let minify = Self::try_load_plugin(dir, "dodeca_minify");
        let svgo = Self::try_load_plugin(dir, "dodeca_svgo");
        let sass = Self::try_load_plugin(dir, "dodeca_sass");
        let css = Self::try_load_plugin(dir, "dodeca_css");
        let js = Self::try_load_plugin(dir, "dodeca_js");
        let pagefind = Self::try_load_plugin(dir, "dodeca_pagefind");
        let image = Self::try_load_plugin(dir, "dodeca_image");
        let fonts = Self::try_load_plugin(dir, "dodeca_fonts");

        PluginRegistry { webp, jxl, minify, svgo, sass, css, js, pagefind, image, fonts }
    }

    /// Check if any plugins were loaded
    fn has_any(&self) -> bool {
        self.webp.is_some()
            || self.jxl.is_some()
            || self.minify.is_some()
            || self.svgo.is_some()
            || self.sass.is_some()
            || self.css.is_some()
            || self.js.is_some()
            || self.pagefind.is_some()
            || self.image.is_some()
            || self.fonts.is_some()
    }

    fn try_load_plugin(dir: &Path, name: &str) -> Option<Plugin> {
        #[cfg(target_os = "macos")]
        let lib_name = format!("lib{name}.dylib");
        #[cfg(target_os = "linux")]
        let lib_name = format!("lib{name}.so");
        #[cfg(target_os = "windows")]
        let lib_name = format!("{name}.dll");

        let path = dir.join(&lib_name);

        if !path.exists() {
            debug!("plugin not found: {}", path.display());
            return None;
        }

        match unsafe { Plugin::load(&path) } {
            Ok(plugin) => {
                let methods: Vec<_> = plugin.methods().map(|m| m.name).collect();
                info!("loaded plugin {} with methods: {:?}", lib_name, methods);
                Some(plugin)
            }
            Err(LoadError::AbiMismatch { expected, found }) => {
                warn!(
                    "plugin {} has incompatible ABI version (expected 0x{:08x}, found 0x{:08x}) - rebuild the plugin",
                    lib_name, expected, found
                );
                None
            }
            Err(LoadError::NoAbiVersion) => {
                warn!(
                    "plugin {} was built with an old plugcard version - rebuild the plugin",
                    lib_name
                );
                None
            }
            Err(e) => {
                warn!("failed to load plugin {}: {}", lib_name, e);
                None
            }
        }
    }
}

/// Get the global plugin registry, initializing it if needed.
pub fn plugins() -> &'static PluginRegistry {
    PLUGINS.get_or_init(|| {
        // Look for plugins in several locations:
        // 1. Next to the executable
        // 2. In plugins/ subdirectory next to executable (for installed releases)
        // 3. In target/debug (for development)
        // 4. In target/release

        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));

        let plugins_dir = exe_dir.as_ref().map(|p| p.join("plugins"));

        #[cfg(debug_assertions)]
        let profile_dir = PathBuf::from("target/debug");
        #[cfg(not(debug_assertions))]
        let profile_dir = PathBuf::from("target/release");

        let search_paths: Vec<PathBuf> = [exe_dir, plugins_dir, Some(profile_dir)]
            .into_iter()
            .flatten()
            .collect();

        for dir in &search_paths {
            let registry = PluginRegistry::load_from_dir(dir);
            if registry.has_any() {
                info!("loaded plugins from {}", dir.display());
                return registry;
            }
        }

        debug!("no plugins found in search paths: {:?}", search_paths);
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
        }
    })
}

/// Encode RGBA pixels to WebP using the plugin if available, otherwise return None.
pub fn encode_webp_plugin(pixels: &[u8], width: u32, height: u32, quality: u8) -> Option<Vec<u8>> {
    let plugin = plugins().webp.as_ref()?;

    // The plugin expects (Vec<u8>, u32, u32, u8) and returns PlugResult<Vec<u8>>
    #[derive(Facet)]
    struct Input {
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        quality: u8,
    }

    let input = Input {
        pixels: pixels.to_vec(),
        width,
        height,
        quality,
    };

    match plugin.call::<Input, PlugResult<Vec<u8>>>("encode_webp", &input) {
        Ok(PlugResult::Ok(data)) => Some(data),
        Ok(PlugResult::Err(e)) => {
            warn!("webp plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("webp plugin call failed: {}", e);
            None
        }
    }
}

/// Encode RGBA pixels to JXL using the plugin if available, otherwise return None.
pub fn encode_jxl_plugin(pixels: &[u8], width: u32, height: u32, quality: u8) -> Option<Vec<u8>> {
    let plugin = plugins().jxl.as_ref()?;

    #[derive(Facet)]
    struct Input {
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        quality: u8,
    }

    let input = Input {
        pixels: pixels.to_vec(),
        width,
        height,
        quality,
    };

    match plugin.call::<Input, PlugResult<Vec<u8>>>("encode_jxl", &input) {
        Ok(PlugResult::Ok(data)) => Some(data),
        Ok(PlugResult::Err(e)) => {
            warn!("jxl plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("jxl plugin call failed: {}", e);
            None
        }
    }
}

/// Decode WebP to pixels using the plugin.
pub fn decode_webp_plugin(data: &[u8]) -> Option<DecodedImage> {
    let plugin = plugins().webp.as_ref()?;

    #[derive(Facet)]
    struct Input {
        data: Vec<u8>,
    }

    let input = Input {
        data: data.to_vec(),
    };

    match plugin.call::<Input, PlugResult<DecodedImage>>("decode_webp", &input) {
        Ok(PlugResult::Ok(decoded)) => Some(decoded),
        Ok(PlugResult::Err(e)) => {
            warn!("webp decode plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("webp decode plugin call failed: {}", e);
            None
        }
    }
}

/// Decode JXL to pixels using the plugin.
pub fn decode_jxl_plugin(data: &[u8]) -> Option<DecodedImage> {
    let plugin = plugins().jxl.as_ref()?;

    #[derive(Facet)]
    struct Input {
        data: Vec<u8>,
    }

    let input = Input {
        data: data.to_vec(),
    };

    match plugin.call::<Input, PlugResult<DecodedImage>>("decode_jxl", &input) {
        Ok(PlugResult::Ok(decoded)) => Some(decoded),
        Ok(PlugResult::Err(e)) => {
            warn!("jxl decode plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("jxl decode plugin call failed: {}", e);
            None
        }
    }
}

/// Minify HTML using the plugin.
///
/// # Panics
/// Panics if the minify plugin is not loaded.
pub fn minify_html_plugin(html: &str) -> Result<String, String> {
    let plugin = plugins()
        .minify
        .as_ref()
        .expect("dodeca-minify plugin not loaded");

    match plugin.call::<String, PlugResult<String>>("minify_html", &html.to_string()) {
        Ok(PlugResult::Ok(minified)) => Ok(minified),
        Ok(PlugResult::Err(e)) => Err(e),
        Err(e) => Err(format!("plugin call failed: {}", e)),
    }
}

/// Optimize SVG using the plugin.
///
/// # Panics
/// Panics if the svgo plugin is not loaded.
pub fn optimize_svg_plugin(svg: &str) -> Result<String, String> {
    let plugin = plugins()
        .svgo
        .as_ref()
        .expect("dodeca-svgo plugin not loaded");

    match plugin.call::<String, PlugResult<String>>("optimize_svg", &svg.to_string()) {
        Ok(PlugResult::Ok(optimized)) => Ok(optimized),
        Ok(PlugResult::Err(e)) => Err(e),
        Err(e) => Err(format!("plugin call failed: {}", e)),
    }
}

/// Input for SASS compilation
#[derive(Facet)]
struct SassInput {
    files: std::collections::HashMap<String, String>,
}

/// Compile SASS/SCSS using the plugin.
///
/// # Panics
/// Panics if the sass plugin is not loaded.
pub fn compile_sass_plugin(
    files: &std::collections::HashMap<String, String>,
) -> Result<String, String> {
    let plugin = plugins()
        .sass
        .as_ref()
        .expect("dodeca-sass plugin not loaded");

    let input = SassInput {
        files: files.clone(),
    };

    match plugin.call::<SassInput, PlugResult<String>>("compile_sass", &input) {
        Ok(PlugResult::Ok(css)) => Ok(css),
        Ok(PlugResult::Err(e)) => Err(e),
        Err(e) => Err(format!("plugin call failed: {}", e)),
    }
}

/// Input for CSS URL rewriting
#[derive(Facet)]
struct CssRewriteInput {
    css: String,
    path_map: std::collections::HashMap<String, String>,
}

/// Rewrite URLs in CSS and minify using the plugin.
///
/// # Panics
/// Panics if the css plugin is not loaded.
pub fn rewrite_urls_in_css_plugin(
    css: &str,
    path_map: &std::collections::HashMap<String, String>,
) -> Result<String, String> {
    let plugin = plugins()
        .css
        .as_ref()
        .expect("dodeca-css plugin not loaded");

    let input = CssRewriteInput {
        css: css.to_string(),
        path_map: path_map.clone(),
    };

    match plugin.call::<CssRewriteInput, PlugResult<String>>("rewrite_urls_in_css", &input) {
        Ok(PlugResult::Ok(result)) => Ok(result),
        Ok(PlugResult::Err(e)) => Err(e),
        Err(e) => Err(format!("plugin call failed: {}", e)),
    }
}

/// Input for JS string literal rewriting
#[derive(Facet)]
struct JsRewriteInput {
    js: String,
    path_map: std::collections::HashMap<String, String>,
}

/// Rewrite string literals in JS using the plugin.
///
/// # Panics
/// Panics if the js plugin is not loaded.
pub fn rewrite_string_literals_in_js_plugin(
    js: &str,
    path_map: &std::collections::HashMap<String, String>,
) -> Result<String, String> {
    let plugin = plugins()
        .js
        .as_ref()
        .expect("dodeca-js plugin not loaded");

    let input = JsRewriteInput {
        js: js.to_string(),
        path_map: path_map.clone(),
    };

    match plugin.call::<JsRewriteInput, PlugResult<String>>("rewrite_string_literals_in_js", &input)
    {
        Ok(PlugResult::Ok(result)) => Ok(result),
        Ok(PlugResult::Err(e)) => Err(e),
        Err(e) => Err(format!("plugin call failed: {}", e)),
    }
}

/// A page to be indexed for search
#[derive(Facet)]
pub struct SearchPage {
    pub url: String,
    pub html: String,
}

/// Output file from search indexing
#[derive(Facet)]
pub struct SearchFile {
    pub path: String,
    pub contents: Vec<u8>,
}

/// Input for building search index
#[derive(Facet)]
struct SearchIndexInput {
    pages: Vec<SearchPage>,
}

/// Output from building search index
#[derive(Facet)]
struct SearchIndexOutput {
    files: Vec<SearchFile>,
}

/// Build a search index from HTML pages using the plugin.
///
/// # Panics
/// Panics if the pagefind plugin is not loaded.
pub fn build_search_index_plugin(pages: Vec<SearchPage>) -> Result<Vec<SearchFile>, String> {
    let plugin = plugins()
        .pagefind
        .as_ref()
        .expect("dodeca-pagefind plugin not loaded");

    let input = SearchIndexInput { pages };

    match plugin.call::<SearchIndexInput, PlugResult<SearchIndexOutput>>("build_search_index", &input) {
        Ok(PlugResult::Ok(output)) => Ok(output.files),
        Ok(PlugResult::Err(e)) => Err(e),
        Err(e) => Err(format!("plugin call failed: {}", e)),
    }
}

// ============================================================================
// Image processing plugin functions
// ============================================================================

/// Decode a PNG image using the plugin.
pub fn decode_png_plugin(data: &[u8]) -> Option<DecodedImage> {
    let plugin = plugins().image.as_ref()?;

    #[derive(Facet)]
    struct Input {
        data: Vec<u8>,
    }

    let input = Input { data: data.to_vec() };

    match plugin.call::<Input, PlugResult<DecodedImage>>("decode_png", &input) {
        Ok(PlugResult::Ok(decoded)) => Some(decoded),
        Ok(PlugResult::Err(e)) => {
            warn!("png decode plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("png decode plugin call failed: {}", e);
            None
        }
    }
}

/// Decode a JPEG image using the plugin.
pub fn decode_jpeg_plugin(data: &[u8]) -> Option<DecodedImage> {
    let plugin = plugins().image.as_ref()?;

    #[derive(Facet)]
    struct Input {
        data: Vec<u8>,
    }

    let input = Input { data: data.to_vec() };

    match plugin.call::<Input, PlugResult<DecodedImage>>("decode_jpeg", &input) {
        Ok(PlugResult::Ok(decoded)) => Some(decoded),
        Ok(PlugResult::Err(e)) => {
            warn!("jpeg decode plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("jpeg decode plugin call failed: {}", e);
            None
        }
    }
}

/// Decode a GIF image using the plugin.
pub fn decode_gif_plugin(data: &[u8]) -> Option<DecodedImage> {
    let plugin = plugins().image.as_ref()?;

    #[derive(Facet)]
    struct Input {
        data: Vec<u8>,
    }

    let input = Input { data: data.to_vec() };

    match plugin.call::<Input, PlugResult<DecodedImage>>("decode_gif", &input) {
        Ok(PlugResult::Ok(decoded)) => Some(decoded),
        Ok(PlugResult::Err(e)) => {
            warn!("gif decode plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("gif decode plugin call failed: {}", e);
            None
        }
    }
}

/// Resize an image using the plugin.
pub fn resize_image_plugin(
    pixels: &[u8],
    width: u32,
    height: u32,
    channels: u8,
    target_width: u32,
) -> Option<DecodedImage> {
    let plugin = plugins().image.as_ref()?;

    #[derive(Facet)]
    struct Input {
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        channels: u8,
        target_width: u32,
    }

    let input = Input {
        pixels: pixels.to_vec(),
        width,
        height,
        channels,
        target_width,
    };

    match plugin.call::<Input, PlugResult<DecodedImage>>("resize_image", &input) {
        Ok(PlugResult::Ok(resized)) => Some(resized),
        Ok(PlugResult::Err(e)) => {
            warn!("resize plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("resize plugin call failed: {}", e);
            None
        }
    }
}

/// Generate a thumbhash data URL using the plugin.
pub fn generate_thumbhash_plugin(pixels: &[u8], width: u32, height: u32) -> Option<String> {
    let plugin = plugins().image.as_ref()?;

    #[derive(Facet)]
    struct Input {
        pixels: Vec<u8>,
        width: u32,
        height: u32,
    }

    let input = Input {
        pixels: pixels.to_vec(),
        width,
        height,
    };

    match plugin.call::<Input, PlugResult<String>>("generate_thumbhash_data_url", &input) {
        Ok(PlugResult::Ok(data_url)) => Some(data_url),
        Ok(PlugResult::Err(e)) => {
            warn!("thumbhash plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("thumbhash plugin call failed: {}", e);
            None
        }
    }
}

// ============================================================================
// Font processing plugin functions
// ============================================================================

/// A parsed @font-face rule
#[derive(Facet, Debug, Clone, PartialEq, Eq)]
pub struct FontFace {
    pub family: String,
    pub src: String,
    pub weight: Option<String>,
    pub style: Option<String>,
}

/// Result of analyzing CSS for font information
#[derive(Facet, Debug, Clone, PartialEq, Eq)]
pub struct FontAnalysis {
    /// Map of font-family name -> characters used
    pub chars_per_font: std::collections::HashMap<String, Vec<char>>,
    /// Parsed @font-face rules
    pub font_faces: Vec<FontFace>,
}

/// Analyze HTML and CSS to collect font usage information.
///
/// # Panics
/// Panics if the fonts plugin is not loaded.
pub fn analyze_fonts_plugin(html: &str, css: &str) -> FontAnalysis {
    let plugin = plugins()
        .fonts
        .as_ref()
        .expect("dodeca-fonts plugin not loaded");

    #[derive(Facet)]
    struct Input {
        html: String,
        css: String,
    }

    let input = Input {
        html: html.to_string(),
        css: css.to_string(),
    };

    match plugin.call::<Input, PlugResult<FontAnalysis>>("analyze_fonts", &input) {
        Ok(PlugResult::Ok(analysis)) => analysis,
        Ok(PlugResult::Err(e)) => panic!("font analysis plugin error: {}", e),
        Err(e) => panic!("font analysis plugin call failed: {}", e),
    }
}

/// Extract inline CSS from HTML (from <style> tags).
///
/// # Panics
/// Panics if the fonts plugin is not loaded.
pub fn extract_css_from_html_plugin(html: &str) -> String {
    let plugin = plugins()
        .fonts
        .as_ref()
        .expect("dodeca-fonts plugin not loaded");

    match plugin.call::<String, PlugResult<String>>("extract_css_from_html", &html.to_string()) {
        Ok(PlugResult::Ok(css)) => css,
        Ok(PlugResult::Err(e)) => panic!("extract css plugin error: {}", e),
        Err(e) => panic!("extract css plugin call failed: {}", e),
    }
}

/// Decompress a WOFF2/WOFF font to TTF.
pub fn decompress_font_plugin(data: &[u8]) -> Option<Vec<u8>> {
    let plugin = plugins().fonts.as_ref()?;

    #[derive(Facet)]
    struct Input {
        data: Vec<u8>,
    }

    let input = Input { data: data.to_vec() };

    match plugin.call::<Input, PlugResult<Vec<u8>>>("decompress_font", &input) {
        Ok(PlugResult::Ok(decompressed)) => Some(decompressed),
        Ok(PlugResult::Err(e)) => {
            warn!("decompress font plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("decompress font plugin call failed: {}", e);
            None
        }
    }
}

/// Subset a font to only include specified characters.
pub fn subset_font_plugin(data: &[u8], chars: &[char]) -> Option<Vec<u8>> {
    let plugin = plugins().fonts.as_ref()?;

    #[derive(Facet)]
    struct Input {
        data: Vec<u8>,
        chars: Vec<char>,
    }

    let input = Input {
        data: data.to_vec(),
        chars: chars.to_vec(),
    };

    match plugin.call::<Input, PlugResult<Vec<u8>>>("subset_font", &input) {
        Ok(PlugResult::Ok(subsetted)) => Some(subsetted),
        Ok(PlugResult::Err(e)) => {
            warn!("subset font plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("subset font plugin call failed: {}", e);
            None
        }
    }
}

/// Compress TTF font data to WOFF2.
pub fn compress_to_woff2_plugin(data: &[u8]) -> Option<Vec<u8>> {
    let plugin = plugins().fonts.as_ref()?;

    #[derive(Facet)]
    struct Input {
        data: Vec<u8>,
    }

    let input = Input { data: data.to_vec() };

    match plugin.call::<Input, PlugResult<Vec<u8>>>("compress_to_woff2", &input) {
        Ok(PlugResult::Ok(woff2)) => Some(woff2),
        Ok(PlugResult::Err(e)) => {
            warn!("compress to woff2 plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("compress to woff2 plugin call failed: {}", e);
            None
        }
    }
}
