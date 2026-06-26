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
    AuthConfig, CodeExecutionConfig, DodecaConfig, LinkCheckMode, MountDef, PageTypeSchema,
    SiteConfig, SourceConfig,
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
    /// Browsable repository URL, exposed to templates for "view on GitHub" links.
    pub repo: Option<String>,
    /// Code implementations scanned for requirement references to compute
    /// coverage of this source's spec rules. Empty when the source declares no
    /// `impls`.
    pub impls: Vec<ResolvedImpl>,
    /// External domains this source asks the link checker to skip. Unioned into
    /// the site's [`skip_domains`](ResolvedConfig::skip_domains).
    pub skip_domains: Vec<String>,
    /// Directory this source's build steps run in and resolve file params
    /// against — its own project root (where its `.config` lives), the checkout
    /// dir for a git mount, or the content dir as a last resort.
    pub project_dir: Utf8PathBuf,
    /// This source's build step definitions (composed from its own `source {}`).
    /// A `build("step")` call from this source's templates resolves here and runs
    /// in [`project_dir`](Self::project_dir).
    pub build_steps: std::collections::HashMap<String, dodeca_config::BuildStepDef>,
    /// This source's frontmatter schemas. Merged into the site-wide
    /// [`page_types`](ResolvedConfig::page_types); a type name may be defined by
    /// only one source.
    pub page_types: std::collections::HashMap<String, PageTypeSchema>,
}

/// A code implementation to scan for requirement references (resolved from
/// [`dodeca_config::ImplDef`]). Globs are project-root-relative; the scanner
/// resolves them at scan time.
#[derive(Debug, Clone, facet::Facet)]
pub struct ResolvedImpl {
    /// Name of this implementation (e.g. `rust`).
    pub name: String,
    /// Globs for source files to scan.
    pub include: Vec<String>,
    /// Globs to exclude from `include`.
    pub exclude: Vec<String>,
    /// Globs for test files (references may only *verify*, not *implement*).
    pub test_include: Vec<String>,
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
    /// Frontmatter schemas keyed by page type. The federated union of every
    /// source's `page_types` (a type name is owned by exactly one source).
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
/// Absolute path to the project config file given the project root (parent of
/// `.config/`). Used by the serve file-watcher to watch for config changes.
pub fn config_file_path(root: &Utf8Path) -> Utf8PathBuf {
    root.join(CONFIG_DIR).join(CONFIG_FILE_STYX)
}

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

    resolve_config(&root, config)
}

/// Resolve a parsed [`DodecaConfig`] against its project `root`: assemble the
/// sources (root `source` + each `mounts` entry, composing the mounted source's
/// own `source {}`), and pull whole-site settings from `site {}`.
fn resolve_config(root: &Utf8Path, config: DodecaConfig) -> Result<ResolvedConfig> {
    let DodecaConfig {
        source,
        site,
        mounts,
    } = config;

    // `site` is optional in the schema (a mount-only sub-config omits it), but
    // the config being built must declare one — that's where `output` lives.
    let site = site.ok_or_else(|| {
        eyre!("config must have a `site` section (with at least an `output` dir)")
    })?;

    let sources = resolve_sources(root, source.as_ref(), mounts.as_deref())?;
    let content_dir = sources
        .first()
        .map(|s| s.content_dir.clone())
        .ok_or_else(|| eyre!("no content sources resolved"))?;
    let output_dir = root.join(&site.output);

    // skip_domains: the site-wide `link_check.skip_domains` base, with every
    // source's unioned in on top, deduped (order-stable).
    let mut skip_domains = Vec::new();
    if let Some(base) = site.link_check.as_ref().and_then(|lc| lc.skip_domains.as_ref()) {
        for d in base {
            if !skip_domains.contains(d) {
                skip_domains.push(d.clone());
            }
        }
    }
    for s in &sources {
        for d in &s.skip_domains {
            if !skip_domains.contains(d) {
                skip_domains.push(d.clone());
            }
        }
    }

    let rate_limit_ms = site.link_check.as_ref().and_then(|lc| lc.rate_limit_ms);
    let link_check_mode = site
        .link_check
        .as_ref()
        .and_then(|lc| lc.mode)
        .unwrap_or_default();
    let stable_assets = site.stable_assets.unwrap_or_default();

    let light_theme_name = site
        .syntax_highlight
        .as_ref()
        .and_then(|sh| sh.light_theme.as_deref())
        .unwrap_or("github-light");
    let dark_theme_name = site
        .syntax_highlight
        .as_ref()
        .and_then(|sh| sh.dark_theme.as_deref())
        .unwrap_or("tokyo-night");
    let light_theme_css = crate::theme_resolver::generate_theme_css(light_theme_name)
        .map_err(|e| eyre!("Failed to load light theme '{}': {}", light_theme_name, e))?;
    let dark_theme_css = crate::theme_resolver::generate_theme_css(dark_theme_name)
        .map_err(|e| eyre!("Failed to load dark theme '{}': {}", dark_theme_name, e))?;

    let base_url = site.base_url.unwrap_or_else(|| "/".to_string());

    warn_orphaned_nested_configs(root, &sources);

    // Build steps are source-scoped: each `ResolvedSource` carries its own, and
    // `build()` resolves against the rendering page's source (see the build-step
    // executor) — there is no whole-site build-steps map.

    let page_types = merge_page_types(&sources)?;

    Ok(ResolvedConfig {
        _root: root.to_owned(),
        base_url,
        sources,
        content_dir,
        output_dir,
        skip_domains,
        rate_limit_ms,
        link_check_mode,
        stable_assets,
        code_execution: site.code_execution.unwrap_or_default(),
        light_theme_css,
        dark_theme_css,
        page_types,
        auth: site.auth,
    })
}

/// Assemble the resolved sources: the aggregator's own root `source` at `/`,
/// plus each `mounts` entry (composing the mounted source's own `source {}`).
/// At least one must be present.
fn resolve_sources(
    root: &Utf8Path,
    source: Option<&SourceConfig>,
    mounts: Option<&[MountDef]>,
) -> Result<Vec<ResolvedSource>> {
    let mut resolved = Vec::new();

    if let Some(src) = source {
        let content = src.content.as_deref().unwrap_or("content");
        resolved.push(ResolvedSource {
            name: String::new(),
            mount: "/".to_string(),
            content_dir: root.join(content),
            checkout_dir: None,
            git: None,
            repo: src.repo.clone(),
            impls: resolve_impls(&src.impls),
            skip_domains: src.skip_domains.clone(),
            // The root source runs build steps in the project root.
            project_dir: root.to_owned(),
            build_steps: src.build_steps.clone().unwrap_or_default(),
            page_types: src.page_types.clone().unwrap_or_default(),
        });
    }

    for m in mounts.unwrap_or_default() {
        resolved.push(resolve_mount(root, m)?);
    }

    if resolved.is_empty() {
        return Err(eyre!(
            "config must set a top-level `source` (root content) and/or `mounts`"
        ));
    }
    Ok(resolved)
}

/// Resolve one `mounts` entry: its location from `local`/`checkout`, its
/// behavior (`impls`, `skip_domains`, `repo`) composed from the mounted source's
/// own `source {}` (read from a `.config` at-or-above its content dir).
fn resolve_mount(root: &Utf8Path, def: &MountDef) -> Result<ResolvedSource> {
    if def.name.trim().is_empty() {
        return Err(eyre!(
            "mount at `{}` has an empty `name`; every source needs a name",
            def.path
        ));
    }
    let mount = normalize_mount(&def.path);
    if mount == "/" {
        return Err(eyre!(
            "mount `{}` resolves to `/`; the root is the aggregator's own \
             top-level `source`, not a `mounts` entry",
            def.name
        ));
    }
    let (content_dir, checkout_dir) = match (&def.local, &def.checkout) {
        (Some(_), Some(_)) => {
            return Err(eyre!(
                "mount `{}` sets both `local` and `checkout`; use one",
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
                "mount `{}` (at `{mount}`) needs `local` (a content dir) or \
                 `checkout` (a repo dir)",
                def.name
            ));
        }
    };

    // Compose the mounted source's own `source {}` for its behavior. The dir
    // holding that config is the source's project root (where its build steps
    // run); without one, fall back to the checkout dir, then the content dir.
    let composed = discover_source_config(&content_dir);
    let project_dir = composed
        .as_ref()
        .map(|(dir, _)| dir.clone())
        .or_else(|| checkout_dir.clone())
        .unwrap_or_else(|| content_dir.clone());
    let composed = composed.map(|(_, s)| s);
    Ok(ResolvedSource {
        name: def.name.clone(),
        mount,
        content_dir,
        checkout_dir,
        git: def.git.clone(),
        // The mount's explicit `repo` wins; otherwise compose it from the
        // mounted source's own config (the cloned-independent-repo case).
        repo: def
            .repo
            .clone()
            .or_else(|| composed.as_ref().and_then(|s| s.repo.clone())),
        impls: composed
            .as_ref()
            .map(|s| resolve_impls(&s.impls))
            .unwrap_or_default(),
        skip_domains: composed
            .as_ref()
            .map(|s| s.skip_domains.clone())
            .unwrap_or_default(),
        project_dir,
        build_steps: composed
            .as_ref()
            .and_then(|s| s.build_steps.clone())
            .unwrap_or_default(),
        page_types: composed
            .as_ref()
            .and_then(|s| s.page_types.clone())
            .unwrap_or_default(),
    })
}

/// Read the `source {}` of the config that owns `content_dir` — the nearest
/// `.config/dodeca.styx` at or above it — for composition. Parse-only (no nested
/// resolve), and best-effort: a source with no config of its own just has no
/// composed behavior.
fn discover_source_config(content_dir: &Utf8Path) -> Option<(Utf8PathBuf, SourceConfig)> {
    let mut current = content_dir;
    loop {
        let styx_file = current.join(CONFIG_DIR).join(CONFIG_FILE_STYX);
        if styx_file.exists() {
            let parsed = fs::read_to_string(&styx_file)
                .ok()
                .and_then(|content| facet_styx::from_str::<DodecaConfig>(&content).ok());
            // The dir holding `.config/` is this source's own project root.
            return parsed.and_then(|c| c.source).map(|s| (current.to_owned(), s));
        }
        current = current.parent()?;
    }
}

/// Warn about nested `.config/dodeca.styx` files buried *inside* a served
/// source's content dir. Such a config used to silently shadow the aggregator
/// when discovery walked up from a content file; now composition is explicit, so
/// it's just dead config that's being ignored — the author either meant to
/// `mounts` that subtree or should remove the config. Scanning is scoped to the
/// content actually served (not the whole project), so unrelated sibling
/// projects / fixtures don't trip it. Best-effort, never fatal.
fn warn_orphaned_nested_configs(_root: &Utf8Path, sources: &[ResolvedSource]) {
    for source in sources {
        for entry in ignore::WalkBuilder::new(&source.content_dir)
            .hidden(false)
            .build()
            .flatten()
        {
            if entry.file_name() != CONFIG_FILE_STYX {
                continue;
            }
            let Some(path) = Utf8Path::from_path(entry.path()) else {
                continue;
            };
            // Only flag configs strictly *within* served content; a source's own
            // config lives above its content dir, not inside it.
            if !path.starts_with(&source.content_dir) {
                continue;
            }
            tracing::warn!(
                config = %path,
                source = %source.content_dir,
                "nested dodeca config is buried inside served content; it is \
                 ignored. Add a `mounts` entry for that subtree, or remove it."
            );
        }
    }
}

/// Federate every source's frontmatter `page_types` into one site-wide map.
///
/// Typed cross-source links resolve against this single index, so the types form
/// one namespace; a type name may therefore be defined by only one source, and a
/// collision is a hard error (silently picking one would mis-validate the
/// other's pages). `None` when no source declares any types.
fn merge_page_types(
    sources: &[ResolvedSource],
) -> Result<Option<std::collections::HashMap<String, PageTypeSchema>>> {
    let mut page_types: std::collections::HashMap<String, PageTypeSchema> =
        std::collections::HashMap::new();
    let mut owner: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for src in sources {
        let label = if src.mount == "/" {
            "the root source".to_string()
        } else {
            format!("mount `{}` (at `{}`)", src.name, src.mount)
        };
        for (name, schema) in &src.page_types {
            if let Some(prev) = owner.get(name) {
                return Err(eyre!(
                    "frontmatter type `{name}` is defined by both {prev} and {label}; \
                     define each type in exactly one source"
                ));
            }
            page_types.insert(name.clone(), schema.clone());
            owner.insert(name.clone(), label.clone());
        }
    }
    Ok((!page_types.is_empty()).then_some(page_types))
}

/// Resolve `ImplDef`s (config schema) into `ResolvedImpl`s.
fn resolve_impls(impls: &[dodeca_config::ImplDef]) -> Vec<ResolvedImpl> {
    impls
        .iter()
        .cloned()
        .map(|i| ResolvedImpl {
            name: i.name,
            include: i.include,
            exclude: i.exclude,
            test_include: i.test_include,
        })
        .collect()
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
    // Initialize the build step executor with every source's steps (each runs in
    // its own project dir; `build()` resolves against the rendering source).
    let executor = std::sync::Arc::new(crate::build_steps::BuildStepExecutor::new(
        &config.sources,
    ));
    crate::host::Host::get().set_build_step_executor(executor);

    RESOLVED_CONFIG.store(Some(std::sync::Arc::new(config)));
    Ok(())
}

/// Get the global config (returns None if not initialized).
///
/// When called inside a request/render scope (a `TASK_DB` is set), this reads
/// through the [`ConfigRegistry`](crate::db::ConfigRegistry) picante input so
/// the *calling tracked query records a dependency on the config* — that's what
/// makes a config change auto-invalidate renders, per-source CSS, and search.
/// Outside a request (pre-db bootstrap, the non-tracked HTTP path) it falls
/// back to the ambient `ArcSwap` snapshot.
///
/// Both are kept in sync: every config install sets the ambient snapshot and
/// (once a db exists) the `ConfigRegistry` input from the same `ResolvedConfig`.
pub fn global_config() -> Option<std::sync::Arc<ResolvedConfig>> {
    if let Ok(Ok(Some(cfg))) =
        crate::db::TASK_DB.try_with(|db| crate::db::ConfigRegistry::config(db.as_ref()))
    {
        return Some(cfg);
    }
    RESOLVED_CONFIG.load_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `SourceConfig` with the given content dir, otherwise default.
    fn src_cfg(content: Option<&str>) -> SourceConfig {
        SourceConfig {
            content: content.map(str::to_string),
            ..Default::default()
        }
    }

    /// A `mounts` entry: name + path + a direct `local` content dir.
    fn mount(name: &str, path: &str, local: Option<&str>) -> MountDef {
        MountDef {
            name: name.to_string(),
            path: path.to_string(),
            local: local.map(str::to_string),
            ..Default::default()
        }
    }

    /// Resolve a root `source` and/or `mounts` against `/proj`.
    fn resolve(
        source: Option<SourceConfig>,
        mounts: Option<Vec<MountDef>>,
    ) -> Result<Vec<ResolvedSource>> {
        resolve_sources(Utf8Path::new("/proj"), source.as_ref(), mounts.as_deref())
    }

    /// A `ResolvedSource` at `mount` declaring frontmatter types `type_names`
    /// (each a trivial `@bool` schema), everything else empty.
    fn source_with_page_types(mount: &str, type_names: &[&str]) -> ResolvedSource {
        ResolvedSource {
            name: mount.trim_matches('/').to_string(),
            mount: mount.to_string(),
            content_dir: Utf8PathBuf::from("/proj/content"),
            checkout_dir: None,
            git: None,
            repo: None,
            impls: Vec::new(),
            skip_domains: Vec::new(),
            project_dir: Utf8PathBuf::from("/proj"),
            build_steps: Default::default(),
            page_types: type_names
                .iter()
                .map(|n| (n.to_string(), PageTypeSchema::Bool))
                .collect(),
        }
    }

    #[test]
    fn merge_page_types_unions_across_sources() {
        let sources = [
            source_with_page_types("/", &["Decision"]),
            source_with_page_types("/wiki/", &["Vision"]),
        ];
        let merged = merge_page_types(&sources).unwrap().unwrap();
        assert_eq!(merged.len(), 2);
        assert!(merged.contains_key("Decision"));
        assert!(merged.contains_key("Vision"));
    }

    #[test]
    fn merge_page_types_rejects_duplicate_name_across_sources() {
        let sources = [
            source_with_page_types("/", &["Decision"]),
            source_with_page_types("/wiki/", &["Decision"]),
        ];
        assert!(merge_page_types(&sources).is_err());
    }

    #[test]
    fn merge_page_types_is_none_when_no_source_declares_any() {
        let sources = [source_with_page_types("/", &[])];
        assert!(merge_page_types(&sources).unwrap().is_none());
    }

    #[test]
    fn root_source_carries_impls() {
        let mut src = src_cfg(Some("docs/content"));
        src.impls = vec![dodeca_config::ImplDef {
            name: "rust".into(),
            include: vec!["rust/**/src/**/*.rs".into()],
            exclude: vec!["**/target/**".into()],
            test_include: vec!["rust/**/tests/**/*.rs".into()],
        }];
        let sources = resolve(Some(src), None).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].mount, "/");
        assert_eq!(sources[0].impls.len(), 1);
        assert_eq!(sources[0].impls[0].include, vec!["rust/**/src/**/*.rs"]);
    }

    #[test]
    fn root_mount_path_is_rejected() {
        // `/` is the top-level `source`, not a `mounts` entry.
        assert!(resolve(None, Some(vec![mount("x", "/", Some("content"))])).is_err());
    }

    /// A minimal `ResolvedConfig` distinguished only by its `base_url`.
    fn resolved(base_url: &str) -> ResolvedConfig {
        ResolvedConfig {
            _root: Utf8PathBuf::from("/proj"),
            base_url: base_url.to_string(),
            sources: vec![],
            content_dir: Utf8PathBuf::from("/proj/content"),
            output_dir: Utf8PathBuf::from("/proj/public"),
            skip_domains: vec![],
            rate_limit_ms: None,
            link_check_mode: LinkCheckMode::default(),
            stable_assets: vec![],
            code_execution: Default::default(),
            light_theme_css: String::new(),
            dark_theme_css: String::new(),
            page_types: None,
            auth: None,
        }
    }

    /// `global_config()` reads through the `ConfigRegistry` picante input when a
    /// `TASK_DB` is in scope (the render/request path) and observes input
    /// updates — the wiring that lets a config reload invalidate renders. Reads
    /// outside a scope fall back to the ambient snapshot.
    #[test]
    fn global_config_reads_through_config_input_in_task_scope() {
        use crate::db::{ConfigRegistry, Database};
        use std::sync::Arc;

        let db = Arc::new(Database::new(None));
        ConfigRegistry::set(&*db, Arc::new(resolved("https://a/"))).unwrap();

        let a = crate::db::TASK_DB
            .sync_scope(db.clone(), || global_config().map(|c| c.base_url.clone()));
        assert_eq!(a.as_deref(), Some("https://a/"));

        // Updating the input is observed by a fresh in-scope read.
        ConfigRegistry::set(&*db, Arc::new(resolved("https://b/"))).unwrap();
        let b = crate::db::TASK_DB
            .sync_scope(db.clone(), || global_config().map(|c| c.base_url.clone()));
        assert_eq!(b.as_deref(), Some("https://b/"));
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
    fn root_source_yields_one_source_at_slash() {
        let sources = resolve(Some(src_cfg(Some("content"))), None).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].mount, "/");
        assert_eq!(sources[0].content_dir, Utf8Path::new("/proj/content"));
    }

    #[test]
    fn root_source_defaults_content_to_content_dir() {
        let sources = resolve(Some(src_cfg(None)), None).unwrap();
        assert_eq!(sources[0].content_dir, Utf8Path::new("/proj/content"));
    }

    #[test]
    fn root_source_plus_mount_resolve() {
        let sources = resolve(
            Some(src_cfg(Some("content"))),
            Some(vec![mount(
                "build",
                "/spec/build",
                Some("../vixen/docs/content"),
            )]),
        )
        .unwrap();
        assert_eq!(sources.len(), 2);
        // The root source is unnamed and mounted at `/`.
        assert_eq!(sources[0].name, "");
        assert_eq!(sources[0].mount, "/");
        assert_eq!(sources[0].content_dir, Utf8Path::new("/proj/content"));
        // The mount carries its own name and normalized mount path.
        assert_eq!(sources[1].name, "build");
        assert_eq!(sources[1].mount, "/spec/build/");
        assert_eq!(
            sources[1].content_dir,
            Utf8Path::new("/proj/../vixen/docs/content")
        );
    }

    #[test]
    fn mounts_alone_resolve() {
        let sources = resolve(
            None,
            Some(vec![mount("build", "/spec/build", Some("../vixen/content"))]),
        )
        .unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name, "build");
        assert_eq!(sources[0].mount, "/spec/build/");
    }

    #[test]
    fn neither_source_nor_mounts_is_an_error() {
        assert!(resolve(None, None).is_err());
    }

    #[test]
    fn no_source_and_empty_mounts_is_an_error() {
        assert!(resolve(None, Some(vec![])).is_err());
    }

    #[test]
    fn git_only_mount_is_rejected_for_now() {
        assert!(resolve(None, Some(vec![mount("build", "/spec/build", None)])).is_err());
    }

    #[test]
    fn mount_without_name_is_rejected() {
        assert!(
            resolve(
                None,
                Some(vec![mount("", "/spec/build", Some("../vixen/content"))]),
            )
            .is_err()
        );
    }

    #[test]
    fn checkout_mount_resolves_content_subpath_and_checkout_dir() {
        let def = MountDef {
            name: "build".into(),
            path: "/spec/build".into(),
            checkout: Some("../vixen".into()),
            content: Some("docs/content".into()),
            git: Some("g.git".into()),
            ..Default::default()
        };
        let sources = resolve(None, Some(vec![def])).unwrap();
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
    fn mount_repo_is_carried_through() {
        // A vendored mount with no config of its own to compose from still
        // carries its explicit `repo` (`/proj/content` doesn't exist, so there
        // is nothing to compose).
        let mut m = mount("x", "/x", Some("content"));
        m.repo = Some("https://example.com/x".into());
        let sources = resolve(None, Some(vec![m])).unwrap();
        assert_eq!(sources[0].repo.as_deref(), Some("https://example.com/x"));
    }

    #[test]
    fn local_and_checkout_together_is_an_error() {
        let def = MountDef {
            name: "x".into(),
            path: "/x".into(),
            local: Some("content".into()),
            checkout: Some("../x".into()),
            ..Default::default()
        };
        assert!(resolve(None, Some(vec![def])).is_err());
    }
}
