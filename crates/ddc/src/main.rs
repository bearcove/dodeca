#![allow(clippy::collapsible_if)]

use camino::{Utf8Path, Utf8PathBuf};
use dodeca::config::{LinkCheckMode, ResolvedConfig};
use dodeca::db::{
    self, CodeCoverageRegistry, CodeRegistry, ConfigRegistry, DataFile, DataRegistry, Database,
    MarkdownRenderSettings, OutputFile, QueryStats, SassFile, SassRegistry, SourceFile,
    SourceRegistry, StaticFile, StaticRegistry, TemplateFile, TemplateRegistry,
};
use dodeca::queries::{self, build_site};
use dodeca::tui::{self, LogEvent};
use dodeca::types::{
    DataContent, DataPath, Route, SassContent, SassPath, SourceContent, SourcePath, StaticPath,
    TemplateContent, TemplatePath,
};
use dodeca::{
    BuildContext, cas, cell_server, cells, file_watcher, host, init, is_data_file_extension,
    link_checker, logging, render, serve, template_paths, tui_host, vite,
};
use dodeca_authoring_lsp::authoring_lsp::{self, AuthoringDiagnostic, AuthoringDiagnosticKind};
use eyre::{Result, eyre};
use facet::Facet;
use figue::{self as args, FigueBuiltins};
use ignore::WalkBuilder;
use owo_colors::OwoColorize;
use std::collections::HashSet;
use std::fs;
use std::sync::Arc;

const AGENT_GUIDE: &str = include_str!("agent_guide.md");

/// ddc - Static site generator
#[derive(Facet, Debug)]
struct Args {
    /// Command to run
    #[facet(args::subcommand)]
    command: Command,

    /// Standard CLI options (--help, --version, --completions)
    #[facet(flatten)]
    builtins: FigueBuiltins,
}

/// Build command arguments
#[derive(Facet, Debug)]
struct BuildArgs {
    /// Project directory (looks for .config/dodeca.styx here)
    #[facet(args::positional, default)]
    path: Option<String>,

    /// Content directory (uses .config/dodeca.styx if not specified)
    #[facet(args::named, args::short = 'c', default)]
    content: Option<String>,

    /// Output directory (uses .config/dodeca.styx if not specified)
    #[facet(args::named, args::short = 'o', default)]
    output: Option<String>,

    /// Override link checking mode (`none`, `internal`, or `full`).
    /// CLI wins over `link_check.mode` in `.config/dodeca.styx`.
    /// `none` skips link checking entirely (useful for fast prod builds);
    /// `internal` checks intra-site links only (no network).
    #[facet(args::named, default)]
    link_check: Option<LinkCheckMode>,

    /// Show TUI progress display
    #[facet(args::named)]
    tui: bool,
}

/// Serve command arguments
#[derive(Facet, Debug)]
struct ServeArgs {
    /// Project directory (looks for .config/dodeca.styx here)
    #[facet(args::positional, default)]
    path: Option<String>,

    /// Content directory (uses .config/dodeca.styx if not specified)
    #[facet(args::named, args::short = 'c', default)]
    content: Option<String>,

    /// Output directory (uses .config/dodeca.styx if not specified)
    #[facet(args::named, args::short = 'o', default)]
    output: Option<String>,

    /// Address to bind on
    #[facet(args::named, args::short = 'a', default = "127.0.0.1".to_string())]
    address: String,

    /// Port to serve on (default: tries 4000-4019, then lets OS choose)
    #[facet(args::named, args::short = 'p', default)]
    port: Option<u16>,

    /// Open browser after starting server
    #[facet(args::named)]
    open: bool,

    /// Disable TUI (show plain output instead)
    #[facet(args::named)]
    no_tui: bool,

    /// Force TUI mode even without a terminal (for testing)
    #[facet(args::named)]
    force_tui: bool,

    /// Start with public access enabled (listen on all interfaces)
    #[facet(args::named, args::short = 'P', rename = "public")]
    public_access: bool,

    /// Control channel path to receive listening socket (for testing)
    #[facet(args::named, default)]
    fd_socket: Option<String>,

    /// Poll git-backed sources every N seconds (`git pull --ff-only`); 0/unset
    /// disables. The file watcher re-renders on pulled changes. This is the
    /// fallback path — webhooks are preferred.
    #[facet(args::named, default)]
    git_poll: Option<u64>,

    /// Local-only: act as this editor user, bypassing oauth2-proxy so you can
    /// drive the in-browser editor (`/_dodeca/edit/<page>`) without a real auth
    /// proxy. Refused on a non-loopback bind. Never use in production.
    #[facet(args::named, default)]
    dev_editor: Option<String>,
}

/// Clean command arguments
#[derive(Facet, Debug)]
struct CleanArgs {
    /// Project directory (looks for .config/dodeca.styx here)
    #[facet(args::positional, default)]
    path: Option<String>,

    /// Also clean Vite cache (node_modules/.vite)
    #[facet(args::named, default)]
    vite: bool,

    /// Also clean build output (output/)
    #[facet(args::named, default)]
    output: bool,

    /// Also clean Vite dist (dist/)
    #[facet(args::named, default)]
    dist: bool,

    /// Clean everything (equivalent to --vite --output --dist)
    #[facet(args::named, default)]
    all: bool,
}

/// Authoring LSP server arguments
#[derive(Facet, Debug)]
struct LspArgs {
    /// Content directory (uses .config/dodeca.styx if not specified)
    #[facet(args::named, args::short = 'c', default)]
    content: Option<String>,

    /// Output directory (uses .config/dodeca.styx if not specified)
    #[facet(args::named, args::short = 'o', default)]
    output: Option<String>,
}

/// Authoring diagnostics command arguments
#[derive(Facet, Debug)]
struct DiagnosticsArgs {
    /// Project directory (looks for .config/dodeca.styx here)
    #[facet(args::positional, default)]
    path: Option<String>,

    /// Content directory (uses .config/dodeca.styx if not specified)
    #[facet(args::named, args::short = 'c', default)]
    content: Option<String>,

    /// Output format
    #[facet(args::named, default)]
    format: DiagnosticsFormat,

    /// Only include diagnostics with this kind, e.g. missingRoute
    #[facet(args::named, default)]
    kind: Option<String>,

    /// Only include link-target diagnostics
    #[facet(args::named, default)]
    dead_links: bool,

    /// Exit non-zero when diagnostics are present
    #[facet(args::named, default)]
    fail: bool,
}

#[derive(Facet, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[facet(rename_all = "snake_case")]
#[repr(u8)]
enum DiagnosticsFormat {
    #[default]
    Text,
    Json,
}

/// Coverage command arguments
#[derive(Facet, Debug)]
struct CoverageArgs {
    #[facet(args::subcommand)]
    command: CoverageCommand,
}

#[derive(Facet, Debug)]
#[repr(u8)]
enum CoverageCommand {
    /// Print coverage summary
    Status(CoverageQueryArgs),
    /// Print configured coverage implementation globs
    Config(CoverageQueryArgs),
    /// List rules without implementation references
    Uncovered(CoverageQueryArgs),
    /// List rules without verification references
    Untested(CoverageQueryArgs),
    /// List code units without requirement references
    Unmapped(CoverageQueryArgs),
    /// List references to older rule versions
    Stale(CoverageQueryArgs),
    /// List references that do not resolve to a known rule
    Invalid(CoverageQueryArgs),
    /// Print coverage for one rule
    Rule(CoverageRuleArgs),
    /// Validate coverage and exit non-zero on failure
    Validate(CoverageValidateArgs),
}

#[derive(Facet, Debug)]
struct CoverageQueryArgs {
    #[facet(flatten)]
    common: CoverageCommonArgs,
}

#[derive(Facet, Debug)]
struct CoverageRuleArgs {
    /// Rule id to inspect
    #[facet(args::positional)]
    id: String,

    #[facet(flatten)]
    common: CoverageCommonArgs,
}

#[derive(Facet, Debug)]
struct CoverageValidateArgs {
    #[facet(flatten)]
    common: CoverageCommonArgs,

    /// Minimum implementation coverage percentage
    #[facet(args::named, default)]
    threshold: Option<u8>,
}

#[derive(Facet, Debug)]
struct CoverageCommonArgs {
    /// Project directory (looks for .config/dodeca.styx here)
    #[facet(args::positional, default)]
    path: Option<String>,

    /// Content directory (uses .config/dodeca.styx if not specified)
    #[facet(args::named, args::short = 'c', default)]
    content: Option<String>,

    /// Output directory (uses .config/dodeca.styx if not specified)
    #[facet(args::named, args::short = 'o', default)]
    output: Option<String>,

    /// Output format
    #[facet(args::named, default)]
    format: CoverageCliFormat,

    /// Restrict coverage to one configured source name
    #[facet(args::named, default)]
    source: Option<String>,

    /// Restrict coverage to one configured implementation
    #[facet(args::named, rename = "impl", default)]
    impl_name: Option<String>,
}

#[derive(Facet, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[facet(rename_all = "snake_case")]
#[repr(u8)]
enum CoverageCliFormat {
    Json,
    #[default]
    Markdown,
}

impl From<CoverageCliFormat> for dodeca::coverage::CoverageOutputFormat {
    fn from(format: CoverageCliFormat) -> Self {
        match format {
            CoverageCliFormat::Json => dodeca::coverage::CoverageOutputFormat::Json,
            CoverageCliFormat::Markdown => dodeca::coverage::CoverageOutputFormat::Markdown,
        }
    }
}

/// Static file server arguments
#[derive(Facet, Debug)]
struct StaticArgs {
    /// Directory to serve
    #[facet(args::positional, default = ".".to_string())]
    path: String,

    /// Address to bind on
    #[facet(args::named, args::short = 'a', default = "127.0.0.1".to_string())]
    address: String,

    /// Port to serve on (default: 8080)
    #[facet(args::named, args::short = 'p', default)]
    port: Option<u16>,

    /// Open browser after starting server
    #[facet(args::named)]
    open: bool,

    /// Start with public access enabled (listen on all interfaces)
    #[facet(args::named, args::short = 'P', rename = "public")]
    public_access: bool,
}

/// Init command arguments
#[derive(Facet, Debug)]
struct InitArgs {
    /// Project name (creates directory with this name)
    #[facet(args::positional)]
    name: String,

    /// Template to use (skips interactive selection)
    #[facet(args::named, args::short = 't', default)]
    template: Option<String>,

    /// Skip initialising Git in the new directory
    #[facet(args::named, rename = "skip-git-init", default)]
    skip_git_init: bool,
}

/// Term command arguments
#[derive(Facet, Debug)]
struct TermArgs {
    /// Command to execute (everything after --)
    #[facet(args::positional, default)]
    command: Vec<String>,

    /// Output file path (default: /tmp/ddc-term)
    #[facet(args::named, args::short = 'o', default)]
    output: Option<String>,

    /// Skip clipboard copy
    #[facet(args::named)]
    no_clipboard: bool,
}

/// Agent guide arguments
#[derive(Facet, Debug)]
struct AgentArgs {}

/// Available commands
#[derive(Facet, Debug)]
#[repr(u8)]
enum Command {
    /// Create a new project
    Init(InitArgs),
    /// Build the site
    Build(BuildArgs),
    /// Build and serve with live reload
    Serve(ServeArgs),
    /// Serve static files from a directory
    Static(StaticArgs),
    /// Clear caches (use --all for full clean)
    Clean(CleanArgs),
    /// Run the authoring LSP server over stdio
    Lsp(LspArgs),
    /// Print authoring diagnostics
    Diagnostics(DiagnosticsArgs),
    /// Query requirement coverage
    Coverage(CoverageArgs),
    /// Print the bundled guide for agents working on Dodeca projects
    Agent(AgentArgs),
    /// Inspect or migrate `.config/dodeca.styx`
    Config(ConfigArgs),
    /// Record terminal session as HTML
    Term(TermArgs),
}

#[derive(Facet, Debug)]
struct ConfigArgs {
    #[facet(args::subcommand)]
    command: ConfigCommand,
}

#[derive(Facet, Debug)]
#[repr(u8)]
enum ConfigCommand {
    /// Rewrite a deprecated v1 config to the `source`/`site`/`mounts` format
    Migrate(MigrateArgs),
}

#[derive(Facet, Debug)]
struct MigrateArgs {
    /// Project dir or path to a `.config/dodeca.styx` (default: discover from CWD)
    #[facet(args::positional, default)]
    path: Option<String>,

    /// Write the migrated config back in place instead of printing to stdout
    #[facet(args::named, args::short = 'w', default)]
    write: bool,
}

/// Resolved configuration for a build
struct ResolvedBuildConfig {
    content_dir: Utf8PathBuf,
    output_dir: Utf8PathBuf,
    /// Project root (`.config/` parent). `impls` code globs resolve against it.
    root: Utf8PathBuf,
    /// All content sources (mount + content dir). For a single-source project
    /// this is one entry at mount `/` whose content dir equals `content_dir`.
    sources: Vec<dodeca::config::ResolvedSource>,
    skip_domains: Vec<String>,
    rate_limit_ms: Option<u64>,
    link_check_mode: LinkCheckMode,
    stable_assets: Vec<String>,
}

#[derive(Facet)]
struct CliDiagnostic {
    source_file: String,
    route: String,
    kind: String,
    target: String,
    resolved_route: Option<String>,
    message: String,
    span: CliDiagnosticSpan,
}

#[derive(Facet)]
struct CliDiagnosticSpan {
    line_start: u32,
    line_end: u32,
    column_start: u32,
    column_end: u32,
    byte_start: usize,
    byte_end: usize,
}

/// Resolve content and output directories from CLI args or config file
fn resolve_dirs(
    path: Option<String>,
    content: Option<String>,
    output: Option<String>,
) -> Result<ResolvedBuildConfig> {
    // Convert to Utf8PathBuf
    let path = path.map(Utf8PathBuf::from);
    let content = content.map(Utf8PathBuf::from);
    let output = output.map(Utf8PathBuf::from);

    // If both content and output are specified, use them directly (no config file needed)
    if let (Some(c), Some(o)) = (&content, &output) {
        return Ok(ResolvedBuildConfig {
            content_dir: c.clone(),
            output_dir: o.clone(),
            root: c
                .parent()
                .map(Utf8Path::to_owned)
                .unwrap_or_else(|| c.clone()),
            sources: vec![dodeca::config::ResolvedSource {
                name: String::new(),
                mount: "/".to_string(),
                content_dir: c.clone(),
                checkout_dir: None,
                git: None,
                repo: None,
                impls: Vec::new(),
                skip_domains: Vec::new(),
                project_dir: c
                    .parent()
                    .map(Utf8Path::to_owned)
                    .unwrap_or_else(|| c.clone()),
                build_steps: Default::default(),
                page_types: Default::default(),
            }],
            skip_domains: vec![],
            rate_limit_ms: None,
            link_check_mode: LinkCheckMode::default(),
            stable_assets: vec![],
        });
    }

    // Try to find config file, optionally from a specific path
    let config = if let Some(ref project_path) = path {
        ResolvedConfig::discover_from(project_path)?
    } else {
        ResolvedConfig::discover()?
    };

    match config {
        Some(cfg) => {
            // Initialize global config for access from render pipeline
            dodeca::config::set_global_config(cfg.clone())?;
            let root = cfg._root.clone();
            // A CLI `--content` override collapses to a single source at `/`;
            // otherwise use the config's resolved sources.
            let cli_content_override = content.is_some();
            let content_dir = content.unwrap_or(cfg.content_dir);
            let output_dir = output.unwrap_or(cfg.output_dir);
            let sources = if cli_content_override {
                vec![dodeca::config::ResolvedSource {
                    name: String::new(),
                    mount: "/".to_string(),
                    content_dir: content_dir.clone(),
                    checkout_dir: None,
                    git: None,
                    repo: None,
                    impls: Vec::new(),
                    skip_domains: Vec::new(),
                    project_dir: root.clone(),
                    build_steps: Default::default(),
                    page_types: Default::default(),
                }]
            } else {
                cfg.sources
            };
            Ok(ResolvedBuildConfig {
                content_dir,
                output_dir,
                root,
                sources,
                skip_domains: cfg.skip_domains,
                rate_limit_ms: cfg.rate_limit_ms,
                link_check_mode: cfg.link_check_mode,
                stable_assets: cfg.stable_assets,
            })
        }
        None => Err(eyre!("No configuration found.")),
    }
}

#[allow(clippy::disallowed_methods)] // Entry point - needs manual runtime management
fn main() -> Result<()> {
    // Install SIGUSR1 handler for debugging (dumps stack traces)
    // (no-op on non-Unix platforms)
    dodeca_debug::install_sigusr1_handler("ddc");

    // There is no internal shared-memory channel/connection registry to dump on
    // SIGUSR1; `dodeca_debug` still dumps host stacks.

    // When spawned by test harness with DODECA_DIE_WITH_PARENT=1, install death-watch
    // so we exit when the test process dies. This prevents orphan accumulation.
    if std::env::var("DODECA_DIE_WITH_PARENT").is_ok() {
        ur_taking_me_with_you::die_with_parent();
    }

    // Parse CLI args with layered configuration support (CLI > env > file > defaults)
    let cli_args: Vec<String> = std::env::args().skip(1).collect();
    let config = args::builder::<Args>()
        .expect("failed to build args schema")
        .cli(|cli| cli.args(cli_args))
        .env(|env| env.prefix("DDC"))
        .help(|h| h.program_name("ddc").version(dodeca::dodeca_version()))
        .build();

    let args = args::Driver::new(config).run().unwrap();

    // Single runtime for all commands
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async_main(args.command))
}

async fn async_main(command: Command) -> Result<()> {
    match command {
        Command::Build(args) => {
            // Initialize tracing early so config errors are visible
            logging::init_standard_tracing();

            let cli_mode = args.link_check;
            let cfg = resolve_dirs(args.path, args.content, args.output)?;

            // Run Vite build for every source that ships its own vite project
            // (the primary's content_dir.parent() plus each mounted source's,
            // e.g. styx-docs/), before dodeca build so the dist assets exist.
            // `maybe_run_vite_build` is a no-op where there's no vite.config.
            //
            // The primary's build is fatal; a mounted source's is not — a subsite
            // whose JS toolchain can't build here shouldn't sink the whole
            // aggregate. Its bundle is simply absent (the page still renders).
            for (i, source) in cfg.sources.iter().enumerate() {
                let project_dir = source.content_dir.parent().unwrap_or(&source.content_dir);
                let res = vite::maybe_run_vite_build(project_dir.as_std_path()).await;
                if i == 0 {
                    res?;
                } else if let Err(e) = res {
                    tracing::warn!(
                        source = %source.name,
                        "vite build failed for mounted source; its bundle will be \
                         unavailable (the rest of the site builds): {e}"
                    );
                }
            }

            // CLI flag wins over `link_check.mode` in dodeca.styx.
            let mode = cli_mode.unwrap_or(cfg.link_check_mode);
            let link_check = match mode {
                LinkCheckMode::None => LinkCheckOptions::None,
                LinkCheckMode::Internal => LinkCheckOptions::InternalOnly,
                LinkCheckMode::Full => LinkCheckOptions::Full {
                    skip_domains: cfg.skip_domains,
                    rate_limit_ms: cfg.rate_limit_ms,
                },
            };

            let options = BuildOptions {
                render_options: render::RenderOptions {
                    livereload: false,
                    dev_mode: false,
                    source_maps: false,
                    render_notes: false,
                },
                progress: None,
                link_check,
            };

            build(
                &cfg.content_dir,
                &cfg.output_dir,
                &cfg.sources,
                &cfg.root,
                options,
            )
            .await?;
            Ok(())
        }
        Command::Serve(args) => {
            // Check if we should use TUI
            use std::io::IsTerminal;
            let use_tui = args.force_tui || (!args.no_tui && std::io::stdout().is_terminal());

            if !use_tui {
                // Initialize tracing early so config errors are visible
                logging::init_standard_tracing();
            }

            let cfg = resolve_dirs(args.path, args.content, args.output)?;

            // Fallback git-pull poller (webhooks preferred). Runs alongside the
            // server; the file watcher re-renders whatever it pulls.
            if let Some(secs) = args.git_poll {
                spawn_git_poll(&cfg.sources, secs);
            }

            // Local-only dev editor: never on a publicly-reachable bind.
            if args.dev_editor.is_some()
                && (args.public_access || !is_loopback_address(&args.address))
            {
                return Err(eyre!(
                    "--dev-editor is local-only; refusing on bind `{}`{}",
                    args.address,
                    if args.public_access {
                        " with --public"
                    } else {
                        ""
                    }
                ));
            }

            if use_tui {
                if args.fd_socket.is_some() {
                    return Err(eyre!("FD passing is only supported in --no-tui mode"));
                }
                serve_with_tui(
                    &cfg.output_dir,
                    &cfg.sources,
                    Bind {
                        address: args.address.clone(),
                        port: args.port,
                    },
                    args.open,
                    cfg.stable_assets,
                    args.public_access,
                    args.dev_editor,
                )
                .await
            } else {
                serve_plain(
                    &cfg.sources,
                    Bind {
                        address: args.address.clone(),
                        port: args.port,
                    },
                    args.open,
                    cfg.stable_assets,
                    args.fd_socket,
                    args.public_access,
                    args.dev_editor,
                )
                .await
            }
        }
        Command::Clean(args) => {
            use dodeca::config::ProjectPaths;

            // Get project paths from config
            let paths = if let Some(p) = args.path {
                // Manual path - create minimal ProjectPaths
                let root = Utf8PathBuf::from(p);
                ProjectPaths::from_config(
                    &ResolvedConfig::discover_from(&root)?
                        .ok_or_else(|| eyre!("No dodeca config found in {}", root))?,
                )
            } else if let Some(cfg) = ResolvedConfig::discover()? {
                cfg.paths()
            } else {
                return Err(eyre!("No dodeca project found"));
            };

            let clean_vite = args.vite || args.all;
            let clean_output = args.output || args.all;
            let clean_dist = args.dist || args.all;

            // Helper to clean a directory
            let clean_dir = |path: &Utf8Path, name: &str| -> Result<bool> {
                if path.exists() {
                    let size = dir_size(path);
                    fs::remove_dir_all(path)?;
                    println!("{} {} ({})", "Cleared:".green(), name, format_bytes(size));
                    Ok(true)
                } else {
                    println!("{} {} (not found)", "Skipped:".dimmed(), name);
                    Ok(false)
                }
            };

            // Remove .cache directory (contains CAS, picante DB, image cache)
            let cache_rel = paths
                .cache
                .strip_prefix(&paths.root)
                .map(|p| format!("{}/", p))
                .unwrap_or_else(|_| ".cache/".to_string());
            clean_dir(&paths.cache, &cache_rel)?;

            // Remove output directory
            if clean_output {
                let output_rel = paths
                    .output
                    .strip_prefix(&paths.root)
                    .map(|p| format!("{}/", p))
                    .unwrap_or_else(|_| "output/".to_string());
                clean_dir(&paths.output, &output_rel)?;
            }

            // Remove Vite dist and cache
            let vite_prefix = paths.vite_prefix();
            if let Some(ref vite_dist) = paths.vite_dist {
                if clean_dist {
                    clean_dir(vite_dist, &format!("{}dist/", vite_prefix))?;
                }
            } else if clean_dist {
                println!("{} dist/ (no Vite project found)", "Skipped:".dimmed());
            }

            if let Some(ref vite_cache) = paths.vite_cache {
                if clean_vite {
                    clean_dir(vite_cache, &format!("{}node_modules/.vite/", vite_prefix))?;
                }
            } else if clean_vite {
                println!(
                    "{} node_modules/.vite/ (no Vite project found)",
                    "Skipped:".dimmed()
                );
            }

            Ok(())
        }
        Command::Lsp(args) => {
            logging::init_standard_tracing();
            // Back the standalone LSP with a loaded picante db + VFS overlays
            // (the same machinery the in-process browser-editor LSP uses), not
            // the disk "world" model.
            let (db, sources) = load_lsp_db(None, args.content.clone(), args.output.clone())?;
            let provider = Arc::new(dodeca::authoring_model::DbAuthoringProvider { db, sources });
            dodeca_authoring_lsp::run_with_provider(args.content, args.output, provider).await
        }
        Command::Diagnostics(args) => {
            logging::init_standard_tracing();
            run_diagnostics(args).await
        }
        Command::Coverage(args) => {
            logging::init_standard_tracing();
            run_coverage(args).await
        }
        Command::Agent(args) => run_agent(args),
        Command::Static(args) => {
            let path = Utf8PathBuf::from(&args.path);
            if !path.exists() {
                return Err(eyre!("Directory does not exist: {}", path));
            }
            if !path.is_dir() {
                return Err(eyre!("Not a directory: {}", path));
            }

            serve_static(
                &path,
                &args.address,
                args.port,
                args.open,
                args.public_access,
            )
            .await
        }
        Command::Init(args) => {
            // Initialize tracing early so errors are visible
            logging::init_standard_tracing();

            init::run_init(args.name, args.template, !args.skip_git_init).await
        }
        Command::Config(args) => {
            logging::init_standard_tracing();
            match args.command {
                ConfigCommand::Migrate(margs) => run_config_migrate(margs),
            }
        }
        Command::Term(args) => run_term(args).await,
    }
}

fn run_agent(_args: AgentArgs) -> Result<()> {
    print!("{AGENT_GUIDE}");
    if !AGENT_GUIDE.ends_with('\n') {
        println!();
    }
    Ok(())
}

/// `ddc config migrate` — rewrite a deprecated v1 config to the new format.
fn run_config_migrate(args: MigrateArgs) -> Result<()> {
    // Resolve the config file: an explicit `.styx` file, a project dir, or
    // discover from CWD.
    let config_path = match args.path {
        Some(p) => {
            let p = Utf8PathBuf::from(p);
            if p.extension() == Some("styx") {
                p
            } else {
                p.join(".config").join("dodeca.styx")
            }
        }
        None => dodeca::config::find_config_file()?
            .ok_or_else(|| eyre!("no .config/dodeca.styx found (pass a path)"))?,
    };
    if !config_path.exists() {
        return Err(eyre!("config not found: {config_path}"));
    }

    let content = std::fs::read_to_string(&config_path)?;
    let (modern, legacy) = dodeca_config::parse_config(&content)
        .map_err(|e| eyre!("failed to parse {config_path}: {e}"))?;
    if !legacy {
        println!("{config_path} is already in the new format; nothing to migrate.");
        return Ok(());
    }

    // Serialize with `omit_none` (drop absent options instead of writing them as
    // `@`, which is noise and doesn't round-trip), then pretty-reformat the text.
    let serialized = facet_styx::to_string_with_options(
        &modern,
        &facet_styx::SerializeOptions::default().omit_none(),
    )
    .map_err(|e| eyre!("failed to serialize migrated config: {e}"))?;
    let body = facet_styx::format_source(
        &serialized,
        facet_styx::SerializeOptions::default().pretty(80),
    );
    let migrated = format!("@schema {{id crate:dodeca-config@1, cli ddc}}\n\n{body}");

    if args.write {
        std::fs::write(&config_path, &migrated)?;
        eprintln!(
            "migrated {config_path} to the new format (comments were not preserved; review the diff)."
        );
    } else {
        print!("{migrated}");
    }
    Ok(())
}

async fn run_diagnostics(args: DiagnosticsArgs) -> Result<()> {
    // Same db + VFS machinery as `ddc lsp` (no overlays): build the project from
    // a loaded picante db, not the disk "world" model.
    let (db, sources) = load_lsp_db(args.path, args.content, None)?;
    let snapshot = dodeca::authoring_model::overlay_snapshot(&db, &sources, Vec::new()).await?;
    let mut diagnostics = authoring_lsp::authoring_diagnostics_for_snapshot(&snapshot).await?;

    diagnostics.retain(|diagnostic| {
        if args.dead_links && !is_dead_link_diagnostic(diagnostic.kind) {
            return false;
        }

        if let Some(kind) = args.kind.as_deref()
            && authoring_lsp::diagnostic_kind_name(diagnostic.kind) != kind
        {
            return false;
        }

        true
    });

    match args.format {
        DiagnosticsFormat::Text => print_diagnostics_text(&diagnostics),
        DiagnosticsFormat::Json => print_diagnostics_json(&diagnostics)?,
    }

    if args.fail && !diagnostics.is_empty() {
        return Err(eyre!("Found {} authoring diagnostic(s)", diagnostics.len()));
    }

    Ok(())
}

async fn run_coverage(args: CoverageArgs) -> Result<()> {
    let (endpoint, common, validate_threshold) = coverage_command_parts(args.command);
    let format = common.format.into();
    let selector =
        dodeca::coverage::CoverageSelector::new(common.source.clone(), common.impl_name.clone());
    let (db, _) = load_lsp_db(common.path, common.content, common.output)?;
    let workspace = db::TASK_DB
        .scope(db.clone(), queries::coverage_workspace(&*db))
        .await?;
    let report = workspace
        .report_for_selector(&selector)
        .ok_or_else(|| eyre!("coverage selection did not match any source/impl"))?;
    let output = dodeca::coverage::coverage_output(&report, endpoint, format)
        .map_err(|err| eyre!("failed to render coverage output: {err}"))?
        .ok_or_else(|| eyre!("coverage query did not match any rule"))?;
    print!("{}", output.body);
    if !output.body.ends_with('\n') {
        println!();
    }

    if let Some(threshold) = validate_threshold {
        let status = dodeca::coverage::status_response(&report);
        let passing = report.invalid_references.is_empty()
            && report.stale_references.is_empty()
            && report.test_impl_references.is_empty()
            && status.implementation_coverage_percent >= f64::from(threshold);
        if !passing {
            return Err(eyre!("coverage validation failed"));
        }
    }

    Ok(())
}

fn coverage_command_parts(
    command: CoverageCommand,
) -> (
    dodeca::coverage::CoverageEndpoint,
    CoverageCommonArgs,
    Option<u8>,
) {
    match command {
        CoverageCommand::Status(args) => (
            dodeca::coverage::CoverageEndpoint::Status,
            args.common,
            None,
        ),
        CoverageCommand::Config(args) => (
            dodeca::coverage::CoverageEndpoint::Config,
            args.common,
            None,
        ),
        CoverageCommand::Uncovered(args) => (
            dodeca::coverage::CoverageEndpoint::Uncovered,
            args.common,
            None,
        ),
        CoverageCommand::Untested(args) => (
            dodeca::coverage::CoverageEndpoint::Untested,
            args.common,
            None,
        ),
        CoverageCommand::Unmapped(args) => (
            dodeca::coverage::CoverageEndpoint::Unmapped,
            args.common,
            None,
        ),
        CoverageCommand::Stale(args) => {
            (dodeca::coverage::CoverageEndpoint::Stale, args.common, None)
        }
        CoverageCommand::Invalid(args) => (
            dodeca::coverage::CoverageEndpoint::Invalid,
            args.common,
            None,
        ),
        CoverageCommand::Rule(args) => (
            dodeca::coverage::CoverageEndpoint::Rule { id: args.id },
            args.common,
            None,
        ),
        CoverageCommand::Validate(args) => (
            dodeca::coverage::CoverageEndpoint::Validate {
                threshold: args.threshold,
            },
            args.common,
            Some(args.threshold.unwrap_or(0)),
        ),
    }
}

fn is_dead_link_diagnostic(kind: AuthoringDiagnosticKind) -> bool {
    matches!(
        kind,
        AuthoringDiagnosticKind::Route
            | AuthoringDiagnosticKind::Anchor
            | AuthoringDiagnosticKind::Source
            | AuthoringDiagnosticKind::StaticAsset
    )
}

fn print_diagnostics_text(diagnostics: &[AuthoringDiagnostic]) {
    if diagnostics.is_empty() {
        println!("No authoring diagnostics found");
        return;
    }

    for diagnostic in diagnostics {
        println!(
            "{}:{}:{} [{}] {}",
            diagnostic.source_file.yellow(),
            diagnostic.line,
            diagnostic.column,
            authoring_lsp::diagnostic_kind_name(diagnostic.kind).cyan(),
            diagnostic.message
        );
    }
}

fn print_diagnostics_json(diagnostics: &[AuthoringDiagnostic]) -> Result<()> {
    let diagnostics = diagnostics.iter().map(cli_diagnostic).collect::<Vec<_>>();
    let json = facet_json::to_string_pretty(&diagnostics)
        .map_err(|err| eyre!("failed to serialize diagnostics: {err:?}"))?;
    println!("{json}");
    Ok(())
}

fn cli_diagnostic(diagnostic: &AuthoringDiagnostic) -> CliDiagnostic {
    CliDiagnostic {
        source_file: diagnostic.source_file.clone(),
        route: diagnostic.route.clone(),
        kind: authoring_lsp::diagnostic_kind_name(diagnostic.kind).to_string(),
        target: diagnostic.target.clone(),
        resolved_route: diagnostic.resolved_route.clone(),
        message: diagnostic.message.clone(),
        span: CliDiagnosticSpan {
            line_start: diagnostic.line,
            line_end: diagnostic.line_end,
            column_start: diagnostic.column,
            column_end: diagnostic.column_end,
            byte_start: diagnostic.byte_start,
            byte_end: diagnostic.byte_end,
        },
    }
}

/// Run terminal recording
async fn run_term(args: TermArgs) -> Result<()> {
    use cell_term_proto::RecordConfig;

    let output_path = args.output.unwrap_or_else(|| "/tmp/ddc-term".to_string());
    let copy_to_clipboard = !args.no_clipboard;

    let config = RecordConfig { shell: None };

    let result = if args.command.is_empty() {
        // Interactive mode
        println!(
            "{} Recording terminal session. Type {} to exit.",
            "→".cyan(),
            "exit".yellow()
        );
        cells::record_term_interactive(config).await?
    } else {
        // Command mode - join the command parts
        let command = args.command.join(" ");
        println!("{} Recording: {}", "→".cyan(), command.yellow());
        cells::record_term_command(command, config).await?
    };

    match result {
        cell_term_proto::TermResult::Success { html } => {
            // Write output
            std::fs::write(&output_path, &html)?;
            println!("\n{} Written to {}", "✓".green(), output_path.cyan());

            // Copy to clipboard if requested (wrapped in ```term fence for markdown)
            if copy_to_clipboard {
                let clipboard_content = format!("```term\n{}\n```", html);
                match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(&clipboard_content)) {
                    Ok(()) => println!("{} Copied to clipboard", "✓".green()),
                    Err(e) => eprintln!("{} Could not copy to clipboard: {}", "⚠".yellow(), e),
                }
            }
            Ok(())
        }
        cell_term_proto::TermResult::Error { message } => Err(eyre!("{}", message)),
    }
}

/// Calculate total size of a directory recursively
fn dir_size(path: &Utf8Path) -> usize {
    WalkBuilder::new(path)
        .build()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len() as usize)
        .sum()
}

/// Format bytes as human-readable size
fn format_bytes(bytes: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = KB * 1024;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Print server URLs with terminal hyperlinks
fn print_server_urls(address: &str, port: u16) {
    println!("\n{}", "Server running at:".bold());

    if address == "0.0.0.0" {
        // List all interfaces
        if let Ok(interfaces) = if_addrs::get_if_addrs() {
            for iface in interfaces {
                if let if_addrs::IfAddr::V4(addr) = iface.addr {
                    let ip = addr.ip;
                    let url = format!("http://{ip}:{port}");
                    println!("  {} {}", "→".cyan(), terminal_link(&url, &url));
                }
            }
        }
    } else {
        let url = format!("http://{address}:{port}");
        println!("  {} {}", "→".cyan(), terminal_link(&url, &url));
    }
    println!();
}

/// Create an OSC 8 terminal hyperlink
fn terminal_link(url: &str, text: &str) -> String {
    format!(
        "\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\",
        url,
        text.blue().underline()
    )
}

/// Link checking options for build
#[derive(Clone, Default)]
pub enum LinkCheckOptions {
    /// No link checking
    #[default]
    None,
    /// Internal links only (fast, no network)
    InternalOnly,
    /// Full link checking (internal + external)
    Full {
        skip_domains: Vec<String>,
        rate_limit_ms: Option<u64>,
    },
}

/// Options for the build function
pub struct BuildOptions {
    /// Render options (livereload, dev_mode)
    pub render_options: render::RenderOptions,
    /// Optional TUI progress reporter
    pub progress: Option<tui::ProgressReporter>,
    /// Link checking configuration
    pub link_check: LinkCheckOptions,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            render_options: render::RenderOptions::default(),
            progress: None,
            link_check: LinkCheckOptions::None,
        }
    }
}

// inject_livereload is now in render.rs
use render::inject_livereload_with_build_info;

/// Get the output path for an HTML route
fn route_to_path(output_dir: &Utf8Path, route: &Route) -> Utf8PathBuf {
    let route_str = route.as_str().trim_matches('/');
    if route_str.is_empty() {
        output_dir.join("index.html")
    } else {
        output_dir.join(route_str).join("index.html")
    }
}

/// Build statistics
#[derive(Debug, Default)]
pub struct BuildStats {
    pub html_written: usize,
    pub html_skipped: usize,
    pub css_written: bool,
    pub css_skipped: bool,
    pub static_written: usize,
    pub static_skipped: usize,
}

/// Format bytes as human-readable size
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Load picante cache from disk (shared logic for build and serve)
async fn load_picante_cache(db: &Database, cache_path: &Utf8Path) {
    use serve::PICANTE_CACHE_VERSION;
    use std::time::Instant;

    // Check version file first - if missing or mismatched, delete the cache
    let version_path = cache_path.with_extension("version");
    let version_ok = if version_path.exists() {
        match std::fs::read_to_string(&version_path) {
            Ok(v) => v.trim().parse::<u32>().ok() == Some(PICANTE_CACHE_VERSION),
            Err(_) => false,
        }
    } else {
        false
    };

    if !version_ok {
        if cache_path.exists() {
            tracing::info!(
                "Picante cache version mismatch (expected v{}), deleting stale cache",
                PICANTE_CACHE_VERSION
            );
            let _ = std::fs::remove_file(cache_path);
        }
        return;
    }

    if !cache_path.exists() {
        tracing::debug!("No picante cache file found, starting fresh");
        return;
    }

    // Get file size before loading
    let file_size = cache_path.metadata().map(|m| m.len()).unwrap_or(0);

    let start = Instant::now();
    match db.load_from_cache(cache_path.as_std_path()).await {
        Ok(true) => {
            let elapsed = start.elapsed();
            tracing::info!(
                "Loaded picante cache ({}) in {:.2?}",
                format_size(file_size),
                elapsed
            );
        }
        Ok(false) => {
            tracing::debug!("No cache file found");
        }
        Err(e) => {
            tracing::warn!("Failed to load picante cache: {:?}", e);
        }
    }
}

/// Save picante cache to disk (shared logic for build and serve)
async fn save_picante_cache(db: &Database, cache_path: &Utf8Path) {
    use serve::PICANTE_CACHE_VERSION;
    use std::time::Instant;

    // Ensure cache directory exists
    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Write version file
    let version_path = cache_path.with_extension("version");
    if let Err(e) = std::fs::write(&version_path, PICANTE_CACHE_VERSION.to_string()) {
        tracing::warn!("Failed to write cache version file: {}", e);
    }

    let start = Instant::now();
    match db.save_to_cache(cache_path.as_std_path()).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            let file_size = cache_path.metadata().map(|m| m.len()).unwrap_or(0);
            tracing::info!(
                "Saved picante cache ({}) in {:.2?}",
                format_size(file_size),
                elapsed
            );
        }
        Err(e) => {
            tracing::warn!("Failed to save picante cache: {:?}", e);
        }
    }
}

/// Build a loaded picante db for the LSP: resolve dirs, then load every input
/// registry (sources, templates, sass, static, data, code) — the same inputs
/// `build()` loads, without rendering. Returns the db and its sources, which
/// back a `DbAuthoringProvider` for the standalone LSP.
fn load_lsp_db(
    path: Option<String>,
    content: Option<String>,
    output: Option<String>,
) -> Result<(Arc<Database>, Vec<dodeca::config::ResolvedSource>)> {
    let cfg = resolve_dirs(path, content, output)?;
    let mut ctx = BuildContext::new(&cfg.content_dir, &cfg.output_dir);
    ctx.set_source_roots(cfg.sources.clone());
    ctx.set_project_root(&cfg.root);
    if let Some(global) = dodeca::config::global_config() {
        ConfigRegistry::set(&*ctx.db, global)?;
    }
    MarkdownRenderSettings::set(&*ctx.db, false, true)?;
    ctx.load_sources()?;
    ctx.load_templates()?;
    ctx.load_sass()?;
    ctx.load_static()?;
    ctx.load_data()?;
    ctx.load_code()?;
    SourceRegistry::set(&*ctx.db, ctx.sources.values().copied().collect())?;
    TemplateRegistry::set(&*ctx.db, ctx.templates.values().copied().collect())?;
    SassRegistry::set(&*ctx.db, ctx.sass_files.values().copied().collect())?;
    StaticRegistry::set(&*ctx.db, ctx.static_files.values().copied().collect())?;
    DataRegistry::set(&*ctx.db, ctx.data_files.values().copied().collect())?;
    CodeRegistry::set(&*ctx.db, ctx.code_files.values().copied().collect())?;
    CodeCoverageRegistry::set(&*ctx.db, ctx.code_coverage_entries.clone())?;
    Ok((ctx.db_arc(), cfg.sources))
}

pub async fn build(
    content_dir: &Utf8PathBuf,
    output_dir: &Utf8PathBuf,
    sources: &[dodeca::config::ResolvedSource],
    project_root: &Utf8Path,
    options: BuildOptions,
) -> Result<BuildContext> {
    use std::time::Instant;

    let start = Instant::now();
    let verbose = options.progress.is_none(); // Print to stdout when no TUI progress
    let render_options = options.render_options;

    // Clone any git-backed source that isn't checked out yet.
    ensure_git_sources(sources)?;

    // Open content-addressed storage at base dir
    let base_dir = content_dir.parent().unwrap_or(content_dir);
    // Initialize cache directory
    let cache_dir = base_dir.join(".cache");

    let cas_path = cache_dir.join("cas.db");
    let mut store = cas::ContentStore::open(&cas_path)?;
    cas::init_asset_cache(cache_dir.as_std_path())?;

    // Create query stats for tracking
    let query_stats = QueryStats::new();
    let mut ctx = BuildContext::with_stats(content_dir, output_dir, Some(Arc::clone(&query_stats)));
    ctx.set_source_roots(sources.to_vec());
    ctx.set_project_root(project_root);

    // Load picante cache from disk (for font subsetting, image processing, etc.)
    let picante_cache_path = cache_dir.join("dodeca.bin");
    load_picante_cache(&ctx.db, &picante_cache_path).await;
    MarkdownRenderSettings::set(
        &*ctx.db,
        render_options.source_maps,
        render_options.render_notes,
    )?;

    // Phase 1: Load everything into picante
    ctx.load_sources()?;
    ctx.load_templates()?;
    ctx.load_sass()?;
    ctx.load_static()?;
    ctx.load_data()?;
    ctx.load_code()?;

    if verbose {
        println!(
            "{} {} sources, {} templates, {} sass, {} static",
            "Loaded".cyan(),
            ctx.sources.len(),
            ctx.templates.len(),
            ctx.sass_files.len(),
            ctx.static_files.len()
        );
    }

    // Set registries as singletons in the database
    let source_vec: Vec<_> = ctx.sources.values().copied().collect();
    let template_vec: Vec<_> = ctx.templates.values().copied().collect();
    let sass_vec: Vec<_> = ctx.sass_files.values().copied().collect();
    let static_vec: Vec<_> = ctx.static_files.values().copied().collect();
    let data_vec: Vec<_> = ctx.data_files.values().copied().collect();

    let code_vec: Vec<_> = ctx.code_files.values().copied().collect();
    let has_code = !code_vec.is_empty();

    SourceRegistry::set(&*ctx.db, source_vec)?;
    TemplateRegistry::set(&*ctx.db, template_vec)?;
    SassRegistry::set(&*ctx.db, sass_vec)?;
    StaticRegistry::set(&*ctx.db, static_vec)?;
    DataRegistry::set(&*ctx.db, data_vec)?;
    CodeRegistry::set(&*ctx.db, code_vec)?;
    CodeCoverageRegistry::set(&*ctx.db, ctx.code_coverage_entries.clone())?;

    // Update progress: parsing phase
    if let Some(ref p) = options.progress {
        p.update(|prog| prog.parse.start(ctx.sources.len()));
    }

    // THE query - produces all outputs (fonts are automatically subsetted)
    let site_output = match db::TASK_DB
        .scope(ctx.db_arc(), build_site(&*ctx.db))
        .await?
    {
        Ok(output) => output,
        Err(site_error) => {
            // Format the error appropriately for CLI
            match site_error {
                queries::SiteError::Parse(build_error) => {
                    eprintln!(
                        "{} Failed to parse {} file(s):",
                        "✗".red(),
                        build_error.errors.len()
                    );
                    for err in &build_error.errors {
                        eprintln!("  {} {}: {}", "→".red(), err.path, err.error);
                    }
                }
                queries::SiteError::Render(render_error) => {
                    // Use ariadne for pretty ANSI formatting
                    let formatted = dodeca::error_pages::format_error_ansi(&render_error.error);
                    eprintln!(
                        "{} Error rendering {}:\n{}",
                        "✗".red(),
                        render_error.route,
                        formatted
                    );
                }
                queries::SiteError::WikiLinks(wiki_error) => {
                    eprintln!("{} {}", "✗".red(), wiki_error);
                }
            }
            std::process::exit(1);
        }
    };

    // Spec coverage summary — only meaningful when sources declare `impls`.
    if has_code {
        let report = db::TASK_DB
            .scope(ctx.db_arc(), queries::coverage_report(&*ctx.db))
            .await?;
        let invalid = report.invalid_references.len();
        println!(
            "{} {}/{} rules covered ({:.0}%){}",
            "Coverage".cyan(),
            report.covered_rules.len(),
            report.total_rules,
            report.coverage_percent(),
            if invalid == 0 {
                String::new()
            } else {
                format!(", {invalid} invalid ref(s)")
            }
        );
    }

    // Code execution validation
    let failed_executions: Vec<_> = site_output
        .code_execution_results
        .iter()
        .filter(|result| result.status == db::CodeExecutionStatus::Failed)
        .collect();

    if !failed_executions.is_empty() {
        for failure in &failed_executions {
            eprintln!(
                "{}Code execution failed in {}:{} ({}): {}",
                "✗ ".red(),
                failure.source_path,
                failure.line,
                failure.language,
                failure.error.as_deref().unwrap_or("Unknown error")
            );
            if !failure.stderr.is_empty() {
                eprintln!("  stderr: {}", failure.stderr);
            }
        }

        // In production mode, fail the build on code execution errors
        if !render_options.dev_mode {
            return Err(eyre!(
                "Build failed: {} code sample(s) failed execution",
                failed_executions.len()
            ));
        } else {
            eprintln!(
                "{}Warning: {} code sample(s) failed execution (continuing in dev mode)",
                "⚠ ".yellow(),
                failed_executions.len()
            );
        }
    } else if !site_output.code_execution_results.is_empty() {
        let executed = site_output
            .code_execution_results
            .iter()
            .filter(|r| r.status == db::CodeExecutionStatus::Success)
            .count();
        let skipped = site_output
            .code_execution_results
            .iter()
            .filter(|r| r.status == db::CodeExecutionStatus::Skipped)
            .count();

        if verbose {
            if skipped > 0 {
                println!(
                    "{} {} code samples executed successfully, {} skipped",
                    "✓".green(),
                    executed,
                    skipped
                );
            } else {
                println!(
                    "{} {} code samples executed successfully",
                    "✓".green(),
                    executed
                );
            }
        }
    }

    if let Some(ref p) = options.progress {
        p.update(|prog| {
            prog.parse.finish();
            prog.render.start(site_output.files.len());
        });
    }

    // Write outputs to disk, only if changed
    let mut stats = BuildStats::default();
    let expected_output_paths: HashSet<String> = site_output
        .files
        .iter()
        .map(|output| match output {
            OutputFile::Html { route, .. } => route_to_path(output_dir, route).to_string(),
            OutputFile::Css { path, .. } | OutputFile::Static { path, .. } => {
                output_dir.join(path.as_str()).to_string()
            }
        })
        .collect();

    for output in &site_output.files {
        match output {
            OutputFile::Html {
                route,
                content,
                head_injections,
                ..
            } => {
                // Apply livereload injection with build info (no dead link checking in build mode)
                let final_html = inject_livereload_with_build_info(
                    content,
                    render_options,
                    None,
                    &site_output.code_execution_results,
                    head_injections,
                )
                .await;
                let path = route_to_path(output_dir, route);

                if store.write_if_changed(&path, final_html.as_bytes())? {
                    stats.html_written += 1;
                } else {
                    stats.html_skipped += 1;
                }
            }
            OutputFile::Css { path, content } => {
                let dest = output_dir.join(path.as_str());
                if store.write_if_changed(&dest, content.as_bytes())? {
                    stats.css_written = true;
                }
            }
            OutputFile::Static { path, content } => {
                let dest = output_dir.join(path.as_str());
                if store.write_if_changed(&dest, content)? {
                    stats.static_written += 1;
                } else {
                    stats.static_skipped += 1;
                }
            }
        }
    }

    let stale_removed = store.remove_stale(&expected_output_paths)?;

    if let Some(ref p) = options.progress {
        p.update(|prog| {
            prog.render.finish();
            prog.sass.finish();
        });
    }

    if verbose {
        let up_to_date =
            stats.html_skipped + stats.static_skipped + if stats.css_written { 0 } else { 1 };

        let mut parts = Vec::new();
        if stats.html_written > 0 {
            parts.push(format!("{} HTML", stats.html_written));
        }
        if stats.static_written > 0 {
            parts.push(format!("{} static", stats.static_written));
        }
        if stats.css_written {
            parts.push("CSS".to_string());
        }
        if stale_removed > 0 {
            parts.push(format!("{stale_removed} stale"));
        }

        if parts.is_empty() {
            println!("{} ({} up-to-date)", "Wrote".cyan(), up_to_date);
        } else if up_to_date > 0 {
            println!(
                "{} {} ({} up-to-date)",
                "Wrote".cyan(),
                parts.join(", "),
                up_to_date
            );
        } else {
            println!("{} {}", "Wrote".cyan(), parts.join(", "));
        }
    }

    // Link checking based on options
    match &options.link_check {
        LinkCheckOptions::None => {
            // No link checking
        }
        LinkCheckOptions::InternalOnly => {
            // Check internal links only
            tracing::info!("Checking internal links...");
            let pages = site_output.files.iter().filter_map(|f| match f {
                OutputFile::Html {
                    route,
                    hrefs,
                    element_ids,
                    ..
                } => Some(link_checker::PreExtractedPage {
                    route,
                    hrefs,
                    element_ids,
                }),
                _ => None,
            });
            let extracted = link_checker::extract_links_from_preextracted(pages);
            let link_result = link_checker::check_internal_links(&extracted);

            if let Some(ref p) = options.progress {
                p.update(|prog| prog.links.finish());
            }

            if !link_result.is_ok() {
                for broken in &link_result.broken_links {
                    eprintln!(
                        "{}: {} -> {}",
                        broken.source_route.as_str().yellow(),
                        broken.href.red(),
                        broken.reason
                    );
                }
                return Err(eyre!(
                    "Found {} broken link(s)",
                    link_result.broken_links.len()
                ));
            }

            if verbose {
                println!(
                    "{} {} links ({} internal, {} external)",
                    "Checked".cyan(),
                    link_result.total_links,
                    link_result.internal_links,
                    link_result.external_links
                );
            }

            if let Some(ref p) = options.progress {
                p.update(|prog| prog.search.finish());
            }
        }
        LinkCheckOptions::Full {
            skip_domains,
            rate_limit_ms,
        } => {
            // Full link checking: internal + external
            tracing::info!("Checking links (internal + external)...");
            let pages = site_output.files.iter().filter_map(|f| match f {
                OutputFile::Html {
                    route,
                    hrefs,
                    element_ids,
                    ..
                } => Some(link_checker::PreExtractedPage {
                    route,
                    hrefs,
                    element_ids,
                }),
                _ => None,
            });
            let extracted = link_checker::extract_links_from_preextracted(pages);
            let mut link_result = link_checker::check_internal_links(&extracted);

            // Check external links with date-based caching
            let today = chrono::Local::now().date_naive();
            let mut external_options =
                link_checker::ExternalLinkOptions::new().skip_domains(skip_domains.iter().cloned());
            if let Some(ms) = rate_limit_ms {
                external_options = external_options.rate_limit_ms(*ms);
            }

            let extracted_external = extracted.external.clone();
            let known_routes = extracted.known_routes.clone();
            let (external_broken, external_checked) = {
                let ext = link_checker::ExtractedLinks {
                    total: extracted.total,
                    internal: vec![],
                    external: extracted_external,
                    known_routes,
                    element_ids: std::collections::HashMap::new(),
                };
                link_checker::check_external_links(&ctx.db, &ext, today, &external_options).await
            };

            link_result.external_checked = external_checked;
            link_result.broken_links.extend(external_broken);

            if let Some(ref p) = options.progress {
                p.update(|prog| prog.links.finish());
            }

            if !link_result.is_ok() {
                // Group broken links by URL to avoid repetition
                let mut by_url: std::collections::BTreeMap<&str, Vec<&str>> =
                    std::collections::BTreeMap::new();
                let mut reasons: std::collections::HashMap<&str, &str> =
                    std::collections::HashMap::new();
                let mut is_external: std::collections::HashMap<&str, bool> =
                    std::collections::HashMap::new();
                let mut diagnostics_by_url: std::collections::HashMap<
                    &str,
                    &dodeca::db::HttpErrorDiagnostics,
                > = std::collections::HashMap::new();

                for broken in &link_result.broken_links {
                    by_url
                        .entry(broken.href.as_str())
                        .or_default()
                        .push(broken.source_route.as_str());
                    reasons.insert(broken.href.as_str(), &broken.reason);
                    is_external.insert(broken.href.as_str(), broken.is_external);
                    if let Some(ref diag) = broken.diagnostics {
                        diagnostics_by_url.insert(broken.href.as_str(), diag);
                    }
                }

                for (href, sources) in &by_url {
                    let prefix = if *is_external.get(href).unwrap_or(&false) {
                        "[ext]"
                    } else {
                        "[int]"
                    };
                    let reason = reasons.get(href).unwrap_or(&"unknown");
                    if sources.len() == 1 {
                        eprintln!(
                            "{} {} -> {} (from {})",
                            prefix.dimmed(),
                            href.red(),
                            reason,
                            sources[0].yellow()
                        );
                    } else {
                        eprintln!(
                            "{} {} -> {} (from {} pages)",
                            prefix.dimmed(),
                            href.red(),
                            reason,
                            sources.len()
                        );
                    }

                    // Show diagnostics for HTTP errors
                    if let Some(diag) = diagnostics_by_url.get(href) {
                        eprintln!("      {}", "Request headers:".dimmed());
                        for (k, v) in &diag.request_headers {
                            eprintln!("        {}: {}", k.cyan(), v);
                        }
                        eprintln!("      {}", "Response headers:".dimmed());
                        for (k, v) in &diag.response_headers {
                            eprintln!("        {}: {}", k.cyan(), v);
                        }
                        if !diag.response_body.is_empty() {
                            eprintln!("      {}", "Response body:".dimmed());
                            // Truncate long bodies for display
                            let body = if diag.response_body.len() > 200 {
                                format!("{}...", &diag.response_body[..200])
                            } else {
                                diag.response_body.clone()
                            };
                            eprintln!("        {}", body.dimmed());
                        }
                    }
                }

                // Link checking is advisory - warn but don't fail the build
                eprintln!(
                    "{} {} broken link(s) ({} internal, {} external) across {} unique URLs",
                    "Warning:".yellow().bold(),
                    link_result.broken_links.len(),
                    link_result.internal_broken(),
                    link_result.external_broken(),
                    by_url.len()
                );
            }

            if verbose {
                println!(
                    "{} {} internal, {} external checked",
                    "Links".green().bold(),
                    link_result.internal_links,
                    link_result.external_checked
                );
            }

            if let Some(ref p) = options.progress {
                p.update(|prog| prog.search.finish());
            }
        }
    }

    if verbose {
        // Show query stats
        println!(
            "{} {} executed, {} reused",
            "Queries".cyan(),
            query_stats.executed(),
            query_stats.reused()
        );

        let elapsed = start.elapsed();
        println!(
            "\n{} in {:.2}s → {}",
            "Done".green().bold(),
            elapsed.as_secs_f64(),
            output_dir.cyan()
        );
    }

    // Save content store hashes
    store.save()?;

    // Save picante cache for next build
    save_picante_cache(&ctx.db, &picante_cache_path).await;

    Ok(ctx)
}

/// Handle a file change event by updating or adding it to picante
fn handle_file_changed(
    path: &Utf8PathBuf,
    config: &file_watcher::WatcherConfig,
    server: &serve::SiteServer,
) {
    use file_watcher::PathCategory;

    let category = config.categorize(path);
    let relative = match config.relative_path(path) {
        Some(r) => r,
        None => {
            tracing::debug!(path = %path, "handle_file_changed: path not in watched dirs, ignoring");
            return;
        }
    };

    tracing::debug!(
        path = %path,
        relative = %relative,
        category = ?category,
        "handle_file_changed: processing"
    );

    match category {
        PathCategory::Content => {
            if let Ok(content) = fs::read_to_string(path) {
                let last_modified = fs::metadata(path.as_std_path())
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let db = &*server.db;
                let mut sources = SourceRegistry::sources(db)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let relative_str = relative.to_string();
                let source_path = SourcePath::new(relative_str.clone());
                let source_content = SourceContent::new(content);

                // Find and replace existing, or add new
                if let Some(pos) = sources.iter().position(|s| {
                    s.path(db)
                        .ok()
                        .map(|p| p.as_str() == relative_str)
                        .unwrap_or(false)
                }) {
                    // Replace with new version (picante inputs are immutable after creation)
                    tracing::debug!(relative = %relative, "handle_file_changed: updating existing source file");
                    sources[pos] = SourceFile::new(db, source_path, source_content, last_modified)
                        .expect("failed to create source file");
                } else {
                    tracing::debug!(relative = %relative, "handle_file_changed: adding new source file");
                    let source = SourceFile::new(db, source_path, source_content, last_modified)
                        .expect("failed to create source file");
                    sources.push(source);
                    println!("  {} Added new source: {}", "+".green(), relative);
                }
                tracing::debug!(
                    count = sources.len(),
                    "handle_file_changed: setting SourceRegistry (triggers picante invalidation)"
                );
                SourceRegistry::set(db, sources).expect("failed to set sources");
            }
        }
        PathCategory::Template => {
            if let Ok(content) = fs::read_to_string(path) {
                let db = &*server.db;
                let mut templates = TemplateRegistry::templates(db)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let relative_str = match template_paths::logical_template_path(&relative) {
                    Some(path) => path,
                    None => return,
                };
                let template_path = TemplatePath::new(relative_str.clone());
                let template_content = TemplateContent::new(content);

                if let Some(pos) = templates.iter().position(|t| {
                    t.path(db)
                        .ok()
                        .map(|p| p.as_str() == relative_str)
                        .unwrap_or(false)
                }) {
                    templates[pos] = TemplateFile::new(db, template_path, template_content)
                        .expect("failed to create template file");
                } else {
                    let template = TemplateFile::new(db, template_path, template_content)
                        .expect("failed to create template file");
                    templates.push(template);
                    println!("  {} Added new template: {}", "+".green(), relative);
                }
                TemplateRegistry::set(db, templates).expect("failed to set templates");
            }
        }
        PathCategory::Sass => {
            if let Ok(content) = fs::read_to_string(path) {
                let db = &*server.db;
                let mut sass_files = SassRegistry::files(db).ok().flatten().unwrap_or_default();
                let relative_str = relative.to_string();
                let sass_path = SassPath::new(relative_str.clone());
                let sass_content = SassContent::new(content);

                if let Some(pos) = sass_files.iter().position(|s| {
                    s.path(db)
                        .ok()
                        .map(|p| p.as_str() == relative_str)
                        .unwrap_or(false)
                }) {
                    sass_files[pos] = SassFile::new(db, sass_path, sass_content)
                        .expect("failed to create sass file");
                } else {
                    let sass = SassFile::new(db, sass_path, sass_content)
                        .expect("failed to create sass file");
                    sass_files.push(sass);
                    println!("  {} Added new sass: {}", "+".green(), relative);
                }
                SassRegistry::set(db, sass_files).expect("failed to set sass files");
            }
        }
        PathCategory::Static | PathCategory::Dist => {
            if let Ok(content) = fs::read(path) {
                // Skip empty files (transient state during git operations)
                if content.is_empty() {
                    return;
                }
                let db = &*server.db;
                let mut static_files = StaticRegistry::files(db).ok().flatten().unwrap_or_default();
                let relative_str = relative.to_string();
                let static_path = StaticPath::new(relative_str.clone());

                if let Some(pos) = static_files.iter().position(|s| {
                    s.path(db)
                        .ok()
                        .map(|p| p.as_str() == relative_str)
                        .unwrap_or(false)
                }) {
                    tracing::debug!(path = %relative_str, size = content.len(), "Replacing static file");
                    static_files[pos] = StaticFile::new(db, static_path, content)
                        .expect("failed to create static file");
                } else {
                    let static_file = StaticFile::new(db, static_path, content)
                        .expect("failed to create static file");
                    static_files.push(static_file);
                    println!("  {} Added new static file: {}", "+".green(), relative);
                }
                StaticRegistry::set(db, static_files).expect("failed to set static files");
            }
        }
        PathCategory::Data => {
            if let Ok(content) = fs::read_to_string(path) {
                let db = &*server.db;
                let mut data_files = DataRegistry::files(db).ok().flatten().unwrap_or_default();
                let relative_str = relative.to_string();
                let data_path = DataPath::new(relative_str.clone());
                let data_content = DataContent::new(content);

                if let Some(pos) = data_files.iter().position(|d| {
                    d.path(db)
                        .ok()
                        .map(|p| p.as_str() == relative_str)
                        .unwrap_or(false)
                }) {
                    data_files[pos] = DataFile::new(db, data_path, data_content)
                        .expect("failed to create data file");
                } else {
                    let data_file = DataFile::new(db, data_path, data_content)
                        .expect("failed to create data file");
                    data_files.push(data_file);
                    println!("  {} Added new data file: {}", "+".green(), relative);
                }
                DataRegistry::set(db, data_files).expect("failed to set data files");
            }
        }
        PathCategory::Code => {
            if let Ok(content) = fs::read_to_string(path) {
                let last_modified = fs::metadata(path.as_std_path())
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let db = &*server.db;
                let mut files = CodeRegistry::files(db).ok().flatten().unwrap_or_default();
                let relative_str = relative.to_string();
                let code_path = dodeca::types::CodePath::new(relative_str.clone());
                let code_content = dodeca::types::CodeContent::new(content);
                if let Some(pos) = files.iter().position(|f| {
                    f.path(db)
                        .ok()
                        .map(|p| p.as_str() == relative_str)
                        .unwrap_or(false)
                }) {
                    files[pos] =
                        dodeca::db::CodeFile::new(db, code_path, code_content, last_modified)
                            .expect("failed to create code file");
                } else {
                    let f = dodeca::db::CodeFile::new(db, code_path, code_content, last_modified)
                        .expect("failed to create code file");
                    files.push(f);
                }
                CodeRegistry::set(db, files).expect("failed to set code files");
            }
        }
        // Config + include changes are handled at the batch level, not here.
        PathCategory::Config | PathCategory::Include => (),
        PathCategory::Unknown => (), // Unknown files don't need picante updates
    }
    // Note: For all file types, picante tracks changes when the registry is updated.
}

/// Handle a file removal event by removing it from picante
fn handle_file_removed(
    path: &Utf8PathBuf,
    config: &file_watcher::WatcherConfig,
    server: &serve::SiteServer,
) {
    use file_watcher::PathCategory;

    let category = config.categorize(path);
    let relative = match config.relative_path(path) {
        Some(r) => r,
        None => return,
    };
    let relative_str = relative.to_string();

    match category {
        PathCategory::Content => {
            let db = &*server.db;
            let mut sources = SourceRegistry::sources(db)
                .ok()
                .flatten()
                .unwrap_or_default();
            if let Some(pos) = sources.iter().position(|s| {
                s.path(db)
                    .ok()
                    .map(|p| p.as_str() == relative_str)
                    .unwrap_or(false)
            }) {
                sources.remove(pos);
                SourceRegistry::set(db, sources).expect("failed to set sources");
            }
        }
        PathCategory::Template => {
            let db = &*server.db;
            let mut templates = TemplateRegistry::templates(db)
                .ok()
                .flatten()
                .unwrap_or_default();
            let relative_str = match template_paths::logical_template_path(&relative) {
                Some(path) => path,
                None => return,
            };
            if let Some(pos) = templates.iter().position(|t| {
                t.path(db)
                    .ok()
                    .map(|p| p.as_str() == relative_str)
                    .unwrap_or(false)
            }) {
                templates.remove(pos);
                TemplateRegistry::set(db, templates).expect("failed to set templates");
            }
        }
        PathCategory::Sass => {
            let db = &*server.db;
            let mut sass_files = SassRegistry::files(db).ok().flatten().unwrap_or_default();
            if let Some(pos) = sass_files.iter().position(|s| {
                s.path(db)
                    .ok()
                    .map(|p| p.as_str() == relative_str)
                    .unwrap_or(false)
            }) {
                sass_files.remove(pos);
                SassRegistry::set(db, sass_files).expect("failed to set sass files");
            }
        }
        PathCategory::Static | PathCategory::Dist => {
            let db = &*server.db;
            let mut static_files = StaticRegistry::files(db).ok().flatten().unwrap_or_default();
            if let Some(pos) = static_files.iter().position(|s| {
                s.path(db)
                    .ok()
                    .map(|p| p.as_str() == relative_str)
                    .unwrap_or(false)
            }) {
                static_files.remove(pos);
                StaticRegistry::set(db, static_files).expect("failed to set static files");
            }
        }
        PathCategory::Data => {
            let db = &*server.db;
            let mut data_files = DataRegistry::files(db).ok().flatten().unwrap_or_default();
            if let Some(pos) = data_files.iter().position(|d| {
                d.path(db)
                    .ok()
                    .map(|p| p.as_str() == relative_str)
                    .unwrap_or(false)
            }) {
                data_files.remove(pos);
                DataRegistry::set(db, data_files).expect("failed to set data files");
            }
        }
        PathCategory::Code => {
            let db = &*server.db;
            let mut files = CodeRegistry::files(db).ok().flatten().unwrap_or_default();
            if let Some(pos) = files.iter().position(|f| {
                f.path(db)
                    .ok()
                    .map(|p| p.as_str() == relative_str)
                    .unwrap_or(false)
            }) {
                files.remove(pos);
                CodeRegistry::set(db, files).expect("failed to set code files");
            }
            let entries = CodeCoverageRegistry::entries(db)
                .ok()
                .flatten()
                .unwrap_or_default()
                .into_iter()
                .filter(|entry| entry.path.as_str() != relative_str)
                .collect();
            CodeCoverageRegistry::set(db, entries).expect("failed to set code coverage entries");
        }
        // Config + include changes are handled at the batch level, not here.
        PathCategory::Config | PathCategory::Include => {}
        PathCategory::Unknown => {}
    }
}

fn prune_missing_sources(config: &file_watcher::WatcherConfig, server: &serve::SiteServer) {
    let db = &*server.db;
    let sources = SourceRegistry::sources(db)
        .ok()
        .flatten()
        .unwrap_or_default();
    let before = sources.len();

    let retained: Vec<SourceFile> = sources
        .into_iter()
        .filter(|source| {
            let Ok(path) = source.path(db) else {
                return false;
            };
            // Reconstruct the on-disk path via the owning source's content dir,
            // reversing the mount prefix — a mounted key like `spec/build/x.md`
            // lives under that source's checkout, not the primary content dir.
            match dodeca::build_context::source_for_key(&config.sources, path.as_str()) {
                Some((src, rel)) => src.content_dir.join(rel).exists(),
                None => config.content_dir.join(path.as_str()).exists(),
            }
        })
        .collect();

    if retained.len() != before {
        let removed = before - retained.len();
        tracing::debug!(
            removed,
            "prune_missing_sources: removing source files missing from disk"
        );
        SourceRegistry::set(db, retained).expect("failed to set sources");
    }
}

/// Counts of files loaded into each registry, for caller-side logging.
struct RegistryCounts {
    sources: usize,
    templates: usize,
    static_files: usize,
    data: usize,
    sass: usize,
}

/// Load every picante input registry (sources, templates, static, data, sass)
/// from `sources` and the primary `parent_dir`. This is the single loader used
/// by both serve startup paths and by config hot-reload: setting each registry
/// is a picante input update, so downstream queries (renders, CSS, search)
/// invalidate and re-derive on demand. Replaces the whole set each call, so
/// removed sources drop out. Does no logging — callers report `RegistryCounts`.
fn load_all_registries(
    server: &serve::SiteServer,
    sources: &[dodeca::config::ResolvedSource],
    parent_dir: &Utf8Path,
) -> Result<RegistryCounts> {
    let static_dir = parent_dir.join("static");
    let dist_dir = parent_dir.join("dist");
    let data_dir = parent_dir.join("data");
    let sass_dir = parent_dir.join("sass");

    // Sources (mount-prefixed keys), via the same loader the build path uses.
    let source_files = dodeca::build_context::load_source_files(&server.db, sources)?;
    let sources_count = source_files.len();
    server.set_sources(source_files.into_iter().map(|(_, file)| file).collect());

    // Code files (from `impls` globs) for spec coverage. They live outside the
    // content tree (project-root-relative), so they load against the config root.
    if let Some(cfg) = dodeca::config::global_config() {
        let coverage_entries = dodeca::build_context::code_coverage_entries(sources, &cfg._root);
        let code = dodeca::build_context::load_code_files(&server.db, sources, &cfg._root)?;
        CodeRegistry::set(&*server.db, code.into_iter().map(|(_, f)| f).collect())
            .expect("failed to set code files");
        CodeCoverageRegistry::set(&*server.db, coverage_entries)
            .expect("failed to set code coverage entries");
    }

    // Templates (mount-prefixed keys) — each mounted source renders with its
    // own chrome, overlaid on the primary baseline.
    let templates: Vec<TemplateFile> =
        dodeca::build_context::load_template_files(&server.db, sources)?
            .into_iter()
            .map(|(_, file)| file)
            .collect();
    let templates_count = templates.len();
    server.set_templates(templates);

    // Static files: primary static/, then dist/ (overrides), then each source's
    // mount-prefixed static, plus the hidden vite manifest.
    let static_count = {
        let db = &*server.db;
        let mut static_files_map: std::collections::BTreeMap<String, StaticFile> =
            std::collections::BTreeMap::new();
        for dir in [&static_dir, &dist_dir] {
            if dir.exists() {
                for entry in WalkBuilder::new(dir).build() {
                    let entry = entry?;
                    let path = Utf8Path::from_path(entry.path())
                        .ok_or_else(|| eyre!("Non-UTF8 path in static/dist directory"))?;
                    if entry
                        .file_type()
                        .map(|ft| ft.is_file() || (ft.is_symlink() && path.is_file()))
                        .unwrap_or_else(|| path.is_file())
                    {
                        let relative = path.strip_prefix(dir)?;
                        let key = relative.to_string();
                        let static_file =
                            StaticFile::new(db, StaticPath::new(key.clone()), fs::read(path)?)?;
                        static_files_map.insert(key, static_file);
                    }
                }
            }
        }
        let manifest_path = dist_dir.join(".vite/manifest.json");
        if manifest_path.exists()
            && let Ok(content) = fs::read(&manifest_path)
        {
            let key = ".vite/manifest.json".to_string();
            let static_file = StaticFile::new(db, StaticPath::new(key.clone()), content)?;
            static_files_map.insert(key, static_file);
        }
        for (path, file) in dodeca::build_context::load_source_static_files(db, sources)? {
            static_files_map.insert(path.as_str().to_string(), file);
        }
        let count = static_files_map.len();
        server.set_static_files(static_files_map.into_values().collect());
        count
    };

    // Data files (primary only).
    let data_count = {
        let db = &*server.db;
        let mut data_files = Vec::new();
        if data_dir.exists() {
            for entry in WalkBuilder::new(&data_dir).build() {
                let entry = entry?;
                let path = Utf8Path::from_path(entry.path())
                    .ok_or_else(|| eyre!("Non-UTF8 path in data directory"))?;
                if path.is_file() && is_data_file_extension(path.extension().unwrap_or("")) {
                    let relative = path.strip_prefix(&data_dir)?;
                    let data_file = DataFile::new(
                        db,
                        DataPath::new(relative.to_string()),
                        DataContent::new(fs::read_to_string(path)?),
                    )?;
                    data_files.push(data_file);
                }
            }
        }
        let count = data_files.len();
        server.set_data_files(data_files);
        count
    };

    // SASS files: primary (bare keys) + each source's mount-prefixed sass
    // (per-source CSS bundles).
    let sass_count = {
        let db = &*server.db;
        let mut sass_files = Vec::new();
        if sass_dir.exists() {
            for entry in WalkBuilder::new(&sass_dir).build() {
                let entry = entry?;
                let path = match Utf8Path::from_path(entry.path()) {
                    Some(p) => p,
                    None => continue,
                };
                if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false)
                    && matches!(path.extension(), Some("scss") | Some("sass"))
                {
                    let relative = path
                        .strip_prefix(&sass_dir)
                        .map(|p| p.to_string())
                        .unwrap_or_else(|_| path.to_string());
                    let sass_file = SassFile::new(
                        db,
                        SassPath::new(relative),
                        SassContent::new(fs::read_to_string(path)?),
                    )?;
                    sass_files.push(sass_file);
                }
            }
        }
        for (_path, file) in dodeca::build_context::load_source_sass_files(db, sources)? {
            sass_files.push(file);
        }
        let count = sass_files.len();
        server.set_sass_files(sass_files);
        count
    };

    Ok(RegistryCounts {
        sources: sources_count,
        templates: templates_count,
        static_files: static_count,
        data: data_count,
        sass: sass_count,
    })
}

/// Build a `WatcherConfig` from a resolved config: primary dirs derived from
/// the primary content dir's parent, every mounted source, and the config file
/// to watch. Paths are canonicalized to match what `notify` reports.
fn build_watcher_config(
    resolved: &dodeca::config::ResolvedConfig,
    config_file: Option<Utf8PathBuf>,
) -> file_watcher::WatcherConfig {
    let content_dir = resolved.content_dir.clone();
    let parent = content_dir
        .parent()
        .map(|p| p.to_owned())
        .unwrap_or_else(|| content_dir.clone());
    let canon = |p: Utf8PathBuf| p.canonicalize_utf8().unwrap_or(p);
    file_watcher::WatcherConfig {
        content_dir: canon(content_dir),
        templates_dir: canon(parent.join("templates")),
        sass_dir: canon(parent.join("sass")),
        static_dir: canon(parent.join("static")),
        dist_dir: canon(parent.join("dist")),
        data_dir: canon(parent.join("data")),
        sources: canonicalize_sources(&resolved.sources),
        config_file: config_file.map(canon),
        // Preserve includes discovered so far so a config reload keeps watching them.
        included_files: dodeca::includes::known_abs(&resolved._root),
        code_files: dodeca::build_context::code_file_abs_paths(&resolved.sources, &resolved._root)
            .into_iter()
            .map(canon)
            .collect(),
        project_root: canon(resolved._root.clone()),
    }
}

/// Add include paths to the live watcher config (so `categorize` recognizes
/// them as [`file_watcher::PathCategory::Include`] and their edits fire events).
/// A no-op when they're all already present.
fn register_included_paths(
    config_swap: &arc_swap::ArcSwap<file_watcher::WatcherConfig>,
    paths: &[Utf8PathBuf],
) {
    let current = config_swap.load();
    if paths.iter().all(|p| current.included_files.contains(p)) {
        return;
    }
    let mut wc = (**current).clone();
    for p in paths {
        wc.included_files.insert(p.clone());
    }
    config_swap.store(std::sync::Arc::new(wc));
}

/// Re-resolve `.config/dodeca.styx` and reload everything in place.
///
/// Publishes the new config to both the ambient snapshot and the `ConfigRegistry`
/// picante input (invalidating every render that read it), reloads all file
/// registries from the new source set (picante re-derives the affected pages),
/// then refreshes the live `WatcherConfig` and starts watching any newly-added
/// source dirs. A parse error keeps the old config (the serve stays up).
fn reload_config(
    server: &serve::SiteServer,
    config_swap: &arc_swap::ArcSwap<file_watcher::WatcherConfig>,
    watcher: &file_watcher::WatcherHandle,
) -> Result<()> {
    let Some(config_file) = config_swap.load().config_file.clone() else {
        tracing::warn!("config changed but no config file is known; skipping reload");
        return Ok(());
    };
    // The project root is the parent of `.config/`.
    let Some(root) = config_file.parent().and_then(|p| p.parent()) else {
        return Ok(());
    };

    let resolved = match dodeca::config::ResolvedConfig::discover_from(root) {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::warn!(root = %root, "config reload: no config found; keeping old config");
            return Ok(());
        }
        Err(e) => {
            tracing::error!(error = %e, "config reload: parse failed; keeping old config");
            return Ok(());
        }
    };

    tracing::info!(
        sources = resolved.sources.len(),
        "config reload: re-resolved"
    );

    // Publish to the ambient snapshot and the picante input (same value).
    dodeca::config::set_global_config(resolved.clone())?;
    ConfigRegistry::set(&*server.db, std::sync::Arc::new(resolved.clone()))
        .expect("failed to set config input on reload");

    // Reload every file registry from the new source set — picante invalidates
    // and re-derives the affected pages, CSS bundles, and search index.
    let parent_dir = resolved
        .content_dir
        .parent()
        .map(|p| p.to_owned())
        .unwrap_or_else(|| resolved.content_dir.clone());
    let counts = load_all_registries(server, &resolved.sources, &parent_dir)?;
    tracing::info!(
        sources = counts.sources,
        templates = counts.templates,
        static_files = counts.static_files,
        data = counts.data,
        sass = counts.sass,
        "config reload: registries reloaded"
    );

    // Refresh the live watcher config (so `categorize` sees new sources) and
    // start watching any newly-added source dirs.
    let new_wc = build_watcher_config(&resolved, Some(config_file));
    file_watcher::watch_dirs(watcher, &new_wc.all_watch_dirs());
    config_swap.store(std::sync::Arc::new(new_wc));
    Ok(())
}

type FileEventHandler = Arc<dyn Fn(&file_watcher::FileEvent) + Send + Sync>;

fn expand_file_events(
    batch: Vec<file_watcher::FileEvent>,
    config: &file_watcher::WatcherConfig,
    on_event: Option<&FileEventHandler>,
) -> Vec<file_watcher::FileEvent> {
    let mut expanded = Vec::new();

    for file_event in batch {
        match file_event {
            file_watcher::FileEvent::DirectoryCreated(path) => {
                if let Some(cb) = on_event {
                    cb(&file_watcher::FileEvent::DirectoryCreated(path.clone()));
                }
                let mut scanned =
                    file_watcher::scan_directory_recursive(path.as_std_path(), config);
                for event in &scanned {
                    if let Some(cb) = on_event {
                        cb(event);
                    }
                }
                expanded.append(&mut scanned);
            }
            other => {
                if let Some(cb) = on_event {
                    cb(&other);
                }
                expanded.push(other);
            }
        }
    }

    expanded
}

fn apply_file_events_blocking(
    batch: Vec<file_watcher::FileEvent>,
    config: &file_watcher::WatcherConfig,
    server: &serve::SiteServer,
    on_event: Option<&FileEventHandler>,
) {
    let expanded = expand_file_events(batch, config, on_event);

    let should_prune_sources = expanded.iter().any(|event| match event {
        file_watcher::FileEvent::Changed(path) | file_watcher::FileEvent::Removed(path) => {
            config.categorize(path) == file_watcher::PathCategory::Content
        }
        file_watcher::FileEvent::DirectoryCreated(path) => {
            config.categorize(path) == file_watcher::PathCategory::Content
        }
    });

    for file_event in expanded {
        match file_event {
            file_watcher::FileEvent::Changed(path) => {
                handle_file_changed(&path, config, server);
            }
            file_watcher::FileEvent::Removed(path) => {
                handle_file_removed(&path, config, server);
            }
            file_watcher::FileEvent::DirectoryCreated(_) => {}
        }
    }

    if should_prune_sources {
        prune_missing_sources(config, server);
    }
}

fn drain_startup_file_events(
    watcher: &file_watcher::WatcherHandle,
    watcher_rx: &file_watcher::WatcherReceiver,
    config: &file_watcher::WatcherConfig,
    server: &serve::SiteServer,
    on_event: Option<&FileEventHandler>,
) {
    let mut file_events = Vec::new();

    while let Ok(event) = watcher_rx.try_recv() {
        let Ok(event) = event else { continue };
        file_events.extend(file_watcher::process_notify_event(event, config, watcher));
    }

    if file_events.is_empty() {
        return;
    }

    tracing::debug!(
        count = file_events.len(),
        "file watcher: applying startup events before readiness"
    );
    apply_file_events_blocking(file_events, config, server, on_event);
}

async fn start_file_watcher(
    server: Arc<serve::SiteServer>,
    watcher_config: file_watcher::WatcherConfig,
    on_event: Option<FileEventHandler>,
    after_apply: Option<Arc<dyn Fn() + Send + Sync>>,
) -> Result<()> {
    let (watcher, watcher_rx) = file_watcher::create_watcher(&watcher_config)?;
    start_file_watcher_from_receiver(
        server,
        watcher_config,
        on_event,
        after_apply,
        watcher,
        watcher_rx,
    )
    .await
}

async fn start_file_watcher_from_receiver(
    server: Arc<serve::SiteServer>,
    watcher_config: file_watcher::WatcherConfig,
    on_event: Option<FileEventHandler>,
    after_apply: Option<Arc<dyn Fn() + Send + Sync>>,
    watcher: file_watcher::WatcherHandle,
    watcher_rx: file_watcher::WatcherReceiver,
) -> Result<()> {
    let (event_tx, mut event_rx) =
        tokio::sync::mpsc::unbounded_channel::<file_watcher::FileEvent>();

    // The watcher config is swappable so a config hot-reload can update what the
    // notify thread and the apply loop see for `categorize` (e.g. a newly-added
    // source) without recreating the watcher.
    let config_swap = Arc::new(arc_swap::ArcSwap::from_pointee(watcher_config));

    // Keep a handle to the watcher for reload-time re-watching of new dirs (the
    // thread below takes ownership of its own clone to keep the watcher alive).
    let watcher_for_reload = watcher.clone();

    let config_thread = config_swap.clone();
    std::thread::spawn(move || {
        let watcher = watcher; // keep Arc alive
        while let Ok(event) = watcher_rx.recv() {
            let Ok(event) = event else { continue };
            let cfg = config_thread.load_full();
            let file_events = file_watcher::process_notify_event(event, &cfg, &watcher);
            for file_event in file_events {
                let _ = event_tx.send(file_event);
            }
        }
    });

    // Load + watch files referenced by `include` shortcodes as they're first
    // seen (the first render of an include happens on a request, not a
    // file-change batch, so we react to a signal instead).
    {
        let server = server.clone();
        let watcher = watcher_for_reload.clone();
        let swap = config_swap.clone();
        dodeca::spawn::spawn(async move {
            loop {
                dodeca::includes::wait_dirty().await;
                if let Some(cfg) = dodeca::config::global_config() {
                    let paths = dodeca::includes::refresh(&server.db, &cfg._root);
                    file_watcher::watch_include_files(&watcher, &paths);
                    register_included_paths(&swap, &paths);
                }
            }
        });
    }

    dodeca::spawn::spawn(async move {
        use tokio::time::{Duration, Instant};

        let debounce = Duration::from_millis(100);
        let max_debounce = Duration::from_millis(500);
        let mut pending: Vec<file_watcher::FileEvent> = Vec::new();
        let mut active_revision: Option<dodeca::revision::RevisionToken> = None;

        loop {
            let first = match event_rx.recv().await {
                Some(ev) => ev,
                None => break,
            };

            if active_revision.is_none() {
                active_revision = Some(server.begin_revision("file changes"));
            }

            pending.push(first);
            let batch_start = Instant::now();
            let mut deadline = batch_start + debounce;

            loop {
                let max_deadline = batch_start + max_debounce;
                let sleep_until = if deadline < max_deadline {
                    deadline
                } else {
                    max_deadline
                };
                tokio::select! {
                    Some(ev) = event_rx.recv() => {
                        pending.push(ev);
                        deadline = Instant::now() + debounce;
                    }
                    _ = tokio::time::sleep_until(sleep_until) => {
                        break;
                    }
                }
            }

            let batch = std::mem::take(&mut pending);
            let config_apply = config_swap.load_full();

            // Did this batch touch the config file? If so, run a full reload
            // after applying the (non-config) file events.
            let config_changed = batch.iter().any(|ev| {
                let path = match ev {
                    file_watcher::FileEvent::Changed(p)
                    | file_watcher::FileEvent::Removed(p)
                    | file_watcher::FileEvent::DirectoryCreated(p) => p,
                };
                config_apply.categorize(path) == file_watcher::PathCategory::Config
            });

            let server_apply = server.clone();
            let on_event = on_event.clone();
            let after_apply = after_apply.clone();
            let token = active_revision.take();

            let apply_result = tokio::task::spawn_blocking(move || {
                apply_file_events_blocking(
                    batch,
                    config_apply.as_ref(),
                    &server_apply,
                    on_event.as_ref(),
                );
                (server_apply, after_apply)
            })
            .await;

            // A config change re-resolves and reloads every registry in place.
            if config_changed {
                let server_reload = server.clone();
                let swap = config_swap.clone();
                let watcher = watcher_for_reload.clone();
                let reload = tokio::task::spawn_blocking(move || {
                    reload_config(&server_reload, &swap, &watcher)
                })
                .await;
                match reload {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => tracing::error!(error = %e, "config reload failed"),
                    Err(e) => tracing::error!(error = %e, "config reload task panicked"),
                }
            }

            // Pick up edits to files pulled in by `include` shortcodes — re-read
            // and republish the registry only if their contents changed (which
            // invalidates exactly the pages that embed them).
            if let Some(cfg) = dodeca::config::global_config() {
                let paths = dodeca::includes::refresh(&server.db, &cfg._root);
                file_watcher::watch_include_files(&watcher_for_reload, &paths);
                register_included_paths(&config_swap, &paths);
            }

            match apply_result {
                Ok((server_apply, after_apply)) => {
                    server_apply.trigger_reload().await;
                    if let Some(cb) = &after_apply {
                        cb();
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "file watcher apply task failed");
                }
            }

            if let Some(token) = token {
                server.end_revision(token);
            }
        }
    });

    Ok(())
}

/// Plain serve mode (no TUI) - serves directly from picante
#[allow(clippy::too_many_arguments)]
/// Canonicalize each source's content dir so the file watcher matches the
/// canonicalized paths `notify` reports (otherwise multi-source incremental
/// updates wouldn't strip-prefix correctly).
/// Spawn a background loop that `git pull`s every git-backed source's checkout
/// every `interval_secs`. Pulled changes land in the watched content dirs, so
/// Phase A's file watcher re-renders them — no bespoke rebuild. The fallback to
/// webhooks; a no-op if there are no git sources.
fn spawn_git_poll(sources: &[dodeca::config::ResolvedSource], interval_secs: u64) {
    let checkouts: Vec<Utf8PathBuf> = sources
        .iter()
        .filter(|s| s.git.is_some())
        .filter_map(|s| s.checkout_dir.clone())
        .collect();
    if checkouts.is_empty() || interval_secs == 0 {
        return;
    }
    dodeca::spawn::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        tick.tick().await; // the first tick fires immediately; skip it
        loop {
            tick.tick().await;
            for checkout in &checkouts {
                match tokio::process::Command::new("git")
                    .args(["-C", checkout.as_str(), "pull", "--ff-only"])
                    .status()
                    .await
                {
                    Ok(status) if !status.success() => {
                        tracing::warn!(checkout = %checkout, "git poll: `git pull` failed");
                    }
                    Err(e) => {
                        tracing::warn!(checkout = %checkout, error = %e, "git poll: spawn failed")
                    }
                    _ => {}
                }
            }
        }
    });
}

/// Clone any git-backed source whose checkout dir is absent on disk into that
/// stable location, so the loader/watcher can read `<checkout>/<content>`. A
/// webhook/poll later `git pull`s the same checkout; FS-notify re-renders.
fn ensure_git_sources(sources: &[dodeca::config::ResolvedSource]) -> Result<()> {
    for (idx, source) in sources.iter().enumerate() {
        let (Some(checkout), Some(git)) = (&source.checkout_dir, &source.git) else {
            continue;
        };
        if checkout.exists() {
            continue;
        }
        println!("  {} cloning {git} → {checkout}", "git".cyan());
        let outcome = std::process::Command::new("git")
            .args(["clone", git, checkout.as_str()])
            .status();
        let failure = match outcome {
            Ok(status) if status.success() => None,
            Ok(status) => Some(format!("git clone exited with {status}")),
            Err(e) => Some(format!("failed to run `git clone`: {e}")),
        };
        let Some(reason) = failure else {
            continue;
        };

        // The primary source (index 0) *is* the site — if it can't be cloned the
        // service has nothing to serve, so fail fast. An additional mounted
        // source that can't be cloned (e.g. the deploy bot doesn't have read on
        // its repo yet) must NOT take the whole site down: skip it, log loudly,
        // and let the rest serve. Its routes 404 until the access is fixed and a
        // `/_dodeca/pull/<name>` (or restart) fetches it.
        if idx == 0 {
            return Err(eyre!(
                "git clone failed for primary source `{}` ({git} → {checkout}): {reason}",
                source.name
            ));
        }
        tracing::error!(
            source = %source.name,
            mount = %source.mount,
            %git,
            checkout = %checkout,
            %reason,
            "git clone failed for mounted source — skipping it; the rest of the \
             site will serve. Fix repo access, then POST /_dodeca/pull/{} (or \
             restart) to fetch it.",
            source.name
        );
        eprintln!(
            "  {} clone failed for mounted source `{}` ({git}): {reason} — skipping; \
             its routes will 404 until fetched",
            "warn".yellow(),
            source.name
        );
    }
    Ok(())
}

/// Print a one-glance summary of the configured sources at serve startup, with
/// each source's mount, kind, and whether its checkout is present on disk.
/// Skipped for a plain single-source project (nothing to disambiguate).
fn print_source_banner(sources: &[dodeca::config::ResolvedSource]) {
    let interesting = sources.len() > 1 || sources.iter().any(|s| !s.name.is_empty());
    if !interesting {
        return;
    }
    println!("{}", "Sources:".bold());
    for s in sources {
        let (kind, loc, present) = match &s.checkout_dir {
            Some(checkout) => ("git", checkout.as_str(), checkout.exists()),
            None => ("local", s.content_dir.as_str(), s.content_dir.exists()),
        };
        let mark = if present {
            "✓".green().to_string()
        } else {
            "✗ absent".red().to_string()
        };
        let name = if s.name.is_empty() {
            "(root)"
        } else {
            s.name.as_str()
        };
        println!("  {mark}  {name}  {}  [{kind}] {loc}", s.mount.dimmed());
    }
}

/// `(name, checkout dir)` for each git-backed source — what the `/_dodeca/pull`
/// webhook and the poller operate on.
fn git_checkouts(sources: &[dodeca::config::ResolvedSource]) -> Vec<(String, Utf8PathBuf)> {
    sources
        .iter()
        .filter(|s| s.git.is_some())
        .filter_map(|s| Some((s.name.clone(), s.checkout_dir.clone()?)))
        .collect()
}

fn canonicalize_sources(
    sources: &[dodeca::config::ResolvedSource],
) -> Vec<dodeca::config::ResolvedSource> {
    sources
        .iter()
        .map(|s| dodeca::config::ResolvedSource {
            content_dir: s
                .content_dir
                .canonicalize_utf8()
                .unwrap_or_else(|_| s.content_dir.clone()),
            ..s.clone()
        })
        .collect()
}

/// True for loopback bind addresses (`127.0.0.1`, `::1`, `localhost`).
fn is_loopback_address(address: &str) -> bool {
    address == "localhost"
        || address
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// In-process authoring-LSP runner for the browser editor. Serves the same
/// `dodeca-authoring-lsp` Backend a desktop editor uses, but over an in-memory
/// duplex instead of a subprocess — no `ddc lsp` spawn, no stdio boundary. The
/// binary owns this because it depends on both `dodeca` and the LSP crate.
struct AuthoringLspRunner {
    content_dir: Utf8PathBuf,
    provider: std::sync::Arc<dyn dodeca::authoring_model::AuthoringProjectProvider>,
}

impl serve::LspRunner for AuthoringLspRunner {
    fn serve(
        &self,
        transport: tokio::io::DuplexStream,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let content = self.content_dir.to_string();
        let provider = self.provider.clone();
        Box::pin(async move {
            let (read, write) = tokio::io::split(transport);
            // The provider reuses the server's built db — no disk re-load.
            dodeca_authoring_lsp::serve_on(read, write, Some(content), None, Some(provider)).await;
        })
    }
}

/// Synthesize a local dev editor identity from a username (`--dev-editor`).
fn dev_editor_identity(user: &str) -> cell_http_proto::Identity {
    cell_http_proto::Identity {
        user: user.to_string(),
        email: format!("{user}@localhost"),
        name: user.to_string(),
        groups: Vec::new(),
    }
}

/// Where the serve commands bind: an address and an optional port.
struct Bind {
    address: String,
    port: Option<u16>,
}

async fn serve_plain(
    sources: &[dodeca::config::ResolvedSource],
    bind: Bind,
    open: bool,
    stable_assets: Vec<String>,
    fd_socket: Option<String>,
    public_access: bool,
    dev_editor: Option<String>,
) -> Result<()> {
    use std::sync::Arc;
    let (address, port) = (bind.address.as_str(), bind.port);

    // Clone any git-backed source that isn't checked out yet.
    ensure_git_sources(sources)?;
    print_source_banner(sources);
    // The primary source's content dir (mount `/`) anchors templates/sass/cache.
    let content_dir = &sources[0].content_dir;

    // IMPORTANT: Receive listening socket FIRST if --fd-socket was provided (for testing)
    // This must happen before any other initialization so the test harness isn't blocked.
    let pre_bound_listener: Option<std::net::TcpListener> =
        if let Some(ref channel_path) = fd_socket {
            #[cfg(unix)]
            {
                use std::os::unix::io::FromRawFd;
                use tokio::io::AsyncWriteExt;
                use tokio::net::UnixStream;

                tracing::info!("Connecting to Unix socket for FD passing: {}", channel_path);
                let mut unix_stream = UnixStream::connect(channel_path)
                    .await
                    .map_err(|e| eyre!("Failed to connect to fd-socket {}: {}", channel_path, e))?;

                tracing::info!("Receiving TCP listener FD from test harness");
                let fd = vox_fdpass::recv_fd(&unix_stream)
                    .await
                    .map_err(|e| eyre!("Failed to receive FD: {}", e))?;

                // SAFETY: We just received this FD from the test harness, which created a valid TcpListener
                // and sent us its file descriptor. We're the only owner now.
                let std_listener = unsafe { std::net::TcpListener::from_raw_fd(fd) };

                // IMPORTANT: tokio requires the listener to be in non-blocking mode
                std_listener
                    .set_nonblocking(true)
                    .map_err(|e| eyre!("Failed to set listener to non-blocking: {}", e))?;

                tracing::info!("Successfully received TCP listener FD");

                // Ack FD receipt to the harness. This avoids any OS-specific edge cases where the
                // harness closing its copy "immediately after send_fd returns" could still lead to
                // transient flakiness for the first connection (observed on macOS).
                unix_stream
                    .write_all(&[0xAC])
                    .await
                    .map_err(|e| eyre!("Failed to send FD ack: {}", e))?;
                Some(std_listener)
            }

            #[cfg(windows)]
            {
                use tokio::io::AsyncWriteExt;

                use std::io::Write;
                eprintln!("[ddc] Connecting to named pipe: {}", channel_path);
                std::io::stderr().flush().ok();
                tracing::info!(
                    "Connecting to named pipe for socket passing: {}",
                    channel_path
                );
                let mut pipe_stream = vox_stream::connect(channel_path)
                    .await
                    .map_err(|e| eyre!("Failed to connect to fd-socket {}: {}", channel_path, e))?;

                eprintln!("[ddc] Connected, receiving TCP listener...");
                std::io::stderr().flush().ok();
                tracing::info!("Receiving TCP listener from test harness");
                let std_listener = vox_fdpass::recv_tcp_listener(&mut pipe_stream)
                    .await
                    .map_err(|e| {
                        eprintln!("[ddc] Failed to receive TCP listener: {}", e);
                        std::io::stderr().flush().ok();
                        eyre!("Failed to receive TCP listener: {}", e)
                    })?;
                eprintln!("[ddc] Received TCP listener successfully");
                std::io::stderr().flush().ok();

                // IMPORTANT: tokio requires the listener to be in non-blocking mode
                std_listener
                    .set_nonblocking(true)
                    .map_err(|e| eyre!("Failed to set listener to non-blocking: {}", e))?;

                tracing::info!("Successfully received TCP listener");

                // Ack socket receipt to the harness
                pipe_stream
                    .write_all(&[0xAC])
                    .await
                    .map_err(|e| eyre!("Failed to send socket ack: {}", e))?;
                Some(std_listener)
            }
        } else {
            None
        };

    // Initialize asset cache (processed images, OG images, etc.)
    let parent_dir = content_dir.parent().unwrap_or(content_dir);
    let cache_dir = parent_dir.join(".cache");
    let templates_dir = parent_dir.join("templates");
    let sass_dir = parent_dir.join("sass");
    let static_dir = parent_dir.join("static");
    let dist_dir = parent_dir.join("dist");
    let data_dir = parent_dir.join("data");
    tracing::info!(
        content_dir = %content_dir,
        cache_dir = %cache_dir,
        "serve_plain: initializing"
    );
    cas::init_asset_cache(cache_dir.as_std_path())?;

    // Start Vite dev server if configured
    let _vite_server = vite::maybe_start_vite(parent_dir.as_std_path()).await;

    let render_options = render::RenderOptions {
        livereload: true, // Enable live reload in plain mode too
        dev_mode: true,
        source_maps: true,
        render_notes: true,
    };

    // Create the site server
    let server = Arc::new(serve::SiteServer::new(
        render_options,
        stable_assets,
        Some(content_dir.to_path_buf()),
    ));
    server.set_git_checkouts(git_checkouts(sources));
    server.set_lsp_runner(std::sync::Arc::new(AuthoringLspRunner {
        content_dir: content_dir.clone(),
        provider: serve::authoring_project_provider(server.clone()),
    }));
    if let Some(user) = dev_editor {
        tracing::warn!(user = %user, "DEV: acting as editor (local-only auth bypass)");
        server.set_dev_editor(Some(dev_editor_identity(&user)));
    }
    let startup_revision = server.begin_revision("startup");

    let watcher_config = file_watcher::WatcherConfig {
        content_dir: content_dir
            .canonicalize_utf8()
            .unwrap_or_else(|_| content_dir.clone()),
        templates_dir: templates_dir
            .canonicalize_utf8()
            .unwrap_or_else(|_| templates_dir.clone()),
        sass_dir: sass_dir
            .canonicalize_utf8()
            .unwrap_or_else(|_| sass_dir.clone()),
        static_dir: static_dir
            .canonicalize_utf8()
            .unwrap_or_else(|_| static_dir.clone()),
        dist_dir: dist_dir
            .canonicalize_utf8()
            .unwrap_or_else(|_| dist_dir.clone()),
        data_dir: data_dir
            .canonicalize_utf8()
            .unwrap_or_else(|_| data_dir.clone()),
        sources: canonicalize_sources(sources),
        config_file: dodeca::config::global_config().map(|c| {
            let p = dodeca::config::config_file_path(&c._root);
            p.canonicalize_utf8().unwrap_or(p)
        }),
        // Populated lazily as `include` shortcodes are first rendered.
        included_files: Default::default(),
        code_files: dodeca::config::global_config()
            .map(|c| {
                dodeca::build_context::code_file_abs_paths(sources, &c._root)
                    .into_iter()
                    .map(|p| p.canonicalize_utf8().unwrap_or(p))
                    .collect()
            })
            .unwrap_or_default(),
        project_root: dodeca::config::global_config()
            .map(|c| {
                c._root
                    .canonicalize_utf8()
                    .unwrap_or_else(|_| c._root.clone())
            })
            .unwrap_or_default(),
    };
    let (startup_watcher, startup_watcher_rx) = file_watcher::create_watcher(&watcher_config)?;

    // Use the requested port (or default to 4000)
    let requested_port: u16 = port.unwrap_or(4000);
    // Determine bind IPs based on --public flag or explicit 0.0.0.0 address
    let bind_ips: Vec<std::net::Ipv4Addr> = if public_access || address == "0.0.0.0" {
        // LAN mode: bind to localhost + all LAN interfaces
        let mut ips = vec![std::net::Ipv4Addr::LOCALHOST];
        ips.extend(tui::get_lan_ips());
        ips
    } else {
        // Local mode: bind to the specified address only
        let bind_ip = match address.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(ip)) => ip,
            Ok(std::net::IpAddr::V6(_)) => std::net::Ipv4Addr::LOCALHOST,
            Err(_) => std::net::Ipv4Addr::LOCALHOST,
        };
        vec![bind_ip]
    };

    // Create channel to receive actual bound port
    let (port_tx, port_rx) = tokio::sync::oneshot::channel();

    // Start the HTTP server in background ASAP so we can accept connections early.
    let server_clone = server.clone();
    dodeca::spawn::spawn(async move {
        if let Err(e) = cell_server::start_http_server_with_shutdown(
            server_clone,
            bind_ips,
            requested_port,
            None,
            Some(port_tx),
            pre_bound_listener,
        )
        .await
        {
            eprintln!("Server error: {e}");
        }
    });

    // Wait for the actual bound port
    let actual_port = port_rx
        .await
        .map_err(|_| eyre!("Failed to get bound port"))?;

    // Publish the resolved config into the picante input so tracked queries
    // (renders, per-source CSS, search) record a dependency on it — a config
    // reload then invalidates them automatically.
    if let Some(cfg) = dodeca::config::global_config() {
        ConfigRegistry::set(&*server.db, cfg).expect("failed to set config input");
    }

    // Load every picante input registry (sources, templates, static, data,
    // sass) — the single loader shared with config hot-reload.
    println!("{}", "Loading source files...".dimmed());
    let counts = load_all_registries(&server, sources, parent_dir)?;
    println!("  Loaded {} source files", counts.sources);
    // Status page — served by the http cell at /_dodeca/status on the content
    // port (HTTP stays in the cell). See note re: a separate localhost port.
    server.set_status_context(sources.to_vec(), actual_port);
    println!(
        "  {} http://127.0.0.1:{actual_port}/_dodeca/status",
        "Status".cyan()
    );
    println!("  Loaded {} templates", counts.templates);
    if counts.static_files > 0 {
        println!("  Loaded {} static files", counts.static_files);
    }
    println!("  Loaded {} data files", counts.data);
    println!("  Loaded {} SASS files", counts.sass);

    let on_event: FileEventHandler = Arc::new(|event: &file_watcher::FileEvent| match event {
        file_watcher::FileEvent::Changed(path) => {
            println!("  Changed: {}", path.file_name().unwrap_or("?"));
        }
        file_watcher::FileEvent::Removed(path) => {
            println!(
                "  {} Removed: {}",
                "-".red(),
                path.file_name().unwrap_or("?")
            );
        }
        file_watcher::FileEvent::DirectoryCreated(path) => {
            println!(
                "  {} New directory: {}",
                "+".green(),
                path.file_name().unwrap_or("?")
            );
        }
    });

    drain_startup_file_events(
        &startup_watcher,
        &startup_watcher_rx,
        &watcher_config,
        &server,
        Some(&on_event),
    );
    start_file_watcher_from_receiver(
        server.clone(),
        watcher_config.clone(),
        Some(on_event),
        None,
        startup_watcher,
        startup_watcher_rx,
    )
    .await?;

    // Mark the startup revision as ready after the watcher is installed and any events that
    // arrived during the initial disk load have been reconciled into the registries.
    server.end_revision(startup_revision);
    tracing::info!("startup revision ready (serve_plain)");

    // Print server URLs (LISTENING_PORT already printed by the HTTP server)
    // Use "0.0.0.0" to trigger multi-interface display when --public is used
    let display_address = if public_access { "0.0.0.0" } else { address };
    print_server_urls(display_address, actual_port);

    if open {
        let url = format!("http://127.0.0.1:{actual_port}");
        if let Err(e) = open::that(&url) {
            eprintln!("{} Failed to open browser: {}", "warning:".yellow(), e);
        }
    }

    // Block forever (server is running in background)
    std::future::pending::<()>().await;

    Ok(())
}

/// Serve with TUI progress display and file watching
///
/// This serves content directly from picante - no files written to disk.
/// HTTP requests query the picante database, which caches/memoizes results.
async fn serve_with_tui(
    _output_dir: &Utf8PathBuf,
    sources: &[dodeca::config::ResolvedSource],
    bind: Bind,
    open: bool,
    stable_assets: Vec<String>,
    start_public: bool,
    dev_editor: Option<String>,
) -> Result<()> {
    use std::sync::Arc;
    use tokio::sync::watch;
    let (address, port) = (bind.address.as_str(), bind.port);

    // Clone any git-backed source that isn't checked out yet.
    ensure_git_sources(sources)?;
    print_source_banner(sources);
    // The primary source's content dir (mount `/`) anchors templates/sass/cache.
    let content_dir = &sources[0].content_dir;

    // Enable TUI mode (must happen before cells init)
    host::Host::get().enable_tui_mode();

    // Take the command receiver for TUI → host command forwarding
    let proto_cmd_rx = host::Host::get()
        .take_command_rx()
        .await
        .expect("Command receiver should be available");

    // Spawn the local TUI display and push updates through direct channels.
    let tui_client = dodeca::tui_display::spawn_tui_display();

    // Initialize asset cache (processed images, OG images, etc.)
    let parent_dir = content_dir.parent().unwrap_or(content_dir);
    let cache_dir = parent_dir.join(".cache");
    cas::init_asset_cache(cache_dir.as_std_path())?;

    // Start Vite dev server if configured
    let _vite_server = vite::maybe_start_vite(parent_dir.as_std_path()).await;

    // Create channels
    let (progress_tx, mut progress_rx) = tui::progress_channel();
    let (server_tx, mut server_rx) = tui::server_status_channel();
    let (event_tx, event_rx) = tui::event_channel();

    // Initialize tracing with TUI layer - routes log events to Activity panel
    let filter_handle = logging::init_tui_tracing(event_tx.clone());

    // Render options with live reload enabled (development mode)
    let render_options = render::RenderOptions {
        livereload: true,
        dev_mode: true,
        source_maps: true,
        render_notes: true,
    };

    // Create the site server - serves directly from picante, no disk I/O
    let server = Arc::new(serve::SiteServer::new(
        render_options,
        stable_assets,
        Some(content_dir.to_path_buf()),
    ));
    server.set_git_checkouts(git_checkouts(sources));
    server.set_lsp_runner(std::sync::Arc::new(AuthoringLspRunner {
        content_dir: content_dir.clone(),
        provider: serve::authoring_project_provider(server.clone()),
    }));
    if let Some(user) = dev_editor {
        tracing::warn!(user = %user, "DEV: acting as editor (local-only auth bypass)");
        server.set_dev_editor(Some(dev_editor_identity(&user)));
    }
    let startup_revision = server.begin_revision("startup");

    // Load cached query results (e.g., processed images) from disk
    let cache_path = content_dir
        .parent()
        .unwrap_or(content_dir)
        .join(".cache/dodeca.bin");
    if let Err(e) = server.load_cache(cache_path.as_std_path()).await {
        let _ = event_tx.send(LogEvent::warn(format!("Failed to load cache: {e}")));
    }

    // Determine initial bind mode (--public flag or explicit 0.0.0.0 address)
    let initial_mode = if start_public || address == "0.0.0.0" {
        tui::BindMode::Lan
    } else {
        tui::BindMode::Local
    };

    // Get the IPs to bind to for a given mode
    fn get_bind_ips(mode: tui::BindMode) -> Vec<std::net::Ipv4Addr> {
        match mode {
            tui::BindMode::Local => vec![std::net::Ipv4Addr::LOCALHOST],
            tui::BindMode::Lan => {
                let mut ips = vec![std::net::Ipv4Addr::LOCALHOST];
                ips.extend(tui::get_lan_ips());
                ips
            }
        }
    }

    // Build URLs from IPs
    fn build_urls(ips: &[std::net::Ipv4Addr], port: u16) -> Vec<String> {
        ips.iter().map(|ip| format!("http://{ip}:{port}")).collect()
    }

    // Get cache sizes for status display
    let base_dir = content_dir.parent().unwrap_or(content_dir);
    let picante_cache_path = base_dir.join(".cache/dodeca.bin");
    let cas_cache_dir = base_dir.join(".cache");
    let code_exec_cache_dir = base_dir.join(".cache/code-execution");
    fn get_cache_sizes(
        picante_path: &Utf8Path,
        cas_dir: &Utf8Path,
        code_exec_dir: &Utf8Path,
    ) -> (usize, usize, usize) {
        let picante_size = picante_path
            .metadata()
            .map(|m| m.len() as usize)
            .unwrap_or(0);
        let code_exec_size = if code_exec_dir.exists() {
            dir_size(code_exec_dir)
        } else {
            0
        };
        let cas_size = if cas_dir.exists() {
            // Subtract picante file and code-execution dir since they're inside .cache
            dir_size(cas_dir)
                .saturating_sub(picante_size)
                .saturating_sub(code_exec_size)
        } else {
            0
        };
        (picante_size, cas_size, code_exec_size)
    }
    let (picante_size, cas_size, code_exec_size) =
        get_cache_sizes(&picante_cache_path, &cas_cache_dir, &code_exec_cache_dir);

    // Set initial server status (use preferred port or 4000 as placeholder until bound)
    let initial_ips = get_bind_ips(initial_mode);
    let display_port = port.unwrap_or(4000);
    let _ = server_tx.send(tui::ServerStatus {
        urls: build_urls(&initial_ips, display_port),
        is_running: false,
        bind_mode: initial_mode,
        picante_cache_size: picante_size as u64,
        cas_cache_size: cas_size as u64,
        code_exec_cache_size: code_exec_size as u64,
    });

    // Load source files into the server
    let _ = event_tx.send(LogEvent::build("Loading source files..."));
    progress_tx.send_modify(|prog| prog.parse.start(0));

    // Publish the resolved config into the picante input (see the serve_plain
    // path) so tracked queries record a dependency on it for reload invalidation.
    if let Some(cfg) = dodeca::config::global_config() {
        ConfigRegistry::set(&*server.db, cfg).expect("failed to set config input");
    }

    // Load every picante input registry (sources, templates, static, data,
    // sass) — the single loader shared with config hot-reload. `parent_dir` and
    // `templates_dir` are also used below by the file watcher, so they stay
    // bound here.
    let parent_dir = content_dir.parent().unwrap_or(content_dir);
    let templates_dir = parent_dir.join("templates");
    let counts = load_all_registries(&server, sources, parent_dir)?;
    progress_tx.send_modify(|prog| prog.parse.finish());
    let _ = event_tx.send(LogEvent::build(format!(
        "Loaded {} source files",
        counts.sources
    )));
    let _ = event_tx.send(LogEvent::build(format!(
        "Loaded {} templates",
        counts.templates
    )));
    if counts.static_files > 0 {
        let _ = event_tx.send(LogEvent::build(format!(
            "Loaded {} static files",
            counts.static_files
        )));
    }
    if counts.data > 0 {
        let _ = event_tx.send(LogEvent::build(format!(
            "Loaded {} data files",
            counts.data
        )));
    }
    let _ = event_tx.send(LogEvent::build(format!(
        "Loaded {} SASS files",
        counts.sass
    )));

    // Mark all tasks as ready - in serve mode, everything is computed on-demand via picante
    progress_tx.send_modify(|prog| {
        prog.render.finish();
        prog.sass.finish();
    });

    // Mark startup revision ready before accepting requests.
    server.end_revision(startup_revision);
    tracing::info!("startup revision ready (serve_with_tui)");

    let _ = event_tx.send(LogEvent::server(
        "Server ready - content served from memory",
    ));

    // Set up file watcher (shared pipeline with plain serve)
    let sass_dir = parent_dir.join("sass");
    let static_dir = parent_dir.join("static");
    let dist_dir = parent_dir.join("dist");
    let data_dir = parent_dir.join("data");

    let watcher_config = file_watcher::WatcherConfig {
        content_dir: content_dir
            .canonicalize_utf8()
            .unwrap_or_else(|_| content_dir.clone()),
        templates_dir: templates_dir
            .canonicalize_utf8()
            .unwrap_or_else(|_| templates_dir.clone()),
        sass_dir: sass_dir
            .canonicalize_utf8()
            .unwrap_or_else(|_| sass_dir.clone()),
        static_dir: static_dir
            .canonicalize_utf8()
            .unwrap_or_else(|_| static_dir.clone()),
        dist_dir: dist_dir
            .canonicalize_utf8()
            .unwrap_or_else(|_| dist_dir.clone()),
        data_dir: data_dir
            .canonicalize_utf8()
            .unwrap_or_else(|_| data_dir.clone()),
        sources: canonicalize_sources(sources),
        config_file: dodeca::config::global_config().map(|c| {
            let p = dodeca::config::config_file_path(&c._root);
            p.canonicalize_utf8().unwrap_or(p)
        }),
        // Populated lazily as `include` shortcodes are first rendered.
        included_files: Default::default(),
        code_files: dodeca::config::global_config()
            .map(|c| {
                dodeca::build_context::code_file_abs_paths(sources, &c._root)
                    .into_iter()
                    .map(|p| p.canonicalize_utf8().unwrap_or(p))
                    .collect()
            })
            .unwrap_or_default(),
        project_root: dodeca::config::global_config()
            .map(|c| {
                c._root
                    .canonicalize_utf8()
                    .unwrap_or_else(|_| c._root.clone())
            })
            .unwrap_or_default(),
    };

    let mut watched_dirs = vec![content_dir.to_string()];
    if templates_dir.exists() {
        watched_dirs.push("templates".to_string());
    }
    if sass_dir.exists() {
        watched_dirs.push("sass".to_string());
    }
    if static_dir.exists() {
        watched_dirs.push("static".to_string());
    }
    if dist_dir.exists() {
        watched_dirs.push("dist".to_string());
    }
    if data_dir.exists() {
        watched_dirs.push("data".to_string());
    }

    let _ = event_tx.send(LogEvent::file_change(format!(
        "Watching: {}",
        watched_dirs.join(", ")
    )));

    // Command channel for TUI -> server communication (async-compatible)
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<tui::ServerCommand>();

    // Shutdown signal for the server
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Start server in background
    let server_for_http = server.clone();
    let server_tx_clone = server_tx.clone();
    let event_tx_clone = event_tx.clone();
    let picante_cache_path_clone = picante_cache_path.clone();
    let cas_cache_dir_clone = cas_cache_dir.clone();

    let start_server = |server: Arc<serve::SiteServer>,
                        mode: tui::BindMode,
                        preferred_port: Option<u16>,
                        shutdown_rx: watch::Receiver<bool>,
                        server_tx: tui::ServerStatusTx,
                        event_tx: tui::EventTx,
                        picante_path: Utf8PathBuf,
                        cas_dir: Utf8PathBuf,
                        code_exec_dir: Utf8PathBuf| {
        dodeca::spawn::spawn(async move {
            let ips = get_bind_ips(mode);
            let requested_port = preferred_port.unwrap_or(4000);

            // Create channel to receive actual bound port
            let (port_tx, port_rx) = tokio::sync::oneshot::channel();

            // Start the server (this spawns the accept loop and sends back the port)
            let server_clone = server.clone();
            let ips_clone = ips.clone();
            let shutdown_rx_clone = shutdown_rx.clone();
            let event_tx_clone = event_tx.clone();

            let server_task = dodeca::spawn::spawn(async move {
                if let Err(e) = cell_server::start_http_server_with_shutdown(
                    server_clone,
                    ips_clone,
                    requested_port,
                    Some(shutdown_rx_clone),
                    Some(port_tx),
                    None, // No pre-bound listener for TUI mode
                )
                .await
                {
                    let _ = event_tx_clone.send(LogEvent::error(format!("Server error: {e}")));
                }
            });

            // Wait for actual bound port
            let actual_port = match port_rx.await {
                Ok(port) => port,
                Err(_) => {
                    let _ = event_tx.send(LogEvent::error("Failed to get bound port".to_string()));
                    return;
                }
            };

            // Get current cache sizes
            let picante_size = picante_path
                .metadata()
                .map(|m| m.len() as usize)
                .unwrap_or(0);
            let code_exec_size = if code_exec_dir.exists() {
                dir_size(&code_exec_dir)
            } else {
                0
            };
            let cas_size = WalkBuilder::new(&cas_dir)
                .build()
                .filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len() as usize)
                .sum::<usize>()
                .saturating_sub(picante_size)
                .saturating_sub(code_exec_size);

            // Update server status
            let _ = server_tx.send(tui::ServerStatus {
                urls: build_urls(&ips, actual_port),
                is_running: true,
                bind_mode: mode,
                picante_cache_size: picante_size as u64,
                cas_cache_size: cas_size as u64,
                code_exec_cache_size: code_exec_size as u64,
            });

            // Log the binding
            let mode_str = match mode {
                tui::BindMode::Local => "localhost only",
                tui::BindMode::Lan => "LAN",
            };
            let _ = event_tx.send(LogEvent::server(format!(
                "Binding to {} IPs on port {} ({})",
                ips.len(),
                actual_port,
                mode_str
            )));
            for ip in &ips {
                let _ = event_tx.send(LogEvent::server(format!("  → {ip}:{actual_port}")));
            }

            // Wait for server to complete (shutdown signal received)
            // This ensures the outer task doesn't return until the server is actually stopped
            let _ = server_task.await;
        })
    };

    let code_exec_cache_dir_clone = code_exec_cache_dir.clone();
    let mut server_handle = start_server(
        server_for_http.clone(),
        initial_mode,
        port,
        shutdown_rx.clone(),
        server_tx_clone.clone(),
        event_tx_clone.clone(),
        picante_cache_path_clone.clone(),
        cas_cache_dir_clone.clone(),
        code_exec_cache_dir_clone.clone(),
    );

    // Open browser if requested (use preferred port or default 4000)
    if open {
        let browser_port = port.unwrap_or(4000);
        let url = format!("http://127.0.0.1:{browser_port}");
        // Give server a moment to bind before opening browser
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Err(e) = open::that(&url) {
            let _ = event_tx.send(LogEvent::warn(format!("Failed to open browser: {e}")));
        }
    }

    let event_tx_for_watcher = event_tx.clone();
    let on_event = Arc::new(
        move |path_event: &file_watcher::FileEvent| match path_event {
            file_watcher::FileEvent::Changed(path) => {
                let ext = path.extension();
                let filename = path.file_name().unwrap_or("?");
                let file_type = match ext {
                    Some("md") => "content",
                    Some("html") => "template",
                    Some("scss") | Some("sass") => "style",
                    Some("css") => "css",
                    Some("js") | Some("ts") => "script",
                    Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("svg")
                    | Some("webp") | Some("avif") => "image",
                    Some("woff") | Some("woff2") | Some("ttf") | Some("otf") => "font",
                    _ => "file",
                };
                let _ = event_tx_for_watcher.send(LogEvent::file_change(format!(
                    "{} changed: {}",
                    file_type, filename
                )));
            }
            file_watcher::FileEvent::Removed(path) => {
                let filename = path.file_name().unwrap_or("?");
                let _ = event_tx_for_watcher
                    .send(LogEvent::file_change(format!("removed: {}", filename)));
            }
            file_watcher::FileEvent::DirectoryCreated(path) => {
                let filename = path.file_name().unwrap_or("?");
                let _ = event_tx_for_watcher.send(LogEvent::file_change(format!(
                    "new directory: {}",
                    filename
                )));
            }
        },
    );

    start_file_watcher(server.clone(), watcher_config, Some(on_event), None).await?;

    // Spawn command handler for rebinding
    let server_for_cmd = server.clone();
    let server_tx_for_cmd = server_tx.clone();
    let event_tx_for_cmd = event_tx.clone();
    let picante_path_for_cmd = picante_cache_path_clone.clone();
    let cas_dir_for_cmd = cas_cache_dir_clone.clone();
    let code_exec_dir_for_cmd = code_exec_cache_dir_clone.clone();
    // Use Arc<Mutex> for the shutdown sender so we can update it for each rebind
    let current_shutdown = Arc::new(std::sync::Mutex::new(shutdown_tx.clone()));
    let current_shutdown_for_handler = current_shutdown.clone();

    dodeca::spawn::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            let new_mode = match cmd {
                tui::ServerCommand::GoPublic => tui::BindMode::Lan,
                tui::ServerCommand::GoLocal => tui::BindMode::Local,
                // Other commands are handled by the TUI host, not here
                _ => continue,
            };

            // Signal current server to shutdown
            {
                let shutdown = current_shutdown_for_handler.lock().unwrap();
                let _ = shutdown.send(true);
            }

            // Wait for server to stop
            let _ = server_handle.await;

            // Create new shutdown channel
            let (new_shutdown_tx, new_shutdown_rx) = watch::channel(false);

            // Update the current shutdown sender for next time
            {
                let mut shutdown = current_shutdown_for_handler.lock().unwrap();
                *shutdown = new_shutdown_tx;
            }

            let _ = event_tx_for_cmd.send(LogEvent::info("Restarting server..."));

            // Start new server
            server_handle = start_server(
                server_for_cmd.clone(),
                new_mode,
                port,
                new_shutdown_rx,
                server_tx_for_cmd.clone(),
                event_tx_for_cmd.clone(),
                picante_path_for_cmd.clone(),
                cas_dir_for_cmd.clone(),
                code_exec_dir_for_cmd.clone(),
            );
        }
    });

    // Bridge commands from the local TUI display to server actions.
    let mut proto_cmd_rx = proto_cmd_rx; // Move the receiver from earlier
    let filter_handle_for_bridge = filter_handle.clone();
    let event_tx_for_bridge = event_tx.clone();
    dodeca::spawn::spawn(async move {
        while let Some(proto_cmd) = proto_cmd_rx.recv().await {
            match proto_cmd {
                cell_tui_proto::ServerCommand::CycleLogLevel => {
                    // Handle directly - cycle the log level
                    let new_level = filter_handle_for_bridge.cycle_log_level();
                    let _ = event_tx_for_bridge.send(tui::LogEvent::info(format!(
                        "Log level: {}",
                        new_level.as_str()
                    )));
                }
                cell_tui_proto::ServerCommand::TogglePicanteDebug => {
                    // Legacy command - toggle picante debug (now mostly obsolete with picante)
                    let enabled = filter_handle_for_bridge.toggle_picante_debug();
                    let _ = event_tx_for_bridge.send(tui::LogEvent::info(format!(
                        "Picante debug: {}",
                        if enabled { "ON" } else { "OFF" }
                    )));
                }
                cell_tui_proto::ServerCommand::SetLogFilter { filter } => {
                    // Set custom log filter expression
                    match filter_handle_for_bridge.set_filter(&filter) {
                        Some(expr) if expr.is_empty() => {
                            let _ = event_tx_for_bridge
                                .send(tui::LogEvent::info("Log filter cleared".to_string()));
                        }
                        Some(expr) => {
                            let _ = event_tx_for_bridge
                                .send(tui::LogEvent::info(format!("Log filter: {}", expr)));
                        }
                        None => {
                            let _ = event_tx_for_bridge.send(tui::LogEvent::warn(format!(
                                "Invalid log filter: {}",
                                filter
                            )));
                        }
                    }
                }
                other => {
                    // Route to old command system for bind mode changes
                    let old_cmd = tui_host::convert_server_command(other);
                    let _ = cmd_tx.send(old_cmd);
                }
            }
        }
    });

    // Seed the TUI with the latest snapshots.
    {
        let progress = progress_rx.borrow().clone();
        let _ = tui_client
            .update_progress(tui_host::convert_build_progress(&progress))
            .await;
    }
    {
        let status = server_rx.borrow().clone();
        let _ = tui_client
            .update_status(tui_host::convert_server_status(&status))
            .await;
    }

    // Spawn forwarders to push updates to the local TUI display.
    // Forward progress updates
    let tui_client_progress = tui_client.clone();
    dodeca::spawn::spawn(async move {
        while progress_rx.changed().await.is_ok() {
            let progress = progress_rx.borrow().clone();
            let _ = tui_client_progress
                .update_progress(tui_host::convert_build_progress(&progress))
                .await;
        }
    });

    // Forward server status updates
    let tui_client_status = tui_client.clone();
    dodeca::spawn::spawn(async move {
        while server_rx.changed().await.is_ok() {
            let status = server_rx.borrow().clone();
            let _ = tui_client_status
                .update_status(tui_host::convert_server_status(&status))
                .await;
        }
    });

    // Forward log events (event_rx is std::sync::mpsc, so use spawn_blocking)
    let tui_client_events = tui_client.clone();
    let rt_handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        while let Ok(event) = event_rx.recv() {
            let client = tui_client_events.clone();
            let proto_event = tui_host::convert_log_event(&event);
            rt_handle.block_on(async move { client.push_event(proto_event).await });
        }
    });

    // Wait for exit command from TUI
    host::Host::get().wait_for_exit().await;

    // Signal server to shutdown (use current_shutdown in case it was swapped)
    {
        let shutdown = current_shutdown.lock().unwrap();
        let _ = shutdown.send(true);
    }

    // Save cache before exit
    if let Err(e) = std::fs::create_dir_all(cache_path.parent().unwrap()) {
        eprintln!("Failed to create cache dir: {e}");
    }
    let cache_start = std::time::Instant::now();
    match server.save_cache(cache_path.as_std_path()).await {
        Ok(()) => eprintln!("Cache saved in {:.2?}", cache_start.elapsed()),
        Err(e) => eprintln!("Failed to save cache: {e}"),
    }

    // Force exit - the forwarder tasks (especially spawn_blocking) won't be
    // cancelled automatically when the async runtime shuts down
    std::process::exit(0)
}

/// Simple static file server - serves files from a directory without any build step
async fn serve_static(
    dir: &Utf8PathBuf,
    address: &str,
    port: Option<u16>,
    open: bool,
    public_access: bool,
) -> Result<()> {
    use cell_http_proto::{ContentService, ServeContent};
    use std::sync::Arc;

    // Canonicalize the directory path for display
    let dir = dir.canonicalize_utf8().unwrap_or_else(|_| dir.to_owned());

    println!(
        "\n{} {}",
        "Serving static files from".green().bold(),
        dir.cyan()
    );

    // Create a simple content service that reads files from disk
    #[derive(Clone)]
    struct StaticContentService {
        root: Utf8PathBuf,
    }

    impl ContentService for StaticContentService {
        async fn find_content(
            &self,
            path: String,
            _identity: Option<cell_http_proto::Identity>,
        ) -> ServeContent {
            // Static serve has no auth gating. Normalize path - remove leading slash
            let path = path.trim_start_matches('/');

            // Try the exact path first, then index.html for directories
            let file_path = self.root.join(path);
            let try_paths = if path.is_empty() || path.ends_with('/') {
                vec![file_path.join("index.html")]
            } else {
                vec![file_path.clone(), file_path.join("index.html")]
            };

            for try_path in try_paths {
                if try_path.is_file() {
                    match std::fs::read(&try_path) {
                        Ok(content) => {
                            let mime = guess_static_mime(try_path.as_str());
                            // For HTML, return as Html variant so livereload could be injected
                            if mime == "text/html" {
                                match String::from_utf8(content) {
                                    Ok(html) => {
                                        return ServeContent::Html {
                                            content: html,
                                            route: format!("/{path}"),
                                            generation: 0,
                                        };
                                    }
                                    Err(e) => {
                                        // Not valid UTF-8, serve as binary
                                        return ServeContent::Static {
                                            content: e.into_bytes(),
                                            mime: mime.to_string(),
                                            generation: 0,
                                        };
                                    }
                                }
                            }
                            return ServeContent::Static {
                                content,
                                mime: mime.to_string(),
                                generation: 0,
                            };
                        }
                        Err(_) => continue,
                    }
                }
            }

            // Not found
            ServeContent::NotFound {
                html: format!(
                    r#"<!DOCTYPE html>
<html><head><title>404 Not Found</title></head>
<body><h1>404 Not Found</h1><p>The requested path <code>/{path}</code> was not found.</p></body>
</html>"#
                ),
                generation: 0,
            }
        }

        async fn get_scope(
            &self,
            _route: String,
            _path: Vec<String>,
        ) -> Vec<cell_http_proto::ScopeEntry> {
            vec![] // No devtools for static mode
        }

        async fn eval_expression(
            &self,
            _route: String,
            _expression: String,
        ) -> cell_http_proto::EvalResult {
            cell_http_proto::EvalResult::Err("Not supported in static mode".to_string())
        }
    }

    let content_service = Arc::new(StaticContentService { root: dir.clone() });

    // Determine bind IPs
    let bind_ips: Vec<std::net::Ipv4Addr> = if public_access || address == "0.0.0.0" {
        let mut ips = vec![std::net::Ipv4Addr::LOCALHOST];
        ips.extend(tui::get_lan_ips());
        ips
    } else {
        let bind_ip = match address.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(ip)) => ip,
            Ok(std::net::IpAddr::V6(_)) => std::net::Ipv4Addr::LOCALHOST,
            Err(_) => std::net::Ipv4Addr::LOCALHOST,
        };
        vec![bind_ip]
    };

    let requested_port = port.unwrap_or(8080);

    // Create channel to receive actual bound port
    let (port_tx, port_rx) = tokio::sync::oneshot::channel();

    // Start the HTTP server with our static content service
    let content_service_clone = content_service.clone();
    dodeca::spawn::spawn(async move {
        if let Err(e) = cell_server::start_static_http_server(
            content_service_clone,
            bind_ips,
            requested_port,
            Some(port_tx),
        )
        .await
        {
            eprintln!("Server error: {e}");
        }
    });

    // Wait for the actual bound port
    let actual_port = port_rx
        .await
        .map_err(|_| eyre!("Failed to get bound port"))?;

    let bind_addr = if public_access { "0.0.0.0" } else { address };
    print_server_urls(bind_addr, actual_port);

    // Open browser if requested
    if open {
        let url = format!("http://127.0.0.1:{actual_port}");
        if let Err(e) = open::that(&url) {
            eprintln!("Failed to open browser: {e}");
        }
    }

    println!("{}", "Press Ctrl+C to stop".dimmed());

    // Wait forever (until Ctrl+C)
    std::future::pending::<()>().await;

    Ok(())
}

/// Guess MIME type for static files
fn guess_static_mime(path: &str) -> &'static str {
    if path.ends_with(".html") || path.ends_with(".htm") {
        "text/html"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".js") {
        "application/javascript"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".gif") {
        "image/gif"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".webp") {
        "image/webp"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else if path.ends_with(".woff") {
        "font/woff"
    } else if path.ends_with(".ttf") {
        "font/ttf"
    } else if path.ends_with(".xml") {
        "application/xml"
    } else if path.ends_with(".txt") {
        "text/plain"
    } else if path.ends_with(".wasm") {
        "application/wasm"
    } else {
        "application/octet-stream"
    }
}
