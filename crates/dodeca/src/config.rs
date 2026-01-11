//! Configuration file discovery and parsing
//!
//! Searches for `.config/dodeca.yaml` walking up from the current directory.
//! The project root is the parent of `.config/`.

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use facet::Facet;
use std::env;
use std::fs;
use std::sync::OnceLock;

// Re-export shared config types
pub use cell_code_execution_proto::CodeExecutionConfig;

/// Configuration file name
const CONFIG_DIR: &str = ".config";
const CONFIG_FILE_YAML: &str = "dodeca.yaml";
const CONFIG_FILE_KDL_LEGACY: &str = "dodeca.kdl";

/// Dodeca configuration from `.config/dodeca.yaml`
#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "snake_case")]
pub struct DodecaConfig {
    /// Base URL for the site (e.g., `https://example.com`)
    /// Used to generate permalinks. Defaults to "/" for local development.
    #[facet(default)]
    pub base_url: Option<String>,

    /// Content directory (relative to project root)
    pub content: String,

    /// Output directory (relative to project root)
    pub output: String,

    /// Link checking configuration
    #[facet(default)]
    pub link_check: Option<LinkCheckConfig>,

    /// Assets that should be served at their original paths (no cache-busting)
    /// e.g., favicon.svg, robots.txt, og-image.png
    #[facet(default)]
    pub stable_assets: Option<Vec<String>>,

    /// Code execution configuration
    #[facet(default)]
    pub code_execution: Option<CodeExecutionConfig>,

    /// Syntax highlighting theme configuration
    #[facet(default)]
    pub syntax_highlight: Option<SyntaxHighlightConfig>,
}

/// Syntax highlighting theme configuration
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct SyntaxHighlightConfig {
    /// Light theme name (e.g., "github-light", "catppuccin-latte")
    #[facet(default)]
    pub light_theme: Option<String>,

    /// Dark theme name (e.g., "tokyo-night", "catppuccin-mocha")
    #[facet(default)]
    pub dark_theme: Option<String>,
}

/// Link checking configuration
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct LinkCheckConfig {
    /// Domains to skip checking (anti-bot policies, known flaky, etc.)
    #[facet(default)]
    pub skip_domains: Option<Vec<String>>,

    /// Minimum delay between requests to the same domain (milliseconds)
    /// Default: 1000ms (1 second)
    #[facet(default)]
    pub rate_limit_ms: Option<u64>,
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
}

impl ResolvedConfig {
    /// Discover and load configuration from current directory
    pub fn discover() -> Result<Option<Self>> {
        let config_path = find_config_file()?;

        match config_path {
            Some(path) => {
                let resolved = load_config(&path)?;
                Ok(Some(resolved))
            }
            None => Ok(None),
        }
    }

    /// Discover and load configuration from a specific project path
    pub fn discover_from(project_path: &Utf8Path) -> Result<Option<Self>> {
        let config_dir = project_path.join(CONFIG_DIR);

        // Check for legacy KDL config and error if found
        let kdl_file = config_dir.join(CONFIG_FILE_KDL_LEGACY);
        if kdl_file.exists() {
            return Err(eyre!(
                "Found legacy configuration file: {}\n\n\
                KDL configuration format is no longer supported.\n\
                Please migrate to YAML format:\n\n\
                1. Rename {} to {}\n\
                2. Convert the content to YAML syntax\n\n\
                Example YAML config:\n\
                ```yaml\n\
                content: docs/\n\
                output: public/\n\
                base_url: https://example.com\n\
                ```",
                kdl_file,
                CONFIG_FILE_KDL_LEGACY,
                CONFIG_FILE_YAML
            ));
        }

        let yaml_file = config_dir.join(CONFIG_FILE_YAML);
        if yaml_file.exists() {
            let resolved = load_config(&yaml_file)?;
            Ok(Some(resolved))
        } else {
            Ok(None)
        }
    }
}

/// Search for `.config/dodeca.yaml` walking up from current directory
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

        // Check for legacy KDL config and error if found
        let kdl_file = config_dir.join(CONFIG_FILE_KDL_LEGACY);
        if kdl_file.exists() {
            return Err(eyre!(
                "Found legacy configuration file: {}\n\n\
                KDL configuration format is no longer supported.\n\
                Please migrate to YAML format:\n\n\
                1. Rename {} to {}\n\
                2. Convert the content to YAML syntax\n\n\
                Example YAML config:\n\
                ```yaml\n\
                content: docs/\n\
                output: public/\n\
                base_url: https://example.com\n\
                ```",
                kdl_file,
                CONFIG_FILE_KDL_LEGACY,
                CONFIG_FILE_YAML
            ));
        }

        let yaml_file = config_dir.join(CONFIG_FILE_YAML);
        if yaml_file.exists() {
            return Ok(Some(yaml_file));
        }

        match current.parent() {
            Some(parent) => current = parent,
            None => return Ok(None),
        }
    }
}

/// Load and resolve configuration from a config file path
fn load_config(config_path: &Utf8Path) -> Result<ResolvedConfig> {
    let content = fs::read_to_string(config_path)?;

    let config: DodecaConfig = facet_yaml::from_str(&content)
        .map_err(|e| eyre!("Failed to parse {}: {}", config_path, e))?;

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
    })
}

// ============================================================================
// Global config access
// ============================================================================

/// Global resolved configuration
static RESOLVED_CONFIG: OnceLock<ResolvedConfig> = OnceLock::new();

/// Initialize the global config (call once at startup)
pub fn set_global_config(config: ResolvedConfig) -> Result<()> {
    RESOLVED_CONFIG
        .set(config)
        .map_err(|_| eyre!("Global config already initialized"))
}

/// Get the global config (returns None if not initialized)
pub fn global_config() -> Option<&'static ResolvedConfig> {
    RESOLVED_CONFIG.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let yaml = r#"
content: docs/
output: public/
"#;

        let config: DodecaConfig = facet_yaml::from_str(yaml).unwrap();
        assert_eq!(config.content, "docs/");
        assert_eq!(config.output, "public/");
        assert!(config.stable_assets.is_none());
    }

    #[test]
    fn test_parse_full_config() {
        let yaml = r#"
content: docs/
output: public/
base_url: https://example.com
link_check:
  skip_domains:
    - example.com
    - test.local
  rate_limit_ms: 500
stable_assets:
  - favicon.svg
  - robots.txt
syntax_highlight:
  light_theme: github-light
  dark_theme: tokyo-night
"#;

        let config: DodecaConfig = facet_yaml::from_str(yaml).unwrap();
        assert_eq!(config.content, "docs/");
        assert_eq!(config.output, "public/");
        assert_eq!(config.base_url, Some("https://example.com".to_string()));
        assert_eq!(
            config.link_check.as_ref().unwrap().skip_domains,
            Some(vec!["example.com".to_string(), "test.local".to_string()])
        );
        assert_eq!(config.link_check.as_ref().unwrap().rate_limit_ms, Some(500));
        assert_eq!(
            config.stable_assets,
            Some(vec!["favicon.svg".to_string(), "robots.txt".to_string()])
        );
    }
}
