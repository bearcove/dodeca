//! Plugin loading and management for dodeca.
//!
//! Plugins are loaded from dynamic libraries (.so on Linux, .dylib on macOS).
//! Currently supports image encoding/decoding plugins (WebP, JXL).

use plugcard::Plugin;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing::{debug, info, warn};

/// Decoded image data returned by plugins
#[derive(serde::Deserialize)]
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
}

impl PluginRegistry {
    /// Load plugins from a directory.
    fn load_from_dir(dir: &Path) -> Self {
        let webp = Self::try_load_plugin(dir, "dodeca_webp");
        let jxl = Self::try_load_plugin(dir, "dodeca_jxl");

        PluginRegistry { webp, jxl }
    }

    fn try_load_plugin(dir: &Path, name: &str) -> Option<Plugin> {
        let lib_name = format!("lib{name}.so");
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
        // 2. In target/debug (for development)
        // 3. In target/release

        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));

        #[cfg(debug_assertions)]
        let profile_dir = PathBuf::from("target/debug");
        #[cfg(not(debug_assertions))]
        let profile_dir = PathBuf::from("target/release");

        let search_paths: Vec<PathBuf> = [exe_dir, Some(profile_dir)]
            .into_iter()
            .flatten()
            .collect();

        for dir in &search_paths {
            let registry = PluginRegistry::load_from_dir(dir);
            if registry.webp.is_some() || registry.jxl.is_some() {
                info!("loaded plugins from {}", dir.display());
                return registry;
            }
        }

        debug!("no plugins found in search paths: {:?}", search_paths);
        PluginRegistry {
            webp: None,
            jxl: None,
        }
    })
}

/// Encode RGBA pixels to WebP using the plugin if available, otherwise return None.
pub fn encode_webp_plugin(pixels: &[u8], width: u32, height: u32, quality: u8) -> Option<Vec<u8>> {
    let plugin = plugins().webp.as_ref()?;

    // The plugin expects (Vec<u8>, u32, u32, u8) and returns Result<Vec<u8>, String>
    #[derive(serde::Serialize)]
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

    match plugin.call::<Input, Result<Vec<u8>, String>>("encode_webp", &input) {
        Ok(Ok(data)) => Some(data),
        Ok(Err(e)) => {
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

    #[derive(serde::Serialize)]
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

    match plugin.call::<Input, Result<Vec<u8>, String>>("encode_jxl", &input) {
        Ok(Ok(data)) => Some(data),
        Ok(Err(e)) => {
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

    #[derive(serde::Serialize)]
    struct Input {
        data: Vec<u8>,
    }

    let input = Input {
        data: data.to_vec(),
    };

    match plugin.call::<Input, Result<DecodedImage, String>>("decode_webp", &input) {
        Ok(Ok(decoded)) => Some(decoded),
        Ok(Err(e)) => {
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

    #[derive(serde::Serialize)]
    struct Input {
        data: Vec<u8>,
    }

    let input = Input {
        data: data.to_vec(),
    };

    match plugin.call::<Input, Result<DecodedImage, String>>("decode_jxl", &input) {
        Ok(Ok(decoded)) => Some(decoded),
        Ok(Err(e)) => {
            warn!("jxl decode plugin error: {}", e);
            None
        }
        Err(e) => {
            warn!("jxl decode plugin call failed: {}", e);
            None
        }
    }
}
