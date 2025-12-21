//! Configuration file discovery and parsing
//!
//! Searches for `.config/dodeca.kdl` walking up from the current directory.
//! The project root is the parent of `.config/`.

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use facet::Facet;
use facet_kdl as kdl;
use std::env;
use std::fs;

// Re-export shared config types
pub use dodeca_code_execution_types::KdlCodeExecutionConfig as CodeExecutionConfig;

/// Configuration file name
const CONFIG_DIR: &str = ".config";
const CONFIG_FILE: &str = "dodeca.kdl";

/// Dodeca configuration from `.config/dodeca.kdl`
#[derive(Debug, Clone, Facet)]
pub struct DodecaConfig {
    /// Content directory (relative to project root)
    #[facet(kdl::child)]
    pub content: ContentDir,

    /// Output directory (relative to project root)
    #[facet(kdl::child)]
    pub output: OutputDir,

    /// Link checking configuration
    #[facet(kdl::child, default)]
    pub link_check: LinkCheckConfig,

    /// Assets that should be served at their original paths (no cache-busting)
    /// e.g., favicon.svg, robots.txt, og-image.png
    #[facet(kdl::child, default)]
    pub stable_assets: StableAssetsConfig,

    /// Code execution configuration
    #[facet(kdl::child, default)]
    pub code_execution: CodeExecutionConfig,
}

/// Link checking configuration
#[derive(Debug, Clone, Default, Facet)]
#[facet(traits(Default))]
pub struct LinkCheckConfig {
    /// Domains to skip checking (anti-bot policies, known flaky, etc.)
    #[facet(kdl::children, default)]
    pub skip_domains: Vec<SkipDomain>,

    /// Minimum delay between requests to the same domain (milliseconds)
    /// Default: 1000ms (1 second)
    #[facet(kdl::property, default)]
    pub rate_limit_ms: Option<u64>,
}

/// Stable assets configuration (served at original paths without cache-busting)
#[derive(Debug, Clone, Default, Facet)]
#[facet(traits(Default))]
pub struct StableAssetsConfig {
    /// Asset paths relative to static/ directory
    #[facet(kdl::children, default)]
    pub paths: Vec<StableAssetPath>,
}

/// A single stable asset path
#[derive(Debug, Clone, Facet)]
pub struct StableAssetPath {
    #[facet(kdl::argument)]
    pub path: String,
}

/// A domain to skip during external link checking
#[derive(Debug, Clone, Facet)]
pub struct SkipDomain {
    #[facet(kdl::argument)]
    pub domain: String,
}

/// Content directory node
#[derive(Debug, Clone, Facet)]
pub struct ContentDir {
    #[facet(kdl::argument)]
    pub path: String,
}

/// Output directory node
#[derive(Debug, Clone, Facet)]
pub struct OutputDir {
    #[facet(kdl::argument)]
    pub path: String,
}

/// Discovered configuration with resolved paths
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Project root (parent of .config/)
    pub _root: Utf8PathBuf,
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
        let config_file = project_path.join(CONFIG_DIR).join(CONFIG_FILE);

        if config_file.exists() {
            let resolved = load_config(&config_file)?;
            Ok(Some(resolved))
        } else {
            Ok(None)
        }
    }
}

/// Search for `.config/dodeca.kdl` walking up from current directory
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
        let config_file = config_dir.join(CONFIG_FILE);

        if config_file.exists() {
            return Ok(Some(config_file));
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

    let config: DodecaConfig = kdl::from_str(&content).map_err(|e| {
        eyre!(
            "Failed to parse {}: {:?}",
            config_path,
            miette::Report::new(e)
        )
    })?;

    // Project root is the parent of .config/
    let config_dir = config_path
        .parent()
        .ok_or_else(|| eyre!("Config file has no parent directory"))?;
    let root = config_dir
        .parent()
        .ok_or_else(|| eyre!(".config directory has no parent"))?
        .to_owned();

    // Resolve paths relative to project root
    let content_dir = root.join(&config.content.path);
    let output_dir = root.join(&config.output.path);

    // Extract skip domains
    let skip_domains = config
        .link_check
        .skip_domains
        .into_iter()
        .map(|s| s.domain)
        .collect();

    // Extract rate limit
    let rate_limit_ms = config.link_check.rate_limit_ms;

    // Extract stable asset paths
    let stable_assets = config
        .stable_assets
        .paths
        .into_iter()
        .map(|p| p.path)
        .collect();

    Ok(ResolvedConfig {
        _root: root,
        content_dir,
        output_dir,
        skip_domains,
        rate_limit_ms,
        stable_assets,
        code_execution: config.code_execution,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let kdl = r#"
            content "docs/"
            output "public/"
            link_check {
            }
            stable_assets {
            }
        "#;

        let config: DodecaConfig = kdl::from_str(kdl).unwrap();
        assert_eq!(config.content.path, "docs/");
        assert_eq!(config.output.path, "public/");
        assert!(config.stable_assets.paths.is_empty());
    }
}
