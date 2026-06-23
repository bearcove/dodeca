//! Configuration file discovery and parsing
//!
//! Searches for `.config/dodeca.styx` walking up from the current directory.
//! The project root is the parent of `.config/`.
//!
//! Config parsing uses facet-styx directly — config is a bootstrap concern
//! that must be resolved before cells are available.

use arc_swap::ArcSwapOption;
use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use std::env;
use std::fs;

// Re-export config types from dodeca-config crate
pub use dodeca_config::{
    AuthConfig, CodeExecutionConfig, DodecaConfig, LinkCheckMode, PageTypeSchema, SourceDef,
};

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

/// A resolved content source: an absolute content directory and the URL
/// namespace it mounts under. A leaf config resolves to exactly one of these
/// at mount `/`; an aggregator config resolves to one per `sources` entry.
#[derive(Debug, Clone, facet::Facet)]
pub struct ResolvedSource {
    /// Stable identity, used for cross-source links (`[[<name>:slug]]`) and
    /// search labelling — independent of `mount`. Empty for the degenerate
    /// single-`content` project (no cross-source linking there).
    pub name: String,
    /// Normalized URL mount prefix: leading slash, trailing slash, root is `/`
    /// (e.g. `/`, `/spec/build/`).
    pub mount: String,
    /// Absolute path to this source's content directory.
    pub content_dir: Utf8PathBuf,
    /// Absolute path to the repo checkout dir the service clones/pulls (the
    /// content dir lives within it). `None` for a direct `local` source.
    pub checkout_dir: Option<Utf8PathBuf>,
    /// Remote to clone/pull the `checkout_dir` from, and to suggest cloning when
    /// the source isn't checked out locally.
    pub git: Option<String>,
}

/// Discovered configuration with resolved paths
#[derive(Debug, Clone, facet::Facet)]
pub struct ResolvedConfig {
    /// Project root (parent of .config/)
    pub _root: Utf8PathBuf,
    /// Base URL for the site (e.g., `https://example.com` or `/` for local dev)
    pub base_url: String,
    /// All content sources, in declaration order. Always non-empty.
    pub sources: Vec<ResolvedSource>,
    /// Absolute path to content directory.
    ///
    /// Transitional: the build pipeline still consumes a single content root.
    /// Until `BuildContext`-per-source lands (change-list #2), this is the first
    /// source's directory — the only one for leaf configs.
    pub content_dir: Utf8PathBuf,
    /// Absolute path to output directory
    pub output_dir: Utf8PathBuf,
    /// Domains to skip during external link checking
    pub skip_domains: Vec<String>,
    /// Rate limit for external link checking (milliseconds between requests to same domain)
    pub rate_limit_ms: Option<u64>,
    /// What to check (none / internal / full). Defaults to `Full`. CLI
    /// flag `--link-check` overrides this at command time.
    pub link_check_mode: LinkCheckMode,
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
    /// Frontmatter schemas keyed by page type.
    pub page_types: Option<std::collections::HashMap<String, PageTypeSchema>>,
    /// Auth config. `Some` ⇒ gate `/_dodeca/*` on a forwarded identity; `None`
    /// ⇒ open (local dev, no proxy).
    pub auth: Option<AuthConfig>,
}

impl ResolvedConfig {
    /// Get all project paths derived from this config
    pub fn paths(&self) -> ProjectPaths {
        ProjectPaths::from_config(self)
    }
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

        // Check for legacy configs and error if found
        check_legacy_configs(&config_dir)?;

        let styx_file = config_dir.join(CONFIG_FILE_STYX);
        if styx_file.exists() {
            let resolved = load_config(&styx_file)?;
            return Ok(Some(resolved));
        }

        Ok(None)
    }

    /// Discover and load configuration by walking up from a file or directory.
    pub fn discover_containing(path: &Utf8Path) -> Result<Option<Self>> {
        let mut current = if path.is_dir() {
            path.to_owned()
        } else if let Some(parent) = path.parent() {
            parent.to_owned()
        } else {
            path.to_owned()
        };

        loop {
            let config_dir = current.join(CONFIG_DIR);
            check_legacy_configs(&config_dir)?;

            let styx_file = config_dir.join(CONFIG_FILE_STYX);
            if styx_file.exists() {
                let resolved = load_config(&styx_file)?;
                return Ok(Some(resolved));
            }

            match current.parent() {
                Some(parent) => current = parent.to_owned(),
                None => return Ok(None),
            }
        }
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

/// Load and resolve configuration from a config file path
fn load_config(config_path: &Utf8Path) -> Result<ResolvedConfig> {
    let content = fs::read_to_string(config_path)?;

    let config: DodecaConfig = facet_styx::from_str(&content)
        .map_err(|e| eyre!("Failed to parse {}: {}", config_path, e))?;

    // Project root is the parent of .config/
    let config_dir = config_path
        .parent()
        .ok_or_else(|| eyre!("Config file has no parent directory"))?;
    let root = config_dir
        .parent()
        .ok_or_else(|| eyre!(".config directory has no parent"))?
        .to_owned();

    // Resolve content sources (explicit `sources`, or a single one synthesized
    // from `content`). Transitional: the pipeline still uses one `content_dir`,
    // so we keep the first source's directory there until per-source rendering
    // lands.
    let sources = resolve_sources(&root, &config)?;
    let content_dir = sources
        .first()
        .map(|s| s.content_dir.clone())
        .ok_or_else(|| eyre!("no content sources resolved"))?;
    let output_dir = root.join(&config.output);

    // Extract skip domains
    let skip_domains = config
        .link_check
        .as_ref()
        .and_then(|lc| lc.skip_domains.clone())
        .unwrap_or_default();

    // Extract rate limit
    let rate_limit_ms = config.link_check.as_ref().and_then(|lc| lc.rate_limit_ms);

    // Extract link check mode (defaults to Full)
    let link_check_mode = config
        .link_check
        .as_ref()
        .and_then(|lc| lc.mode)
        .unwrap_or_default();

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
        sources,
        content_dir,
        output_dir,
        skip_domains,
        rate_limit_ms,
        link_check_mode,
        stable_assets,
        code_execution: config.code_execution.unwrap_or_default(),
        light_theme_css,
        dark_theme_css,
        build_steps: config.build_steps,
        page_types: config.page_types,
        auth: config.auth,
    })
}

/// Resolve the content sources from a config: either the explicit `sources`
/// list, or a single source synthesized from `content` and mounted at `/`.
/// Exactly one of the two must be present.
fn resolve_sources(root: &Utf8Path, config: &DodecaConfig) -> Result<Vec<ResolvedSource>> {
    match (&config.sources, &config.content) {
        (Some(_), Some(_)) => Err(eyre!(
            "config sets both `content` and `sources`; use one — \
             `content` for a single-source project, `sources` for an aggregator"
        )),
        (Some(sources), None) => {
            if sources.is_empty() {
                return Err(eyre!(
                    "`sources` is present but empty; list at least one source or use `content`"
                ));
            }
            sources.iter().map(|s| resolve_source(root, s)).collect()
        }
        (None, Some(content)) => Ok(vec![ResolvedSource {
            name: String::new(),
            mount: "/".to_string(),
            content_dir: root.join(content),
            checkout_dir: None,
            git: None,
        }]),
        (None, None) => Err(eyre!(
            "config must set either `content` (single source) or `sources` (multiple)"
        )),
    }
}

/// Resolve one `SourceDef` to an absolute content dir + normalized mount.
/// Git-only sources (no `local`) are rejected for now — fetching is deferred.
fn resolve_source(root: &Utf8Path, def: &SourceDef) -> Result<ResolvedSource> {
    let mount = normalize_mount(&def.mount);
    if def.name.trim().is_empty() {
        return Err(eyre!(
            "source mounted at `{mount}` has an empty `name`; \
             every source needs a name (used for cross-source links)"
        ));
    }
    let (content_dir, checkout_dir) = match (&def.local, &def.checkout) {
        (Some(_), Some(_)) => {
            return Err(eyre!(
                "source `{}` sets both `local` and `checkout`; use one \
                 (`local` for a direct content dir, `checkout` for a repo)",
                def.name
            ));
        }
        (Some(local), None) => (root.join(local), None),
        (None, Some(checkout)) => {
            let checkout_dir = root.join(checkout);
            let content_dir = match &def.content {
                Some(content) => checkout_dir.join(content),
                None => checkout_dir.clone(),
            };
            (content_dir, Some(checkout_dir))
        }
        (None, None) => {
            return Err(eyre!(
                "source `{}` (mounted at `{mount}`) needs `local` (a content \
                 dir) or `checkout` (a repo dir)",
                def.name
            ));
        }
    };
    Ok(ResolvedSource {
        name: def.name.clone(),
        mount,
        content_dir,
        checkout_dir,
        git: def.git.clone(),
    })
}

/// Normalize a mount prefix to a canonical form: a leading and trailing slash,
/// with the root collapsing to `/` (e.g. `spec/build` → `/spec/build/`,
/// `/` → `/`).
fn normalize_mount(raw: &str) -> String {
    let trimmed = raw.trim_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        format!("/{trimmed}/")
    }
}

// ============================================================================
// Global config access
// ============================================================================

/// Global resolved configuration.
///
/// Stored in an [`ArcSwapOption`] so the serve loop can hot-reload it when
/// `.config/dodeca.styx` changes: a new config is published with a single
/// atomic store, and existing readers that hold an `Arc` to the previous one
/// keep it alive until they drop it (then it's freed — no leak). Lock-free on
/// the read path, which matters because `global_config()` is hit per render.
static RESOLVED_CONFIG: ArcSwapOption<ResolvedConfig> = ArcSwapOption::const_empty();

/// Install a config as the global one (startup) or replace it (hot-reload).
///
/// Re-publishes the build-step executor for the new config, then atomically
/// swaps the config in. Safe to call more than once: a later call supersedes
/// the earlier config for all *new* `global_config()` reads.
pub fn set_global_config(config: ResolvedConfig) -> Result<()> {
    // Initialize build step executor
    let executor = std::sync::Arc::new(crate::build_steps::BuildStepExecutor::new(
        config.build_steps.clone(),
        config._root.clone(),
    ));
    crate::host::Host::get().set_build_step_executor(executor);

    RESOLVED_CONFIG.store(Some(std::sync::Arc::new(config)));
    Ok(())
}

/// Get the global config (returns None if not initialized).
///
/// Returns an owned `Arc` snapshot: the caller sees a consistent config even if
/// a hot-reload swaps in a new one mid-use.
pub fn global_config() -> Option<std::sync::Arc<ResolvedConfig>> {
    RESOLVED_CONFIG.load_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal `DodecaConfig` with everything but `content`/`sources` defaulted.
    fn config(content: Option<&str>, sources: Option<Vec<SourceDef>>) -> DodecaConfig {
        DodecaConfig {
            base_url: None,
            content: content.map(str::to_string),
            output: "public".to_string(),
            sources,
            link_check: None,
            stable_assets: None,
            code_execution: None,
            syntax_highlight: None,
            build_steps: None,
            page_types: None,
            auth: None,
        }
    }

    fn source(name: &str, mount: &str, local: Option<&str>) -> SourceDef {
        SourceDef {
            name: name.to_string(),
            mount: mount.to_string(),
            local: local.map(str::to_string),
            checkout: None,
            content: None,
            git: None,
        }
    }

    #[test]
    fn normalize_mount_canonicalizes() {
        assert_eq!(normalize_mount("/"), "/");
        assert_eq!(normalize_mount(""), "/");
        assert_eq!(normalize_mount("spec/build"), "/spec/build/");
        assert_eq!(normalize_mount("/spec/build"), "/spec/build/");
        assert_eq!(normalize_mount("/spec/build/"), "/spec/build/");
    }

    #[test]
    fn single_content_yields_one_root_source() {
        let root = Utf8Path::new("/proj");
        let sources = resolve_sources(root, &config(Some("content"), None)).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].mount, "/");
        assert_eq!(sources[0].content_dir, Utf8Path::new("/proj/content"));
    }

    #[test]
    fn explicit_sources_resolve_with_mounts() {
        let root = Utf8Path::new("/proj");
        let defs = vec![
            source("kb", "/", Some("content")),
            source("build", "/spec/build", Some("../vixen/docs/content")),
        ];
        let sources = resolve_sources(root, &config(None, Some(defs))).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].name, "kb");
        assert_eq!(sources[0].mount, "/");
        assert_eq!(sources[0].content_dir, Utf8Path::new("/proj/content"));
        assert_eq!(sources[1].name, "build");
        assert_eq!(sources[1].mount, "/spec/build/");
        assert_eq!(
            sources[1].content_dir,
            Utf8Path::new("/proj/../vixen/docs/content")
        );
    }

    #[test]
    fn content_and_sources_together_is_an_error() {
        let root = Utf8Path::new("/proj");
        let defs = vec![source("kb", "/", Some("content"))];
        assert!(resolve_sources(root, &config(Some("content"), Some(defs))).is_err());
    }

    #[test]
    fn neither_content_nor_sources_is_an_error() {
        let root = Utf8Path::new("/proj");
        assert!(resolve_sources(root, &config(None, None)).is_err());
    }

    #[test]
    fn empty_sources_list_is_an_error() {
        let root = Utf8Path::new("/proj");
        assert!(resolve_sources(root, &config(None, Some(vec![]))).is_err());
    }

    #[test]
    fn git_only_source_is_rejected_for_now() {
        let root = Utf8Path::new("/proj");
        let defs = vec![source("build", "/spec/build", None)];
        assert!(resolve_sources(root, &config(None, Some(defs))).is_err());
    }

    #[test]
    fn source_without_name_is_rejected() {
        let root = Utf8Path::new("/proj");
        let defs = vec![source("", "/spec/build", Some("../vixen/docs/content"))];
        assert!(resolve_sources(root, &config(None, Some(defs))).is_err());
    }

    #[test]
    fn checkout_source_resolves_content_subpath_and_checkout_dir() {
        let root = Utf8Path::new("/proj");
        let def = SourceDef {
            name: "build".into(),
            mount: "/spec/build".into(),
            local: None,
            checkout: Some("../vixen".into()),
            content: Some("docs/content".into()),
            git: Some("g.git".into()),
        };
        let sources = resolve_sources(root, &config(None, Some(vec![def]))).unwrap();
        assert_eq!(
            sources[0].content_dir,
            Utf8Path::new("/proj/../vixen/docs/content")
        );
        assert_eq!(
            sources[0].checkout_dir.as_deref(),
            Some(Utf8Path::new("/proj/../vixen"))
        );
        assert_eq!(sources[0].git.as_deref(), Some("g.git"));
    }

    #[test]
    fn local_and_checkout_together_is_an_error() {
        let root = Utf8Path::new("/proj");
        let def = SourceDef {
            name: "x".into(),
            mount: "/".into(),
            local: Some("content".into()),
            checkout: Some("../x".into()),
            content: None,
            git: None,
        };
        assert!(resolve_sources(root, &config(None, Some(vec![def]))).is_err());
    }
}
