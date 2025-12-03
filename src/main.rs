#![allow(clippy::collapsible_if)]

mod cache_bust;
mod cas;
mod config;
mod data;
mod db;
mod error_pages;
mod file_watcher;
mod html_diff;
mod image;
mod link_checker;
mod logging;
mod plugins;
mod queries;
mod render;
mod search;
mod serve;
mod svg;
mod template;
mod theme;
mod tui;
mod types;
mod url_rewrite;

use crate::config::ResolvedConfig;
use crate::db::{
    DataFile, DataRegistry, Database, OutputFile, QueryStats, SassFile, SassRegistry, SourceFile,
    SourceRegistry, StaticFile, StaticRegistry, TemplateFile, TemplateRegistry,
};
use crate::queries::build_site;
use crate::tui::LogEvent;
use crate::types::{
    DataContent, DataPath, Route, SassContent, SassPath, SassPathRef, SourceContent, SourcePath,
    SourcePathRef, StaticPath, TemplateContent, TemplatePath, TemplatePathRef,
};
use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{Result, eyre::eyre};
use facet::Facet;
use facet_args as args;
use ignore::WalkBuilder;
use owo_colors::OwoColorize;
use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

/// Build command arguments
#[derive(Facet, Debug)]
struct BuildArgs {
    /// Project directory (looks for .config/dodeca.kdl here)
    #[facet(args::positional, default)]
    path: Option<String>,

    /// Content directory (uses .config/dodeca.kdl if not specified)
    #[facet(args::named, args::short = 'c', default)]
    content: Option<String>,

    /// Output directory (uses .config/dodeca.kdl if not specified)
    #[facet(args::named, args::short = 'o', default)]
    output: Option<String>,

    /// Show TUI progress display
    #[facet(args::named)]
    tui: bool,
}

/// Serve command arguments
#[derive(Facet, Debug)]
struct ServeArgs {
    /// Project directory (looks for .config/dodeca.kdl here)
    #[facet(args::positional, default)]
    path: Option<String>,

    /// Content directory (uses .config/dodeca.kdl if not specified)
    #[facet(args::named, args::short = 'c', default)]
    content: Option<String>,

    /// Output directory (uses .config/dodeca.kdl if not specified)
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
}

/// Clean command arguments
#[derive(Facet, Debug)]
struct CleanArgs {
    /// Project directory (looks for .config/dodeca.kdl here)
    #[facet(args::positional, default)]
    path: Option<String>,
}

/// Parsed command from CLI
enum Command {
    Build(BuildArgs),
    Serve(ServeArgs),
    Clean(CleanArgs),
}

fn print_usage() {
    eprintln!(
        "{} - Static site generator\n",
        "ddc".cyan().bold()
    );
    eprintln!("{}", "USAGE:".yellow());
    eprintln!("    ddc <COMMAND> [OPTIONS]\n");
    eprintln!("{}", "COMMANDS:".yellow());
    eprintln!("    {}      Build the site", "build".green());
    eprintln!("    {}      Build and serve with live reload", "serve".green());
    eprintln!("    {}      Clear all caches", "clean".green());
    eprintln!("\n{}", "BUILD OPTIONS:".yellow());
    eprintln!("    [path]           Project directory");
    eprintln!("    -c, --content    Content directory");
    eprintln!("    -o, --output     Output directory");
    eprintln!("    --tui            Show TUI progress display");
    eprintln!("\n{}", "SERVE OPTIONS:".yellow());
    eprintln!("    [path]           Project directory");
    eprintln!("    -c, --content    Content directory");
    eprintln!("    -o, --output     Output directory");
    eprintln!("    -a, --address    Address to bind on (default: 127.0.0.1)");
    eprintln!("    -p, --port       Port to serve on (default: 4000)");
    eprintln!("    --open           Open browser after starting server");
    eprintln!("    --no-tui         Disable TUI (show plain output)");
    eprintln!("    -P, --public     Listen on all interfaces (LAN access)");
    eprintln!("\n{}", "CLEAN OPTIONS:".yellow());
    eprintln!("    [path]           Project directory");
}

fn parse_args() -> Result<Command> {
    // Set up miette graphical reporter for nice error output
    miette::set_hook(Box::new(|_| {
        Box::new(
            miette::MietteHandlerOpts::new()
                .terminal_links(true)
                .unicode(true)
                .context_lines(3)
                .build(),
        )
    }))
    .ok(); // Ignore error if already set

    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        return Err(eyre!("No command specified"));
    }

    let cmd = &args[1];
    let rest: Vec<&str> = args[2..].iter().map(|s| s.as_str()).collect();

    match cmd.as_str() {
        "build" => {
            let build_args: BuildArgs = facet_args::from_slice(&rest).map_err(|e| {
                // Print miette error directly for nice formatting
                eprintln!("{:?}", miette::Report::new(e));
                eyre!("Failed to parse build arguments")
            })?;
            Ok(Command::Build(build_args))
        }
        "serve" => {
            let serve_args: ServeArgs = facet_args::from_slice(&rest).map_err(|e| {
                eprintln!("{:?}", miette::Report::new(e));
                eyre!("Failed to parse serve arguments")
            })?;
            Ok(Command::Serve(serve_args))
        }
        "clean" => {
            let clean_args: CleanArgs = facet_args::from_slice(&rest).map_err(|e| {
                eprintln!("{:?}", miette::Report::new(e));
                eyre!("Failed to parse clean arguments")
            })?;
            Ok(Command::Clean(clean_args))
        }
        "--help" | "-h" | "help" => {
            print_usage();
            std::process::exit(0);
        }
        _ => {
            print_usage();
            Err(eyre!("Unknown command: {}", cmd))
        }
    }
}

/// Resolved configuration for a build
struct ResolvedBuildConfig {
    content_dir: Utf8PathBuf,
    output_dir: Utf8PathBuf,
    skip_domains: Vec<String>,
    rate_limit_ms: Option<u64>,
    stable_assets: Vec<String>,
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
            skip_domains: vec![],
            rate_limit_ms: None,
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
            let content_dir = content.unwrap_or(cfg.content_dir);
            let output_dir = output.unwrap_or(cfg.output_dir);
            Ok(ResolvedBuildConfig {
                content_dir,
                output_dir,
                skip_domains: cfg.skip_domains,
                rate_limit_ms: cfg.rate_limit_ms,
                stable_assets: cfg.stable_assets,
            })
        }
        None => {
            let config_path = path
                .as_ref()
                .map(|p| format!("{}/.config/dodeca.kdl", p))
                .unwrap_or_else(|| ".config/dodeca.kdl".to_string());
            Err(eyre!(
                "{}\n\n\
                     Create a config file at {} with:\n\n\
                     \x20   {}\n\
                     \x20   {}\n\n\
                     Or specify both {} and {} on the command line.",
                "No configuration found.".red().bold(),
                config_path.cyan(),
                "content \"path/to/content\"".green(),
                "output \"path/to/output\"".green(),
                "--content".yellow(),
                "--output".yellow()
            ))
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let command = parse_args()?;

    match command {
        Command::Build(args) => {
            let cfg = resolve_dirs(args.path, args.content, args.output)?;

            // Always use the mini build TUI for now
            build_with_mini_tui(&cfg.content_dir, &cfg.output_dir, &cfg.skip_domains, cfg.rate_limit_ms)?;
        }
        Command::Serve(args) => {
            let cfg = resolve_dirs(args.path, args.content, args.output)?;

            // Check if we should use TUI
            use std::io::IsTerminal;
            let use_tui = args.force_tui || (!args.no_tui && std::io::stdout().is_terminal());

            if use_tui {
                serve_with_tui(&cfg.content_dir, &cfg.output_dir, &args.address, args.port, args.open, cfg.stable_assets, args.public_access).await?;
            } else {
                // Plain mode - no TUI, serve from Salsa
                logging::init_standard_tracing();
                serve_plain(&cfg.content_dir, &args.address, args.port, args.open, cfg.stable_assets).await?;
            }
        }
        Command::Clean(args) => {
            // Find the project directory
            let base_dir = if let Some(p) = args.path {
                Utf8PathBuf::from(p)
            } else if let Some(cfg) = ResolvedConfig::discover()? {
                cfg.content_dir.parent().map(|p| p.to_owned()).unwrap_or(cfg.content_dir)
            } else {
                Utf8PathBuf::from(".")
            };

            let cache_dir = base_dir.join(".cache");
            let cas_db = base_dir.join(".dodeca.db");

            let mut cleared = Vec::new();

            // Remove .cache directory (Salsa DB + image cache)
            if cache_dir.exists() {
                let size = dir_size(&cache_dir);
                fs::remove_dir_all(&cache_dir)?;
                cleared.push(format!(".cache/ ({})", format_bytes(size)));
            }

            // Remove CAS database (canopydb is a directory)
            if cas_db.exists() {
                let size = dir_size(&cas_db);
                fs::remove_dir_all(&cas_db)?;
                cleared.push(format!(".dodeca.db/ ({})", format_bytes(size)));
            }

            if cleared.is_empty() {
                println!("{}", "No caches to clear.".dimmed());
            } else {
                println!("{} {}", "Cleared:".green(), cleared.join(", "));
            }
        }
    }

    Ok(())
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BuildMode {
    /// Full build - block on link checking and search index
    Full,
    /// Quick build - just HTML, async link checking
    Quick,
}

/// The build context with Salsa database
pub struct BuildContext {
    pub db: Database,
    pub content_dir: Utf8PathBuf,
    pub output_dir: Utf8PathBuf,
    /// Source files keyed by source path
    pub sources: BTreeMap<SourcePath, SourceFile>,
    /// Template files keyed by template path
    pub templates: BTreeMap<TemplatePath, TemplateFile>,
    /// Sass/SCSS files keyed by sass path
    pub sass_files: BTreeMap<SassPath, SassFile>,
    /// Static files keyed by static path
    pub static_files: BTreeMap<StaticPath, StaticFile>,
    /// Data files keyed by data path
    pub data_files: BTreeMap<DataPath, DataFile>,
    /// Query statistics (if tracking enabled)
    pub stats: Option<Arc<QueryStats>>,
}

impl BuildContext {
    pub fn new(content_dir: &Utf8Path, output_dir: &Utf8Path) -> Self {
        Self::with_stats(content_dir, output_dir, None)
    }

    pub fn with_stats(
        content_dir: &Utf8Path,
        output_dir: &Utf8Path,
        stats: Option<Arc<QueryStats>>,
    ) -> Self {
        let db = match &stats {
            Some(s) => Database::new_with_stats(Arc::clone(s)),
            None => Database::new(),
        };
        Self {
            db,
            content_dir: content_dir.to_owned(),
            output_dir: output_dir.to_owned(),
            sources: BTreeMap::new(),
            templates: BTreeMap::new(),
            sass_files: BTreeMap::new(),
            static_files: BTreeMap::new(),
            data_files: BTreeMap::new(),
            stats,
        }
    }

    /// Get the templates directory (sibling to content dir)
    pub fn templates_dir(&self) -> Utf8PathBuf {
        self.content_dir
            .parent()
            .unwrap_or(&self.content_dir)
            .join("templates")
    }

    /// Get the sass directory (sibling to content dir)
    pub fn sass_dir(&self) -> Utf8PathBuf {
        self.content_dir
            .parent()
            .unwrap_or(&self.content_dir)
            .join("sass")
    }

    /// Get the static directory (sibling to content dir)
    pub fn static_dir(&self) -> Utf8PathBuf {
        self.content_dir
            .parent()
            .unwrap_or(&self.content_dir)
            .join("static")
    }

    /// Get the data directory (sibling to content dir)
    pub fn data_dir(&self) -> Utf8PathBuf {
        self.content_dir
            .parent()
            .unwrap_or(&self.content_dir)
            .join("data")
    }

    /// Load all source files into the database
    pub fn load_sources(&mut self) -> Result<()> {
        let md_files: Vec<Utf8PathBuf> = WalkBuilder::new(&self.content_dir)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
            .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
            .collect();

        for path in md_files {
            let content = fs::read_to_string(&path)?;
            let last_modified = fs::metadata(&path)?
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let relative = path
                .strip_prefix(&self.content_dir)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());

            let source_path = SourcePath::new(relative);
            let source_content = SourceContent::new(content);
            let source = SourceFile::new(&self.db, source_path.clone(), source_content, last_modified);
            self.sources.insert(source_path, source);
        }

        Ok(())
    }

    /// Load all template files into the database
    pub fn load_templates(&mut self) -> Result<()> {
        let templates_dir = self.templates_dir();
        if !templates_dir.exists() {
            return Ok(()); // No templates directory is fine
        }

        let template_files: Vec<Utf8PathBuf> = WalkBuilder::new(&templates_dir)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "html")
                    .unwrap_or(false)
            })
            .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
            .collect();

        for path in template_files {
            let content = fs::read_to_string(&path)?;
            let relative = path
                .strip_prefix(&templates_dir)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());

            let template_path = TemplatePath::new(relative);
            let template_content = TemplateContent::new(content);
            let template = TemplateFile::new(&self.db, template_path.clone(), template_content);
            self.templates.insert(template_path, template);
        }

        Ok(())
    }

    /// Load all Sass/SCSS files into the database
    pub fn load_sass(&mut self) -> Result<()> {
        let sass_dir = self.sass_dir();
        if !sass_dir.exists() {
            return Ok(()); // No sass directory is fine
        }

        let sass_files: Vec<Utf8PathBuf> = WalkBuilder::new(&sass_dir)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "scss" || ext == "sass")
                    .unwrap_or(false)
            })
            .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
            .collect();

        for path in sass_files {
            let content = fs::read_to_string(&path)?;
            let relative = path
                .strip_prefix(&sass_dir)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());

            let sass_path = SassPath::new(relative);
            let sass_content = SassContent::new(content);
            let sass_file = SassFile::new(&self.db, sass_path.clone(), sass_content);
            self.sass_files.insert(sass_path, sass_file);
        }

        Ok(())
    }

    /// Load all static files into the database
    pub fn load_static(&mut self) -> Result<()> {
        let static_dir = self.static_dir();
        if !static_dir.exists() {
            return Ok(()); // No static directory is fine
        }

        let static_files: Vec<Utf8PathBuf> = WalkBuilder::new(&static_dir)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
            .collect();

        for path in static_files {
            let content = fs::read(&path)?;
            let relative = path
                .strip_prefix(&static_dir)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());

            let static_path = StaticPath::new(relative);
            let static_file = StaticFile::new(&self.db, static_path.clone(), content);
            self.static_files.insert(static_path, static_file);
        }

        Ok(())
    }

    /// Load all data files into the database
    pub fn load_data(&mut self) -> Result<()> {
        let data_dir = self.data_dir();
        if !data_dir.exists() {
            return Ok(()); // No data directory is fine
        }

        let data_files: Vec<Utf8PathBuf> = WalkBuilder::new(&data_dir)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| {
                // Only load supported data file formats
                e.path()
                    .extension()
                    .map(|ext| {
                        let ext = ext.to_string_lossy().to_lowercase();
                        matches!(ext.as_str(), "kdl" | "json" | "toml" | "yaml" | "yml")
                    })
                    .unwrap_or(false)
            })
            .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
            .collect();

        for path in data_files {
            let content = fs::read_to_string(&path)?;
            let relative = path
                .strip_prefix(&data_dir)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());

            let data_path = DataPath::new(relative);
            let data_content = DataContent::new(content);
            let data_file = DataFile::new(&self.db, data_path.clone(), data_content);
            self.data_files.insert(data_path, data_file);
        }

        Ok(())
    }

    /// Update a single source file (for incremental rebuilds)
    pub fn update_source(&mut self, relative_path: &SourcePathRef) -> Result<bool> {
        let full_path = self.content_dir.join(relative_path.as_str());
        if !full_path.exists() {
            // File was deleted
            self.sources.remove(relative_path);
            return Ok(true);
        }

        let content = fs::read_to_string(&full_path)?;
        let source_content = SourceContent::new(content);
        let last_modified = fs::metadata(&full_path)?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // Check if we already have this source
        if let Some(existing) = self.sources.get(relative_path) {
            // Update the content and mtime - Salsa will detect if they changed
            use salsa::Setter;
            existing.set_content(&mut self.db).to(source_content);
            existing.set_last_modified(&mut self.db).to(last_modified);
        } else {
            // New file
            let source_path = SourcePath::new(relative_path.to_string());
            let source = SourceFile::new(&self.db, source_path.clone(), source_content, last_modified);
            self.sources.insert(source_path, source);
        }

        Ok(true)
    }

    /// Update a single template file (for incremental rebuilds)
    pub fn update_template(&mut self, relative_path: &TemplatePathRef) -> Result<bool> {
        let templates_dir = self.templates_dir();
        let full_path = templates_dir.join(relative_path.as_str());
        if !full_path.exists() {
            // File was deleted
            self.templates.remove(relative_path);
            return Ok(true);
        }

        let content = fs::read_to_string(&full_path)?;
        let template_content = TemplateContent::new(content);

        // Check if we already have this template
        if let Some(existing) = self.templates.get(relative_path) {
            // Update the content - Salsa will detect if it changed
            use salsa::Setter;
            existing.set_content(&mut self.db).to(template_content);
        } else {
            // New file
            let template_path = TemplatePath::new(relative_path.to_string());
            let template = TemplateFile::new(&self.db, template_path.clone(), template_content);
            self.templates.insert(template_path, template);
        }

        Ok(true)
    }

    /// Update a single Sass file (for incremental rebuilds)
    pub fn update_sass(&mut self, relative_path: &SassPathRef) -> Result<bool> {
        let sass_dir = self.sass_dir();
        let full_path = sass_dir.join(relative_path.as_str());
        if !full_path.exists() {
            // File was deleted
            self.sass_files.remove(relative_path);
            return Ok(true);
        }

        let content = fs::read_to_string(&full_path)?;
        let sass_content = SassContent::new(content);

        // Check if we already have this sass file
        if let Some(existing) = self.sass_files.get(relative_path) {
            // Update the content - Salsa will detect if it changed
            use salsa::Setter;
            existing.set_content(&mut self.db).to(sass_content);
        } else {
            // New file
            let sass_path = SassPath::new(relative_path.to_string());
            let sass_file = SassFile::new(&self.db, sass_path.clone(), sass_content);
            self.sass_files.insert(sass_path, sass_file);
        }

        Ok(true)
    }
}

// inject_livereload is now in render.rs
use render::inject_livereload;

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

pub fn build(
    content_dir: &Utf8PathBuf,
    output_dir: &Utf8PathBuf,
    mode: BuildMode,
    progress: Option<tui::ProgressReporter>,
    render_options: render::RenderOptions,
) -> Result<BuildContext> {
    use std::time::Instant;

    let start = Instant::now();
    let verbose = progress.is_none(); // Print to stdout when no TUI progress

    // Open content-addressed storage at base dir
    let base_dir = content_dir.parent().unwrap_or(content_dir);
    let cas_path = base_dir.join(".dodeca.db");
    let store = cas::ContentStore::open(&cas_path)?;

    // Initialize asset cache (processed images, OG images, etc.)
    let cache_dir = base_dir.join(".cache");
    cas::init_asset_cache(cache_dir.as_std_path())?;

    // Create query stats for tracking
    let query_stats = QueryStats::new();
    let mut ctx = BuildContext::with_stats(content_dir, output_dir, Some(Arc::clone(&query_stats)));

    // Phase 1: Load everything into Salsa
    ctx.load_sources()?;
    ctx.load_templates()?;
    ctx.load_sass()?;
    ctx.load_static()?;
    ctx.load_data()?;

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

    // Create registries for the query
    let source_vec: Vec<_> = ctx.sources.values().copied().collect();
    let template_vec: Vec<_> = ctx.templates.values().copied().collect();
    let sass_vec: Vec<_> = ctx.sass_files.values().copied().collect();
    let static_vec: Vec<_> = ctx.static_files.values().copied().collect();
    let data_vec: Vec<_> = ctx.data_files.values().copied().collect();

    let source_registry = SourceRegistry::new(&ctx.db, source_vec);
    let template_registry = TemplateRegistry::new(&ctx.db, template_vec);
    let sass_registry = SassRegistry::new(&ctx.db, sass_vec);
    let static_registry = StaticRegistry::new(&ctx.db, static_vec);
    let data_registry = DataRegistry::new(&ctx.db, data_vec);

    // Update progress: parsing phase
    if let Some(ref p) = progress {
        p.update(|prog| prog.parse.start(ctx.sources.len()));
    }

    // THE query - produces all outputs (fonts are automatically subsetted)
    let site_output = build_site(
        &ctx.db,
        source_registry,
        template_registry,
        sass_registry,
        static_registry,
        data_registry,
    );

    if let Some(ref p) = progress {
        p.update(|prog| {
            prog.parse.finish();
            prog.render.start(site_output.files.len());
        });
    }

    // Write outputs to disk, only if changed
    let mut stats = BuildStats::default();

    for output in &site_output.files {
        match output {
            OutputFile::Html { route, content } => {
                // Check for render errors in production mode
                if !render_options.dev_mode && content.contains(render::RENDER_ERROR_MARKER) {
                    let error_start = content.find("<pre>").map(|i| i + 5).unwrap_or(0);
                    let error_end = content.find("</pre>").unwrap_or(content.len());
                    let error_msg = &content[error_start..error_end];
                    return Err(eyre!(
                        "Template error rendering {}: {}",
                        route.as_str(),
                        error_msg
                    ));
                }

                // Apply livereload injection if needed (no dead link checking in build mode)
                let final_html = inject_livereload(content, render_options, None);
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

    if let Some(ref p) = progress {
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

    if mode == BuildMode::Full {
        // Check internal links
        let pages = site_output.files.iter().filter_map(|f| match f {
            OutputFile::Html { route, content } => Some(link_checker::Page {
                route,
                html: content,
            }),
            _ => None,
        });
        let link_result = link_checker::check_links(pages);

        if let Some(ref p) = progress {
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

        // Build search index in a separate thread (pagefind is async)
        let output_for_search = site_output.clone();
        let search_files = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(search::build_search_index(&output_for_search))
        })
        .join()
        .map_err(|_| eyre!("search thread panicked"))??;

        // Write search index files
        for (path, content) in &search_files {
            let dest = output_dir.join(path.trim_start_matches('/'));
            store.write_if_changed(&dest, content)?;
        }

        if verbose {
            println!("{} {} search files", "Indexed".cyan(), search_files.len());
        }

        if let Some(ref p) = progress {
            p.update(|prog| prog.search.finish());
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

    Ok(ctx)
}

/// Build with mini inline TUI - shows progress without taking over screen
fn build_with_mini_tui(
    content_dir: &Utf8PathBuf,
    output_dir: &Utf8PathBuf,
    skip_domains: &[String],
    rate_limit_ms: Option<u64>,
) -> Result<()> {
    use crossterm::cursor;
    use ratatui::prelude::*;
    use ratatui::widgets::Paragraph;
    use std::io::{self, IsTerminal};
    use std::time::Instant;

    // Initialize tracing (respects RUST_LOG)
    logging::init_standard_tracing();

    let start = Instant::now();
    let is_terminal = io::stdout().is_terminal();

    // Open CAS
    let base_dir = content_dir.parent().unwrap_or(content_dir);
    let cas_path = base_dir.join(".dodeca.db");
    let store = cas::ContentStore::open(&cas_path)?;

    // Initialize asset cache (processed images, OG images, etc.)
    let cache_dir = base_dir.join(".cache");
    cas::init_asset_cache(cache_dir.as_std_path())?;

    // Create query stats
    let query_stats = QueryStats::new();
    let stats_for_display = Arc::clone(&query_stats);

    // Create build context with stats
    let mut ctx = BuildContext::with_stats(content_dir, output_dir, Some(query_stats));

    // Load files
    ctx.load_sources()?;
    ctx.load_templates()?;
    ctx.load_sass()?;
    ctx.load_static()?;
    ctx.load_data()?;

    let input_count = ctx.sources.len()
        + ctx.templates.len()
        + ctx.sass_files.len()
        + ctx.static_files.len()
        + ctx.data_files.len();

    // Set up inline terminal (3 lines) - only if we have a TTY
    let mut terminal = if is_terminal {
        let mut stdout = io::stdout();
        crossterm::execute!(stdout, cursor::Hide)?;

        let backend = ratatui::backend::CrosstermBackend::new(stdout);
        let options = ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Inline(3),
        };
        Some(ratatui::Terminal::with_options(backend, options)?)
    } else {
        None
    };

    // Draw progress (only when we have a terminal)
    let draw_progress = |terminal: &mut Option<ratatui::Terminal<_>>,
                         phase: &str,
                         stats: &QueryStats,
                         written: usize,
                         skipped: usize| {
        if let Some(term) = terminal {
            term.draw(|frame| {
                let area = frame.area();

                let text = vec![
                    Line::from(vec![
                        Span::styled("Phase: ", Style::default().fg(Color::Cyan)),
                        Span::raw(phase),
                    ]),
                    Line::from(vec![
                        Span::styled("Queries: ", Style::default().fg(Color::Cyan)),
                        Span::raw(format!(
                            "{} executed, {} reused",
                            stats.executed(),
                            stats.reused()
                        )),
                    ]),
                    Line::from(vec![
                        Span::styled("Output: ", Style::default().fg(Color::Cyan)),
                        Span::raw(format!("{written} written, {skipped} up-to-date")),
                    ]),
                ];

                let paragraph = Paragraph::new(text);
                frame.render_widget(paragraph, area);
            })
            .ok();
        }
    };

    // Initial draw
    draw_progress(&mut terminal, "Loading...", &stats_for_display, 0, 0);

    // Create registries
    let source_vec: Vec<_> = ctx.sources.values().copied().collect();
    let template_vec: Vec<_> = ctx.templates.values().copied().collect();
    let sass_vec: Vec<_> = ctx.sass_files.values().copied().collect();
    let static_vec: Vec<_> = ctx.static_files.values().copied().collect();
    let data_vec: Vec<_> = ctx.data_files.values().copied().collect();

    let source_registry = SourceRegistry::new(&ctx.db, source_vec);
    let template_registry = TemplateRegistry::new(&ctx.db, template_vec);
    let sass_registry = SassRegistry::new(&ctx.db, sass_vec);
    let static_registry = StaticRegistry::new(&ctx.db, static_vec);
    let data_registry = DataRegistry::new(&ctx.db, data_vec);

    draw_progress(&mut terminal, "Building...", &stats_for_display, 0, 0);

    // Run the build query
    let site_output = build_site(
        &ctx.db,
        source_registry,
        template_registry,
        sass_registry,
        static_registry,
        data_registry,
    );

    draw_progress(&mut terminal, "Writing...", &stats_for_display, 0, 0);

    // Write outputs
    let mut written = 0usize;
    let mut skipped = 0usize;

    let render_options = render::RenderOptions {
        livereload: false,
        dev_mode: false,
    };

    for output in &site_output.files {
        match output {
            OutputFile::Html { route, content } => {
                if !render_options.dev_mode && content.contains(render::RENDER_ERROR_MARKER) {
                    // Clean up terminal before error
                    if terminal.is_some() {
                        drop(terminal);
                        crossterm::execute!(io::stdout(), cursor::Show)?;
                    }

                    let error_start = content.find("<pre>").map(|i| i + 5).unwrap_or(0);
                    let error_end = content.find("</pre>").unwrap_or(content.len());
                    let error_msg = &content[error_start..error_end];
                    return Err(eyre!(
                        "Template error rendering {}: {}",
                        route.as_str(),
                        error_msg
                    ));
                }

                let final_html = inject_livereload(content, render_options, None);
                let path = route_to_path(output_dir, route);

                if store.write_if_changed(&path, final_html.as_bytes())? {
                    written += 1;
                } else {
                    skipped += 1;
                }
            }
            OutputFile::Css { path, content } => {
                let dest = output_dir.join(path.as_str());
                if store.write_if_changed(&dest, content.as_bytes())? {
                    written += 1;
                } else {
                    skipped += 1;
                }
            }
            OutputFile::Static { path, content } => {
                let dest = output_dir.join(path.as_str());
                if store.write_if_changed(&dest, content)? {
                    written += 1;
                } else {
                    skipped += 1;
                }
            }
        }

        // Update display periodically
        draw_progress(
            &mut terminal,
            "Writing...",
            &stats_for_display,
            written,
            skipped,
        );
    }

    // Check links
    draw_progress(
        &mut terminal,
        "Checking links...",
        &stats_for_display,
        written,
        skipped,
    );

    let pages = site_output.files.iter().filter_map(|f| match f {
        OutputFile::Html { route, content } => Some(link_checker::Page {
            route,
            html: content,
        }),
        _ => None,
    });
    let extracted = link_checker::extract_links(pages);
    let mut link_result = link_checker::check_internal_links(&extracted);

    // Check external links with date-based caching
    let today = chrono::Local::now().date_naive();
    let mut external_options =
        link_checker::ExternalLinkOptions::new().skip_domains(skip_domains.iter().cloned());
    if let Some(ms) = rate_limit_ms {
        external_options = external_options.rate_limit_ms(ms);
    }

    // Run external link checking in a separate thread with its own runtime
    let extracted_external = extracted.external.clone();
    let known_routes = extracted.known_routes.clone();
    let (external_broken, external_checked) = {
        let ext = link_checker::ExtractedLinks {
            total: extracted.total,
            internal: vec![],
            external: extracted_external,
            known_routes,
        };
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let mut cache = std::collections::HashMap::new();
                link_checker::check_external_links(&ext, &mut cache, today, &external_options).await
            })
        })
        .join()
        .map_err(|_| eyre!("external link check thread panicked"))?
    };

    link_result.external_checked = external_checked;
    link_result.broken_links.extend(external_broken);

    if !link_result.is_ok() {
        // Clean up terminal before error
        if terminal.is_some() {
            drop(terminal);
            crossterm::execute!(io::stdout(), cursor::Show)?;
        }

        for broken in &link_result.broken_links {
            let prefix = if broken.is_external { "[ext]" } else { "[int]" };
            eprintln!(
                "{} {}: {} -> {}",
                prefix.dimmed(),
                broken.source_route.as_str().yellow(),
                broken.href.as_str().red(),
                broken.reason
            );
        }
        return Err(eyre!(
            "Found {} broken link(s) ({} internal, {} external)",
            link_result.broken_links.len(),
            link_result.internal_broken(),
            link_result.external_broken()
        ));
    }

    // Build search index
    draw_progress(
        &mut terminal,
        "Indexing...",
        &stats_for_display,
        written,
        skipped,
    );

    // Use a new runtime for pagefind since we're called from sync context within tokio
    let search_files = {
        let output = site_output.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(search::build_search_index(&output))
        })
        .join()
        .map_err(|_| eyre!("search thread panicked"))??
    };

    // Write search index files
    for (path, content) in &search_files {
        let dest = output_dir.join(path.trim_start_matches('/'));
        store.write_if_changed(&dest, content)?;
    }

    // Final state
    draw_progress(&mut terminal, "Done!", &stats_for_display, written, skipped);

    // Clean up terminal
    if terminal.is_some() {
        drop(terminal);
        crossterm::execute!(io::stdout(), cursor::Show)?;
    }

    // Calculate output directory size
    let output_size = dir_size(output_dir);

    // Print final summary
    let elapsed = start.elapsed();
    println!(
        "{} {} inputs → {} queries ({} executed) → {} written, {} up-to-date + {} search",
        "Build".green().bold(),
        input_count,
        stats_for_display.total(),
        stats_for_display.executed(),
        written,
        skipped,
        search_files.len()
    );
    println!(
        "{} {} internal, {} external checked",
        "Links".green().bold(),
        link_result.internal_links,
        link_result.external_checked
    );
    println!(
        "{} in {:.2}s → {} ({})",
        "Done".green().bold(),
        elapsed.as_secs_f64(),
        output_dir.cyan(),
        format_bytes(output_size)
    );

    Ok(())
}

/// Handle a file change event by updating or adding it to Salsa
fn handle_file_changed(
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

    match category {
        PathCategory::Content => {
            if let Ok(content) = fs::read_to_string(path) {
                let last_modified = fs::metadata(path.as_std_path())
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let mut db = server.db.lock().unwrap();
                let mut sources = server.sources.write().unwrap();
                let relative_str = relative.to_string();
                let mut found = false;
                for source in sources.iter() {
                    if source.path(&*db).as_str() == relative_str {
                        use salsa::Setter;
                        source.set_content(&mut *db).to(SourceContent::new(content.clone()));
                        source.set_last_modified(&mut *db).to(last_modified);
                        found = true;
                        break;
                    }
                }
                if !found {
                    let source_path = SourcePath::new(relative_str);
                    let source = SourceFile::new(&*db, source_path, SourceContent::new(content), last_modified);
                    sources.push(source);
                    println!("  {} Added new source: {}", "+".green(), relative);
                }
            }
        }
        PathCategory::Template => {
            if let Ok(content) = fs::read_to_string(path) {
                let mut db = server.db.lock().unwrap();
                let mut templates = server.templates.write().unwrap();
                let relative_str = relative.to_string();
                let mut found = false;
                for template in templates.iter() {
                    if template.path(&*db).as_str() == relative_str {
                        use salsa::Setter;
                        template.set_content(&mut *db).to(TemplateContent::new(content.clone()));
                        found = true;
                        break;
                    }
                }
                if !found {
                    let template_path = TemplatePath::new(relative_str);
                    let template = TemplateFile::new(&*db, template_path, TemplateContent::new(content));
                    templates.push(template);
                    println!("  {} Added new template: {}", "+".green(), relative);
                }
            }
        }
        PathCategory::Sass => {
            if let Ok(content) = fs::read_to_string(path) {
                let mut db = server.db.lock().unwrap();
                let mut sass_files = server.sass_files.write().unwrap();
                let relative_str = relative.to_string();
                let mut found = false;
                for sass_file in sass_files.iter() {
                    if sass_file.path(&*db).as_str() == relative_str {
                        use salsa::Setter;
                        sass_file.set_content(&mut *db).to(SassContent::new(content.clone()));
                        found = true;
                        break;
                    }
                }
                if !found {
                    let sass_path = SassPath::new(relative_str);
                    let sass = SassFile::new(&*db, sass_path, SassContent::new(content));
                    sass_files.push(sass);
                    println!("  {} Added new sass: {}", "+".green(), relative);
                }
            }
        }
        PathCategory::Static => {
            if let Ok(content) = fs::read(path) {
                // Skip empty files (transient state during git operations)
                if content.is_empty() {
                    return;
                }
                let mut db = server.db.lock().unwrap();
                let mut static_files = server.static_files.write().unwrap();
                let relative_str = relative.to_string();

                // Find existing StaticFile and update via set_content() to trigger Salsa invalidation
                let mut found = false;
                for static_file in static_files.iter() {
                    if static_file.path(&*db).as_str() == relative_str {
                        use salsa::Setter;
                        tracing::debug!(path = %relative_str, size = content.len(), "Calling set_content() for static file");
                        static_file.set_content(&mut *db).to(content.clone());
                        found = true;
                        break;
                    }
                }
                if !found {
                    // New file - create a fresh StaticFile
                    let static_path = StaticPath::new(relative_str);
                    let static_file = StaticFile::new(&*db, static_path, content);
                    static_files.push(static_file);
                    println!("  {} Added new static file: {}", "+".green(), relative);
                }
            }
            return;
        }
        PathCategory::Data => {
            if let Ok(content) = fs::read_to_string(path) {
                let mut db = server.db.lock().unwrap();
                let mut data_files = server.data_files.write().unwrap();
                let relative_str = relative.to_string();
                let mut found = false;
                for data_file in data_files.iter() {
                    if data_file.path(&*db).as_str() == relative_str {
                        use salsa::Setter;
                        data_file.set_content(&mut *db).to(DataContent::new(content.clone()));
                        found = true;
                        break;
                    }
                }
                if !found {
                    let data_path = DataPath::new(relative_str);
                    let data_file = DataFile::new(&*db, data_path, DataContent::new(content));
                    data_files.push(data_file);
                    println!("  {} Added new data file: {}", "+".green(), relative);
                }
            }
        }
        PathCategory::Unknown => return, // Unknown files don't need Salsa updates
    }
    // Note: For all file types (Content, Template, Sass, Static, Data), the Salsa input
    // set_content() call triggers proper invalidation of dependent queries.
}

/// Handle a file removal event by removing it from Salsa
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
            let db = server.db.lock().unwrap();
            let mut sources = server.sources.write().unwrap();
            if let Some(pos) = sources.iter().position(|s| s.path(&*db).as_str() == relative_str) {
                sources.remove(pos);
            }
        }
        PathCategory::Template => {
            let db = server.db.lock().unwrap();
            let mut templates = server.templates.write().unwrap();
            if let Some(pos) = templates.iter().position(|t| t.path(&*db).as_str() == relative_str) {
                templates.remove(pos);
            }
        }
        PathCategory::Sass => {
            let db = server.db.lock().unwrap();
            let mut sass_files = server.sass_files.write().unwrap();
            if let Some(pos) = sass_files.iter().position(|s| s.path(&*db).as_str() == relative_str) {
                sass_files.remove(pos);
            }
        }
        PathCategory::Static => {
            let db = server.db.lock().unwrap();
            let mut static_files = server.static_files.write().unwrap();
            if let Some(pos) = static_files.iter().position(|s| s.path(&*db).as_str() == relative_str) {
                static_files.remove(pos);
            }
        }
        PathCategory::Data => {
            let db = server.db.lock().unwrap();
            let mut data_files = server.data_files.write().unwrap();
            if let Some(pos) = data_files.iter().position(|d| d.path(&*db).as_str() == relative_str) {
                data_files.remove(pos);
            }
        }
        PathCategory::Unknown => {}
    }
}

/// Plain serve mode (no TUI) - serves directly from Salsa
async fn serve_plain(
    content_dir: &Utf8PathBuf,
    address: &str,
    port: Option<u16>,
    open: bool,
    stable_assets: Vec<String>,
) -> Result<()> {
    use std::sync::Arc;
    use tokio::sync::watch;

    // Initialize asset cache (processed images, OG images, etc.)
    let parent_dir = content_dir.parent().unwrap_or(content_dir);
    let cache_dir = parent_dir.join(".cache");
    cas::init_asset_cache(cache_dir.as_std_path())?;

    let render_options = render::RenderOptions {
        livereload: true, // Enable live reload in plain mode too
        dev_mode: true,
    };

    // Create the site server
    let server = Arc::new(serve::SiteServer::new(render_options, stable_assets));

    // Load source files
    println!("{}", "Loading source files...".dimmed());
    let md_files: Vec<Utf8PathBuf> = WalkBuilder::new(content_dir)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
        .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
        .collect();

    {
        let db = server.db.lock().unwrap();
        let mut sources = server.sources.write().unwrap();

        for path in &md_files {
            let content = fs::read_to_string(path)?;
            let last_modified = fs::metadata(path)?
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let relative = path
                .strip_prefix(content_dir)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());

            let source_path = SourcePath::new(relative);
            let source_content = SourceContent::new(content);
            let source = SourceFile::new(&*db, source_path, source_content, last_modified);
            sources.push(source);
        }
    }
    println!("  Loaded {} source files", md_files.len());

    // Load templates
    let parent_dir = content_dir.parent().unwrap_or(content_dir);
    let templates_dir = parent_dir.join("templates");

    if templates_dir.exists() {
        let template_files: Vec<Utf8PathBuf> = WalkBuilder::new(&templates_dir)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "html")
                    .unwrap_or(false)
            })
            .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
            .collect();

        let db = server.db.lock().unwrap();
        let mut templates = server.templates.write().unwrap();

        for path in &template_files {
            let content = fs::read_to_string(path)?;
            let relative = path
                .strip_prefix(&templates_dir)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());

            let template_path = TemplatePath::new(relative);
            let template_content = TemplateContent::new(content);
            let template = TemplateFile::new(&*db, template_path, template_content);
            templates.push(template);
        }
        println!("  Loaded {} templates", templates.len());
    }

    // Load static files into Salsa
    let static_dir = parent_dir.join("static");
    if static_dir.exists() {
        let walker = WalkBuilder::new(&static_dir).build();
        let db = server.db.lock().unwrap();
        let mut static_files = server.static_files.write().unwrap();
        let mut count = 0;

        for entry in walker {
            let entry = entry?;
            let path = Utf8Path::from_path(entry.path())
                .ok_or_else(|| eyre!("Non-UTF8 path in static directory"))?;

            if path.is_file() {
                let relative = path.strip_prefix(&static_dir)?;
                let content = fs::read(path)?;

                let static_path = StaticPath::new(relative.to_string());
                let static_file = StaticFile::new(&*db, static_path, content);
                static_files.push(static_file);
                count += 1;
            }
        }
        println!("  Loaded {count} static files");
    }

    // Load data files into Salsa
    let data_dir = parent_dir.join("data");
    if data_dir.exists() {
        let walker = WalkBuilder::new(&data_dir).build();
        let db = server.db.lock().unwrap();
        let mut data_files = server.data_files.write().unwrap();
        let mut count = 0;

        for entry in walker {
            let entry = entry?;
            let path = Utf8Path::from_path(entry.path())
                .ok_or_else(|| eyre!("Non-UTF8 path in data directory"))?;

            if path.is_file() {
                // Only load supported data formats
                let ext = path.extension().unwrap_or("");
                if matches!(ext, "kdl" | "json" | "toml" | "yaml" | "yml") {
                    let relative = path.strip_prefix(&data_dir)?;
                    let content = fs::read_to_string(path)?;

                    let data_path = DataPath::new(relative.to_string());
                    let data_file = DataFile::new(&*db, data_path, DataContent::new(content));
                    data_files.push(data_file);
                    count += 1;
                }
            }
        }
        println!("  Loaded {count} data files");
    }

    // Build search index in background
    println!("{}", "Building search index...".dimmed());
    let server_for_search = server.clone();
    std::thread::spawn(move || {
        match rebuild_search_for_serve(&server_for_search) {
            Ok(search_files) => {
                let count = search_files.len();
                // Debug: print all paths
                let mut paths: Vec<_> = search_files.keys().collect();
                paths.sort();
                for path in &paths {
                    println!("  Search file: {path}");
                }
                let mut sf = server_for_search.search_files.write().unwrap();
                *sf = search_files;
                println!("  Search index ready ({count} files)");
            }
            Err(e) => {
                eprintln!("{} Search index error: {}", "error:".red(), e);
            }
        }
    });

    // Set up file watcher for live reload using the file_watcher module
    // This handles:
    // - File creation, modification, and deletion
    // - File moves/renames (treating as delete + create)
    // - New directory detection (adds to watcher)
    let templates_dir = parent_dir.join("templates");
    let sass_dir = parent_dir.join("sass");
    // data_dir already defined above when loading data files

    // Canonicalize paths for comparison with notify events
    let watcher_config = file_watcher::WatcherConfig {
        content_dir: content_dir.canonicalize_utf8().unwrap_or_else(|_| content_dir.clone()),
        templates_dir: templates_dir.canonicalize_utf8().unwrap_or_else(|_| templates_dir.clone()),
        sass_dir: sass_dir.canonicalize_utf8().unwrap_or_else(|_| sass_dir.clone()),
        static_dir: static_dir.canonicalize_utf8().unwrap_or_else(|_| static_dir.clone()),
        data_dir: data_dir.canonicalize_utf8().unwrap_or_else(|_| data_dir.clone()),
    };

    let (watcher, watcher_rx) = file_watcher::create_watcher(&watcher_config)?;
    let server_for_watcher = server.clone();

    std::thread::spawn(move || {
        use std::sync::mpsc::TryRecvError;
        use std::time::{Duration, Instant};

        let debounce = Duration::from_millis(100);
        let mut last_rebuild = Instant::now() - debounce;
        let watcher = watcher; // keep Arc alive
        let mut pending_events: Vec<file_watcher::FileEvent> = Vec::new();

        loop {
            let event = match watcher_rx.recv() {
                Ok(event) => event,
                Err(_) => break,
            };
            let event = match event {
                Ok(e) => e,
                Err(_) => continue,
            };

            let mut file_events = file_watcher::process_notify_event(event, &watcher_config, &watcher);
            if file_events.is_empty() {
                continue;
            }
            pending_events.append(&mut file_events);

            // Coalesce any immediately available events so a burst is handled together.
            loop {
                match watcher_rx.try_recv() {
                    Ok(Ok(next)) => {
                        let mut more =
                            file_watcher::process_notify_event(next, &watcher_config, &watcher);
                        if !more.is_empty() {
                            pending_events.append(&mut more);
                        }
                    }
                    Ok(Err(_)) => continue,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }

            let elapsed = last_rebuild.elapsed();
            if elapsed < debounce {
                std::thread::sleep(debounce - elapsed);
            }

            for file_event in pending_events.drain(..) {
                match file_event {
                    file_watcher::FileEvent::Changed(path) => {
                        println!("  Changed: {}", path.file_name().unwrap_or("?"));
                        handle_file_changed(&path, &watcher_config, &server_for_watcher);
                    }
                    file_watcher::FileEvent::Removed(path) => {
                        println!("  {} Removed: {}", "-".red(), path.file_name().unwrap_or("?"));
                        handle_file_removed(&path, &watcher_config, &server_for_watcher);
                    }
                    file_watcher::FileEvent::DirectoryCreated(path) => {
                        println!("  {} New directory: {}", "+".green(), path.file_name().unwrap_or("?"));
                    }
                }
            }

            server_for_watcher.trigger_reload();
            last_rebuild = Instant::now();
        }
    });

    // Determine IPs to bind to
    let ips = if address == "0.0.0.0" {
        let mut ips = vec![std::net::Ipv4Addr::LOCALHOST];
        ips.extend(tui::get_lan_ips());
        ips
    } else {
        vec![address.parse().unwrap_or(std::net::Ipv4Addr::LOCALHOST)]
    };

    // Bind first to get actual port
    let bound = serve::bind_listeners(&ips, port).await?;
    let actual_port = bound.port;

    // Print machine-readable port (for tests/scripts)
    println!("LISTENING_PORT={actual_port}");

    // Print human-readable URLs
    print_server_urls(address, actual_port);

    if open {
        let url = format!("http://127.0.0.1:{actual_port}");
        if let Err(e) = open::that(&url) {
            eprintln!("{} Failed to open browser: {}", "warning:".yellow(), e);
        }
    }

    // Now run the server
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    serve::run_on_listeners(server, bound, shutdown_rx).await?;

    Ok(())
}

/// Rebuild search index for serve mode
///
/// Takes a snapshot of the server's current state, builds all HTML via Salsa,
/// and updates the search index. Salsa memoization makes this fast for unchanged pages.
fn rebuild_search_for_serve(server: &serve::SiteServer) -> Result<search::SearchFiles> {
    use crate::db::{SassRegistry, SourceRegistry, StaticRegistry, TemplateRegistry};

    // Lock the database and get current registries
    let db = server.db.lock().map_err(|_| eyre!("db lock poisoned"))?;
    let sources = server
        .sources
        .read()
        .map_err(|_| eyre!("sources lock poisoned"))?;
    let templates = server
        .templates
        .read()
        .map_err(|_| eyre!("templates lock poisoned"))?;
    let sass_files = server
        .sass_files
        .read()
        .map_err(|_| eyre!("sass lock poisoned"))?;

    // Create registries
    let source_registry = SourceRegistry::new(&*db, sources.clone());
    let template_registry = TemplateRegistry::new(&*db, templates.clone());
    let sass_registry = SassRegistry::new(&*db, sass_files.clone());
    let static_registry = StaticRegistry::new(&*db, vec![]); // Empty - search doesn't need static files
    let data_registry = DataRegistry::new(&*db, vec![]); // Empty - search doesn't need data files

    // Build the site (Salsa will cache/reuse unchanged computations)
    let site_output = queries::build_site(
        &*db,
        source_registry,
        template_registry,
        sass_registry,
        static_registry,
        data_registry,
    );

    // Drop locks before async work
    drop(db);
    drop(sources);
    drop(templates);
    drop(sass_files);

    // Build search index in a separate thread with its own runtime
    let output = site_output.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(search::build_search_index(&output))
    })
    .join()
    .map_err(|_| eyre!("search thread panicked"))?
}

/// Serve with TUI progress display and file watching
///
/// This serves content directly from Salsa - no files written to disk.
/// HTTP requests query the Salsa database, which caches/memoizes results.
async fn serve_with_tui(
    content_dir: &Utf8PathBuf,
    _output_dir: &Utf8PathBuf,
    address: &str,
    port: Option<u16>,
    open: bool,
    stable_assets: Vec<String>,
    start_public: bool,
) -> Result<()> {
    use notify::{RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::Arc;
    use std::sync::mpsc;
    use tokio::sync::watch;

    // Initialize asset cache (processed images, OG images, etc.)
    let parent_dir = content_dir.parent().unwrap_or(content_dir);
    let cache_dir = parent_dir.join(".cache");
    cas::init_asset_cache(cache_dir.as_std_path())?;

    // Create channels
    let (progress_tx, progress_rx) = tui::progress_channel();
    let (server_tx, server_rx) = tui::server_status_channel();
    let (event_tx, event_rx) = tui::event_channel();

    // Initialize tracing with TUI layer - routes log events to Activity panel
    let filter_handle = logging::init_tui_tracing(event_tx.clone());

    // Render options with live reload enabled (development mode)
    let render_options = render::RenderOptions {
        livereload: true,
        dev_mode: true,
    };

    // Create the site server - serves directly from Salsa, no disk I/O
    let server = Arc::new(serve::SiteServer::new(render_options, stable_assets));

    // Load cached query results (e.g., processed images) from disk
    let cache_path = content_dir.parent().unwrap_or(content_dir).join(".cache/dodeca.bin");
    if let Err(e) = server.load_cache(cache_path.as_std_path()) {
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
    let salsa_cache_path = base_dir.join(".cache/dodeca.bin");
    let cas_cache_dir = base_dir.join(".cache");
    fn get_cache_sizes(salsa_path: &Utf8Path, cas_dir: &Utf8Path) -> (usize, usize) {
        let salsa_size = salsa_path.metadata().map(|m| m.len() as usize).unwrap_or(0);
        let cas_size = if cas_dir.exists() {
            dir_size(cas_dir).saturating_sub(salsa_size) // Subtract salsa file since it's inside .cache
        } else {
            0
        };
        (salsa_size, cas_size)
    }
    let (salsa_size, cas_size) = get_cache_sizes(&salsa_cache_path, &cas_cache_dir);

    // Set initial server status (use preferred port or 4000 as placeholder until bound)
    let initial_ips = get_bind_ips(initial_mode);
    let display_port = port.unwrap_or(4000);
    let _ = server_tx.send(tui::ServerStatus {
        urls: build_urls(&initial_ips, display_port),
        is_running: false,
        bind_mode: initial_mode,
        salsa_cache_size: salsa_size,
        cas_cache_size: cas_size,
    });

    // Load source files into the server
    let _ = event_tx.send(LogEvent::build("Loading source files..."));
    progress_tx.send_modify(|prog| prog.parse.start(0));

    let md_files: Vec<Utf8PathBuf> = WalkBuilder::new(content_dir)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
        .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
        .collect();

    {
        let db = server.db.lock().unwrap();
        let mut sources = server.sources.write().unwrap();

        for path in &md_files {
            let content = fs::read_to_string(path)?;
            let last_modified = fs::metadata(path)?
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let relative = path
                .strip_prefix(content_dir)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());

            let source_path = SourcePath::new(relative);
            let source_content = SourceContent::new(content);
            let source = SourceFile::new(&*db, source_path, source_content, last_modified);
            sources.push(source);
        }
    }

    progress_tx.send_modify(|prog| prog.parse.finish());
    let _ = event_tx.send(LogEvent::build(format!(
        "Loaded {} source files",
        md_files.len()
    )));

    // Load template files into the server
    let parent_dir = content_dir.parent().unwrap_or(content_dir);
    let templates_dir = parent_dir.join("templates");

    if templates_dir.exists() {
        let template_files: Vec<Utf8PathBuf> = WalkBuilder::new(&templates_dir)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "html")
                    .unwrap_or(false)
            })
            .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
            .collect();

        let db = server.db.lock().unwrap();
        let mut templates = server.templates.write().unwrap();

        for path in &template_files {
            let content = fs::read_to_string(path)?;
            let relative = path
                .strip_prefix(&templates_dir)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());

            let template_path = TemplatePath::new(relative);
            let template_content = TemplateContent::new(content);
            let template = TemplateFile::new(&*db, template_path, template_content);
            templates.push(template);
        }

        let _ = event_tx.send(LogEvent::build(format!(
            "Loaded {} templates",
            templates.len()
        )));
    }

    // Load static files into Salsa
    let static_dir = parent_dir.join("static");
    if static_dir.exists() {
        let walker = WalkBuilder::new(&static_dir).build();
        let db = server.db.lock().unwrap();
        let mut static_files = server.static_files.write().unwrap();
        let mut count = 0;

        for entry in walker {
            let entry = entry?;
            let path = Utf8Path::from_path(entry.path())
                .ok_or_else(|| eyre!("Non-UTF8 path in static directory"))?;

            if path.is_file() {
                let relative = path.strip_prefix(&static_dir)?;
                let content = fs::read(path)?;

                let static_path = StaticPath::new(relative.to_string());
                let static_file = StaticFile::new(&*db, static_path, content);
                static_files.push(static_file);
                count += 1;
            }
        }

        let _ = event_tx.send(LogEvent::build(format!("Loaded {count} static files")));
    }

    // Load SASS files into Salsa (CSS compiled on-demand via query)
    let sass_dir = parent_dir.join("sass");
    if sass_dir.exists() {
        let sass_files_list: Vec<Utf8PathBuf> = WalkBuilder::new(&sass_dir)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "scss" || ext == "sass")
                    .unwrap_or(false)
            })
            .filter_map(|e| Utf8PathBuf::from_path_buf(e.into_path()).ok())
            .collect();

        let db = server.db.lock().unwrap();
        let mut sass_files = server.sass_files.write().unwrap();

        for path in &sass_files_list {
            let content = fs::read_to_string(path)?;
            let relative = path
                .strip_prefix(&sass_dir)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| path.to_string());

            let sass_path = SassPath::new(relative);
            let sass_content = SassContent::new(content);
            let sass_file = SassFile::new(&*db, sass_path, sass_content);
            sass_files.push(sass_file);
        }

        let _ = event_tx.send(LogEvent::build(format!(
            "Loaded {} SASS files",
            sass_files.len()
        )));
    }

    // Build initial search index in background
    let _ = event_tx.send(LogEvent::search("Building search index..."));
    let server_for_search = server.clone();
    let event_tx_for_search = event_tx.clone();
    std::thread::spawn(move || match rebuild_search_for_serve(&server_for_search) {
        Ok(search_files) => {
            let count = search_files.len();
            let mut sf = server_for_search.search_files.write().unwrap();
            *sf = search_files;
            let _ = event_tx_for_search.send(LogEvent::search(format!(
                "Search index ready ({count} pages)"
            )));
        }
        Err(e) => {
            let _ = event_tx_for_search.send(
                LogEvent::error(format!("Search index error: {e}"))
                    .with_kind(crate::tui::EventKind::Search)
            );
        }
    });

    // Mark all tasks as ready - in serve mode, everything is computed on-demand via Salsa
    progress_tx.send_modify(|prog| {
        prog.render.finish();
        prog.sass.finish();
    });

    let _ = event_tx.send(LogEvent::server("Server ready - content served from memory"));

    // Set up file watcher for content and templates
    let (watch_tx, watch_rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(watch_tx, notify::Config::default())?;
    watcher.watch(content_dir.as_std_path(), RecursiveMode::Recursive)?;

    let sass_dir = parent_dir.join("sass");
    let mut watched_dirs = vec![content_dir.to_string()];

    if templates_dir.exists() {
        watcher.watch(templates_dir.as_std_path(), RecursiveMode::Recursive)?;
        watched_dirs.push("templates".to_string());
    }
    if sass_dir.exists() {
        watcher.watch(sass_dir.as_std_path(), RecursiveMode::Recursive)?;
        watched_dirs.push("sass".to_string());
    }
    let static_dir = parent_dir.join("static");
    if static_dir.exists() {
        watcher.watch(static_dir.as_std_path(), RecursiveMode::Recursive)?;
        watched_dirs.push("static".to_string());
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
    let salsa_cache_path_clone = salsa_cache_path.clone();
    let cas_cache_dir_clone = cas_cache_dir.clone();

    let start_server = |server: Arc<serve::SiteServer>,
                        mode: tui::BindMode,
                        preferred_port: Option<u16>,
                        shutdown_rx: watch::Receiver<bool>,
                        server_tx: tui::ServerStatusTx,
                        event_tx: tui::EventTx,
                        salsa_path: Utf8PathBuf,
                        cas_dir: Utf8PathBuf| {
        tokio::spawn(async move {
            let ips = get_bind_ips(mode);

            // Bind to find actual port
            let bound = match serve::bind_listeners(&ips, preferred_port).await {
                Ok(b) => b,
                Err(e) => {
                    let _ = event_tx.send(LogEvent::error(format!("Failed to bind: {e}")));
                    return;
                }
            };
            let actual_port = bound.port;

            // Print machine-readable port for tests
            println!("LISTENING_PORT={actual_port}");

            // Get current cache sizes
            let salsa_size = salsa_path.metadata().map(|m| m.len() as usize).unwrap_or(0);
            let cas_size = WalkBuilder::new(&cas_dir)
                .build()
                .filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len() as usize)
                .sum::<usize>()
                .saturating_sub(salsa_size);

            // Update server status
            let _ = server_tx.send(tui::ServerStatus {
                urls: build_urls(&ips, actual_port),
                is_running: true,
                bind_mode: mode,
                salsa_cache_size: salsa_size,
                cas_cache_size: cas_size,
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

            if let Err(e) = serve::run_on_listeners(server, bound, shutdown_rx).await {
                let _ = event_tx.send(LogEvent::error(format!("Server error: {e}")));
            }
        })
    };

    let mut server_handle = start_server(
        server_for_http.clone(),
        initial_mode,
        port,
        shutdown_rx.clone(),
        server_tx_clone.clone(),
        event_tx_clone.clone(),
        salsa_cache_path_clone.clone(),
        cas_cache_dir_clone.clone(),
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

    // Spawn file watcher handler
    // Canonicalize paths for comparison with notify events (which return absolute paths)
    let content_for_watcher = content_dir
        .canonicalize_utf8()
        .unwrap_or_else(|_| content_dir.clone());
    let templates_dir_for_watcher = templates_dir
        .canonicalize_utf8()
        .unwrap_or_else(|_| templates_dir.clone());
    let sass_dir_for_watcher = sass_dir
        .canonicalize_utf8()
        .unwrap_or_else(|_| sass_dir.clone());
    let static_dir_for_watcher = static_dir
        .canonicalize_utf8()
        .unwrap_or_else(|_| static_dir.clone());
    let event_tx_for_watcher = event_tx.clone();
    let server_for_watcher = server.clone();

    std::thread::spawn(move || {
        use std::time::Instant;
        let mut last_rebuild = Instant::now();
        let debounce = std::time::Duration::from_millis(100);

        while let Ok(res) = watch_rx.recv() {
            match res {
                Ok(event) => {
                    // Debounce rapid events
                    if last_rebuild.elapsed() < debounce {
                        continue;
                    }

                    // React to modify/create/rename events (rename catches atomic saves)
                    use notify::EventKind;
                    match event.kind {
                        EventKind::Modify(_) | EventKind::Create(_) => {
                            // Accept content/template/sass files by extension, or any file in static dir
                            // Filter out temp files (editors write to .tmp then rename)
                            let paths: Vec<Utf8PathBuf> = event
                                .paths
                                .iter()
                                .filter(|p| {
                                    // Skip temp files created by editors
                                    let path_str = p.to_string_lossy();
                                    if path_str.contains(".tmp.") || path_str.ends_with("~") {
                                        return false;
                                    }
                                    let is_known_ext = p.extension()
                                        .map(|e| e == "md" || e == "scss" || e == "html")
                                        .unwrap_or(false);
                                    let is_static = p.starts_with(static_dir_for_watcher.as_std_path());
                                    is_known_ext || is_static
                                })
                                .filter_map(|p| Utf8PathBuf::from_path_buf(p.clone()).ok())
                                .collect();

                            if paths.is_empty() {
                                continue;
                            }

                            for path in &paths {
                                // Determine file type for logging
                                let ext = path.extension();
                                let filename = path.file_name().unwrap_or("?");
                                let file_type = match ext {
                                    Some("md") => "content",
                                    Some("html") => "template",
                                    Some("scss") | Some("sass") => "style",
                                    Some("css") => "css",
                                    Some("js") | Some("ts") => "script",
                                    Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("svg") | Some("webp") | Some("avif") => "image",
                                    Some("woff") | Some("woff2") | Some("ttf") | Some("otf") => "font",
                                    _ => "file",
                                };
                                let _ = event_tx_for_watcher.send(
                                    LogEvent::file_change(format!("{} changed: {}", file_type, filename))
                                );

                                // Update the file in the server's Salsa database
                                if ext == Some("md") {
                                    if let Ok(relative) = path.strip_prefix(&content_for_watcher) {
                                        if let Ok(content) = fs::read_to_string(path) {
                                            let mut db = server_for_watcher.db.lock().unwrap();
                                            let sources =
                                                server_for_watcher.sources.read().unwrap();

                                            // Find existing source and update it
                                            let relative_str = relative.to_string();
                                            for source in sources.iter() {
                                                if source.path(&*db).as_str() == relative_str {
                                                    use salsa::Setter;
                                                    source
                                                        .set_content(&mut *db)
                                                        .to(SourceContent::new(content.clone()));
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                } else if ext == Some("html") {
                                    if let Ok(relative) =
                                        path.strip_prefix(&templates_dir_for_watcher)
                                    {
                                        if let Ok(content) = fs::read_to_string(path) {
                                            let mut db = server_for_watcher.db.lock().unwrap();
                                            let templates =
                                                server_for_watcher.templates.read().unwrap();

                                            // Find existing template and update it
                                            let relative_str = relative.to_string();
                                            for template in templates.iter() {
                                                if template.path(&*db).as_str() == relative_str {
                                                    use salsa::Setter;
                                                    template
                                                        .set_content(&mut *db)
                                                        .to(TemplateContent::new(content.clone()));
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                } else if ext == Some("scss") || ext == Some("sass") {
                                    if let Ok(relative) = path.strip_prefix(&sass_dir_for_watcher) {
                                        if let Ok(content) = fs::read_to_string(path) {
                                            let mut db = server_for_watcher.db.lock().unwrap();
                                            let sass_files =
                                                server_for_watcher.sass_files.read().unwrap();

                                            // Find existing sass file and update it
                                            let relative_str = relative.to_string();
                                            for sass_file in sass_files.iter() {
                                                if sass_file.path(&*db).as_str() == relative_str {
                                                    use salsa::Setter;
                                                    sass_file
                                                        .set_content(&mut *db)
                                                        .to(SassContent::new(content.clone()));
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                } else if path.starts_with(&static_dir_for_watcher) {
                                    // Static file changed (CSS, fonts, images, etc.)
                                    if let Ok(relative) = path.strip_prefix(&static_dir_for_watcher) {
                                        if let Ok(content) = fs::read(path) {
                                            // Skip empty files (transient state during git operations)
                                            if content.is_empty() {
                                                continue;
                                            }
                                            let mut db = server_for_watcher.db.lock().unwrap();
                                            let static_files =
                                                server_for_watcher.static_files.read().unwrap();

                                            // Find existing static file and update it
                                            let relative_str = relative.to_string();
                                            tracing::debug!("Looking for static file: {relative_str}");
                                            for static_file in static_files.iter() {
                                                let stored_path = static_file.path(&*db).as_str().to_string();
                                                if stored_path == relative_str {
                                                    tracing::info!("Updating static file: {relative_str}");
                                                    use salsa::Setter;
                                                    static_file
                                                        .set_content(&mut *db)
                                                        .to(content.clone());
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Trigger live reload - browsers will re-fetch and get fresh content from Salsa
                            // (trigger_reload logs detailed patch/reload decisions)
                            server_for_watcher.trigger_reload();

                            // Rebuild search index in background (debounced with live reload)
                            let server_for_search = server_for_watcher.clone();
                            let event_tx_for_search = event_tx_for_watcher.clone();
                            std::thread::spawn(move || {
                                match rebuild_search_for_serve(&server_for_search) {
                                    Ok(search_files) => {
                                        let count = search_files.len();
                                        let mut sf =
                                            server_for_search.search_files.write().unwrap();
                                        *sf = search_files;
                                        let _ = event_tx_for_search.send(
                                            LogEvent::search(format!("Search index updated ({count} pages)"))
                                        );
                                    }
                                    Err(e) => {
                                        let _ = event_tx_for_search.send(
                                            LogEvent::error(format!("Search rebuild failed: {e}"))
                                                .with_kind(crate::tui::EventKind::Search)
                                        );
                                    }
                                }
                            });

                            last_rebuild = Instant::now();
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    let _ = event_tx_for_watcher.send(LogEvent::error(format!("Watch error: {e}")));
                }
            }
        }
    });

    // Spawn command handler for rebinding
    let server_for_cmd = server.clone();
    let server_tx_for_cmd = server_tx.clone();
    let event_tx_for_cmd = event_tx.clone();
    let salsa_path_for_cmd = salsa_cache_path_clone.clone();
    let cas_dir_for_cmd = cas_cache_dir_clone.clone();
    // Use Arc<Mutex> for the shutdown sender so we can update it for each rebind
    let current_shutdown = Arc::new(std::sync::Mutex::new(shutdown_tx.clone()));
    let current_shutdown_for_handler = current_shutdown.clone();

    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            let new_mode = match cmd {
                tui::ServerCommand::GoPublic => tui::BindMode::Lan,
                tui::ServerCommand::GoLocal => tui::BindMode::Local,
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
                salsa_path_for_cmd.clone(),
                cas_dir_for_cmd.clone(),
            );
        }
    });

    // Run TUI on main thread
    let mut terminal = tui::init_terminal()?;
    let mut app = tui::ServeApp::new(progress_rx, server_rx, event_rx, cmd_tx, filter_handle);
    let _ = app.run(&mut terminal);
    tui::restore_terminal()?;

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
    match server.save_cache(cache_path.as_std_path()) {
        Ok(()) => eprintln!("Cache saved in {:.2?}", cache_start.elapsed()),
        Err(e) => eprintln!("Failed to save cache: {e}"),
    }

    Ok(())
}
