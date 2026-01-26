//! Configuration file discovery and parsing
//!
//! Searches for `.config/dodeca.styx` walking up from the current directory.
//! The project root is the parent of `.config/`.
//!
//! Config parsing is delegated to the config cell to reduce monomorphization.

use camino::{Utf8Path, Utf8PathBuf};
use cell_config_proto::ParseConfigResult;
use eyre::{Result, eyre};
use std::env;
use std::fs;
use std::sync::OnceLock;

// Re-export config types from dodeca-config crate
pub use dodeca_config::{CodeExecutionConfig, DodecaConfig};

/// Configuration file names
const CONFIG_DIR: &str = ".config";
const CONFIG_FILE_STYX: &str = "dodeca.styx";
const CONFIG_FILE_YAML_LEGACY: &str = "dodeca.yaml";
const CONFIG_FILE_KDL_LEGACY: &str = "dodeca.kdl";

/// All project paths, derived from configuration
#[derive(Debug, Clone)]
pub struct ProjectPaths {
    /// Project root (where .config/ lives)
    pub root: Utf8PathBuf,
    /// Output directory (built site)
    pub output: Utf8PathBuf,
    /// Cache directory (.cache/)
    pub cache: Utf8PathBuf,
    /// Vite project directory (where vite.config.ts lives), if any
    pub vite: Option<Utf8PathBuf>,
    /// Vite dist output (vite_dir/dist), if vite exists
    pub vite_dist: Option<Utf8PathBuf>,
    /// Vite node_modules cache (vite_dir/node_modules/.vite), if vite exists
    pub vite_cache: Option<Utf8PathBuf>,
}

impl ProjectPaths {
    /// Create ProjectPaths from a ResolvedConfig
    pub fn from_config(config: &ResolvedConfig) -> Self {
        let root = config._root.clone();
        let content = &config.content_dir;
        let output = config.output_dir.clone();

        // cache is sibling of content
        let content_parent = content.parent().unwrap_or(&root);
        let cache = content_parent.join(".cache");

        // Find vite project - check content_parent first, then root, then common subdirs
        let vite = Self::find_vite_dir(&root, content_parent);
        let vite_dist = vite.as_ref().map(|v| v.join("dist"));
        let vite_cache = vite.as_ref().map(|v| v.join("node_modules/.vite"));

        Self {
            root,
            output,
            cache,
            vite,
            vite_dist,
            vite_cache,
        }
    }

    /// Find the Vite project directory
    fn find_vite_dir(root: &Utf8Path, content_parent: &Utf8Path) -> Option<Utf8PathBuf> {
        // Check content parent first (e.g., docs/)
        if Self::has_vite_config(content_parent) {
            return Some(content_parent.to_owned());
        }

        // Check root
        if Self::has_vite_config(root) {
            return Some(root.to_owned());
        }

        // Check common subdirectories from root
        for subdir in ["docs", "web", "frontend", "client", "site"] {
            let candidate = root.join(subdir);
            if Self::has_vite_config(&candidate) {
                return Some(candidate);
            }
        }

        None
    }

    /// Check if a directory has a Vite configuration file
    fn has_vite_config(dir: &Utf8Path) -> bool {
        dir.join("vite.config.ts").exists()
            || dir.join("vite.config.js").exists()
            || dir.join("vite.config.mts").exists()
            || dir.join("vite.config.mjs").exists()
    }

    /// Get the relative path of the vite directory from root, for display
    pub fn vite_prefix(&self) -> String {
        match &self.vite {
            Some(vite_dir) => {
                let rel = vite_dir.strip_prefix(&self.root).unwrap_or(vite_dir);
                if rel.as_str().is_empty() {
                    String::new()
                } else {
                    format!("{}/", rel)
                }
            }
            None => String::new(),
        }
    }
}

/// Discovered configuration with resolved paths
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Project root (parent of .config/)
    pub _root: Utf8PathBuf,
    /// Base URL for the site (e.g., `https://example.com` or `/` for local dev)
    pub base_url: String,
    /// Absolute path to content directory
    pub content_dir: Utf8PathBuf,
    /// Absolute path to output directory
    pub output_dir: Utf8PathBuf,
    /// Domains to skip during external link checking
    pub skip_domains: Vec<String>,
    /// Rate limit for external link checking (milliseconds between requests to same domain)
    pub rate_limit_ms: Option<u64>,
    /// Asset paths that should be served at original paths (no cache-busting)
    pub stable_assets: Vec<String>,
    /// Code execution configuration
    /// TODO: Pass this through to the picante query system instead of using default config
    #[allow(dead_code)]
    pub code_execution: CodeExecutionConfig,
    /// Generated CSS for light theme
    pub light_theme_css: String,
    /// Generated CSS for dark theme
    pub dark_theme_css: String,
    /// Build step definitions from config
    pub build_steps: Option<std::collections::HashMap<String, dodeca_config::BuildStepDef>>,
}

impl ResolvedConfig {
    /// Get all project paths derived from this config
    pub fn paths(&self) -> ProjectPaths {
        ProjectPaths::from_config(self)
    }
}

impl ResolvedConfig {
    /// Discover and load configuration from current directory
    pub async fn discover() -> Result<Option<Self>> {
        let config_path = find_config_file()?;

        match config_path {
            Some(path) => {
                let resolved = load_config(&path).await?;
                Ok(Some(resolved))
            }
            None => Ok(None),
        }
    }

    /// Discover and load configuration from a specific project path
    pub async fn discover_from(project_path: &Utf8Path) -> Result<Option<Self>> {
        let config_dir = project_path.join(CONFIG_DIR);

        // Check for legacy configs and error if found
        check_legacy_configs(&config_dir)?;

        let styx_file = config_dir.join(CONFIG_FILE_STYX);
        if styx_file.exists() {
            let resolved = load_config(&styx_file).await?;
            return Ok(Some(resolved));
        }

        Ok(None)
    }
}

/// Check for legacy config formats and return helpful error
fn check_legacy_configs(config_dir: &Utf8Path) -> Result<()> {
    let kdl_file = config_dir.join(CONFIG_FILE_KDL_LEGACY);
    if kdl_file.exists() {
        return Err(eyre!(
            "Found legacy configuration file: {}\n\n\
            KDL configuration format is no longer supported.\n\
            Please migrate to Styx format:\n\n\
            1. Rename {} to {}\n\
            2. Convert the content to Styx syntax\n\n\
            Example Styx config:\n\
            ```styx\n\
            content docs/\n\
            output public/\n\
            base_url https://example.com\n\
            ```",
            kdl_file,
            CONFIG_FILE_KDL_LEGACY,
            CONFIG_FILE_STYX
        ));
    }

    let yaml_file = config_dir.join(CONFIG_FILE_YAML_LEGACY);
    if yaml_file.exists() {
        return Err(eyre!(
            "Found legacy configuration file: {}\n\n\
            YAML configuration format is no longer supported.\n\
            Please migrate to Styx format:\n\n\
            1. Rename {} to {}\n\
            2. Convert the content to Styx syntax\n\n\
            Example Styx config:\n\
            ```styx\n\
            content docs/\n\
            output public/\n\
            base_url https://example.com\n\
            ```",
            yaml_file,
            CONFIG_FILE_YAML_LEGACY,
            CONFIG_FILE_STYX
        ));
    }

    Ok(())
}

/// Search for `.config/dodeca.styx` walking up from current directory
fn find_config_file() -> Result<Option<Utf8PathBuf>> {
    let cwd = env::current_dir()?;
    let cwd = Utf8PathBuf::try_from(cwd).map_err(|e| {
        eyre!(
            "Current directory is not valid UTF-8: {}",
            e.as_path().display()
        )
    })?;

    let mut current = cwd.as_path();

    loop {
        let config_dir = current.join(CONFIG_DIR);

        // Check for legacy configs and error if found
        check_legacy_configs(&config_dir)?;

        let styx_file = config_dir.join(CONFIG_FILE_STYX);
        if styx_file.exists() {
            return Ok(Some(styx_file));
        }

        match current.parent() {
            Some(parent) => current = parent,
            None => return Ok(None),
        }
    }
}

/// Load and resolve configuration from a config file path via the config cell
async fn load_config(config_path: &Utf8Path) -> Result<ResolvedConfig> {
    let content = fs::read_to_string(config_path)?;

    let client = crate::cells::config_cell()
        .await
        .ok_or_else(|| eyre!("Config cell not available"))?;

    let config: DodecaConfig = match client.parse_styx(content).await {
        Ok(ParseConfigResult::Success { config }) => *config,
        Ok(ParseConfigResult::Error { message }) => {
            return Err(eyre!("Failed to parse {}: {}", config_path, message));
        }
        Err(e) => {
            return Err(eyre!("RPC error parsing {}: {:?}", config_path, e));
        }
    };

    // Project root is the parent of .config/
    let config_dir = config_path
        .parent()
        .ok_or_else(|| eyre!("Config file has no parent directory"))?;
    let root = config_dir
        .parent()
        .ok_or_else(|| eyre!(".config directory has no parent"))?
        .to_owned();

    // Resolve paths relative to project root
    let content_dir = root.join(&config.content);
    let output_dir = root.join(&config.output);

    // Extract skip domains
    let skip_domains = config
        .link_check
        .as_ref()
        .and_then(|lc| lc.skip_domains.clone())
        .unwrap_or_default();

    // Extract rate limit
    let rate_limit_ms = config.link_check.as_ref().and_then(|lc| lc.rate_limit_ms);

    // Extract stable asset paths
    let stable_assets = config.stable_assets.unwrap_or_default();

    // Resolve theme names with defaults
    let light_theme_name = config
        .syntax_highlight
        .as_ref()
        .and_then(|sh| sh.light_theme.as_deref())
        .unwrap_or("github-light");
    let dark_theme_name = config
        .syntax_highlight
        .as_ref()
        .and_then(|sh| sh.dark_theme.as_deref())
        .unwrap_or("tokyo-night");

    // Generate CSS for both themes
    let light_theme_css = crate::theme_resolver::generate_theme_css(light_theme_name)
        .map_err(|e| eyre!("Failed to load light theme '{}': {}", light_theme_name, e))?;
    let dark_theme_css = crate::theme_resolver::generate_theme_css(dark_theme_name)
        .map_err(|e| eyre!("Failed to load dark theme '{}': {}", dark_theme_name, e))?;

    // Get base_url, defaulting to "/" for local development
    let base_url = config.base_url.unwrap_or_else(|| "/".to_string());

    Ok(ResolvedConfig {
        _root: root,
        base_url,
        content_dir,
        output_dir,
        skip_domains,
        rate_limit_ms,
        stable_assets,
        code_execution: config.code_execution.unwrap_or_default(),
        light_theme_css,
        dark_theme_css,
        build_steps: config.build_steps,
    })
}

// ============================================================================
// Global config access
// ============================================================================

/// Global resolved configuration
static RESOLVED_CONFIG: OnceLock<ResolvedConfig> = OnceLock::new();

/// Initialize the global config (call once at startup)
pub fn set_global_config(config: ResolvedConfig) -> Result<()> {
    // Initialize build step executor
    let executor = std::sync::Arc::new(crate::build_steps::BuildStepExecutor::new(
        config.build_steps.clone(),
        config._root.clone(),
    ));
    crate::host::Host::get().set_build_step_executor(executor);

    RESOLVED_CONFIG
        .set(config)
        .map_err(|_| eyre!("Global config already initialized"))
}

/// Get the global config (returns None if not initialized)
pub fn global_config() -> Option<&'static ResolvedConfig> {
    RESOLVED_CONFIG.get()
}

// Note: Config parsing tests are now handled by the config cell.
// The cell uses facet-styx for parsing.
