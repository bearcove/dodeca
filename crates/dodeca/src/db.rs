use crate::types::{
    DataContent, DataPath, HtmlBody, Route, SassContent, SassPath, SourceContent, SourcePath,
    StaticPath, TemplateContent, TemplatePath, Title,
};
use std::sync::Arc;

tokio::task_local! {
    /// Task-local storage for the current database Arc.
    /// Set at the start of a request, used by render functions.
    pub static TASK_DB: Arc<Database>;
}
use std::sync::atomic::{AtomicUsize, Ordering};

/// Statistics about query execution
#[derive(Debug, Default)]
pub struct QueryStats {
    /// Number of queries that were executed (cache miss)
    pub executed: AtomicUsize,
    /// Number of queries that were reused (cache hit)
    pub reused: AtomicUsize,
}

impl QueryStats {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn executed(&self) -> usize {
        self.executed.load(Ordering::Relaxed)
    }

    pub fn reused(&self) -> usize {
        self.reused.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> usize {
        self.executed() + self.reused()
    }
}

/// Input: A source file with its content
#[picante::input]
pub struct SourceFile {
    /// The path to this file (relative to content dir)
    #[key]
    pub path: SourcePath,

    /// The raw content of the file
    pub content: SourceContent,

    /// Last modification time as Unix timestamp (seconds since epoch)
    pub last_modified: i64,
}

/// Input: A template file with its content
#[picante::input]
pub struct TemplateFile {
    /// The path to this file (relative to templates dir)
    #[key]
    pub path: TemplatePath,

    /// The raw content of the template
    pub content: TemplateContent,
}

/// Input: A Sass/SCSS file with its content
#[picante::input]
pub struct SassFile {
    /// The path to this file (relative to sass dir)
    #[key]
    pub path: SassPath,

    /// The raw content of the Sass file
    pub content: SassContent,
}

/// Input template registry - tracks template set as a whole (singleton)
#[picante::input]
pub struct TemplateRegistry {
    pub templates: Vec<TemplateFile>,
}

/// Input sass registry - tracks sass file set as a whole (singleton)
#[picante::input]
pub struct SassRegistry {
    pub files: Vec<SassFile>,
}

/// Input: A static file with its binary content
#[picante::input]
pub struct StaticFile {
    /// The path to this file (relative to static dir)
    #[key]
    pub path: StaticPath,

    /// The binary content of the file
    pub content: Vec<u8>,
}

/// Input static file registry - tracks static files as a whole (singleton)
#[picante::input]
pub struct StaticRegistry {
    pub files: Vec<StaticFile>,
}

/// Input: A data file (KDL, JSON, TOML, YAML) with its content
#[picante::input]
pub struct DataFile {
    /// The path to this file (relative to data dir, e.g., "versions.toml")
    #[key]
    pub path: DataPath,

    /// The raw content of the file
    pub content: DataContent,
}

/// Input data file registry - tracks data files as a whole (singleton)
#[picante::input]
pub struct DataRegistry {
    pub files: Vec<DataFile>,
}

/// Input source registry - tracks all source files as a whole (singleton)
#[picante::input]
pub struct SourceRegistry {
    pub sources: Vec<SourceFile>,
}

/// Interned character set for font subsetting
/// Using a sorted Vec<char> for deterministic hashing
#[picante::interned]
pub struct CharSet {
    pub chars: Vec<char>,
}

/// A heading extracted from page/section content
#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
pub struct Heading {
    /// The heading text
    pub title: String,
    /// The anchor ID (for linking)
    pub id: String,
    /// The heading level (1-6)
    pub level: u8,
}

/// A requirement definition for specification traceability.
///
/// Requirements are declared with `r[req.name]` syntax in markdown,
/// similar to the Rust Reference's mdbook-spec.
#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
pub struct ReqDefinition {
    /// The requirement identifier (e.g., "channel.id.allocation")
    pub id: String,
    /// The anchor ID for linking (e.g., "r-channel.id.allocation")
    pub anchor_id: String,
}

/// A section in the site tree (corresponds to _index.md files)
#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
pub struct Section {
    pub route: Route,
    pub title: Title,
    pub description: Option<String>,
    pub weight: i32,
    pub body_html: HtmlBody,
    /// Headings extracted from content
    pub headings: Vec<Heading>,
    /// Requirement definitions for specification traceability
    pub reqs: Vec<ReqDefinition>,
    /// Last modification time as Unix timestamp (seconds since epoch)
    pub last_updated: i64,
    /// Custom fields from the `[extra]` table in frontmatter
    pub extra: facet_value::Value,
}

/// A page in the site tree (non-index .md files)
#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
pub struct Page {
    pub route: Route,
    pub title: Title,
    pub weight: i32,
    pub body_html: HtmlBody,
    pub section_route: Route,
    /// Headings extracted from content
    pub headings: Vec<Heading>,
    /// Rule definitions for specification traceability
    pub rules: Vec<ReqDefinition>,
    /// Last modification time as Unix timestamp (seconds since epoch)
    pub last_updated: i64,
    /// Custom fields from the `[extra]` table in frontmatter
    pub extra: facet_value::Value,
}

/// The complete site tree - sections and pages
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct SiteTree {
    pub sections: std::collections::BTreeMap<Route, Section>,
    pub pages: std::collections::BTreeMap<Route, Page>,
}

/// Rendered HTML output for a page or section
#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
pub struct RenderedHtml(pub String);

/// Output of parsing: contains all the data needed for tree building
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct ParsedData {
    /// Source path relative to content dir
    pub source_path: SourcePath,
    /// URL route (e.g., "/learn/showcases/json/")
    pub route: Route,
    /// Parsed title
    pub title: Title,
    /// Optional description from frontmatter
    pub description: Option<String>,
    /// Weight for sorting
    pub weight: i32,
    /// Body HTML
    pub body_html: HtmlBody,
    /// Is this a section index (_index.md)?
    pub is_section: bool,
    /// Headings extracted from content
    pub headings: Vec<Heading>,
    /// Rule definitions for specification traceability
    pub reqs: Vec<ReqDefinition>,
    /// Last modification time as Unix timestamp (seconds since epoch)
    pub last_updated: i64,
    /// Custom fields from the `[extra]` table in frontmatter
    pub extra: facet_value::Value,
}

/// A single output file to be written to disk
#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
#[repr(C)]
pub enum OutputFile {
    /// HTML page output: route -> html content
    Html { route: Route, content: String },
    /// CSS output from compiled SASS (path includes cache-bust hash)
    Css { path: StaticPath, content: String },
    /// Static file: relative path -> binary content (path includes cache-bust hash)
    Static { path: StaticPath, content: Vec<u8> },
}

/// Complete site output - all files that need to be written
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct SiteOutput {
    pub files: Vec<OutputFile>,
    /// Code execution results for validation
    pub code_execution_results: Vec<CodeExecutionResult>,
}

/// Status of code sample execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum CodeExecutionStatus {
    /// Code was executed and succeeded
    Success,
    /// Code was executed and failed
    Failed,
    /// Code was not executed (noexec, non-Rust, etc.)
    Skipped,
}

/// Result of executing a code sample during build
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct CodeExecutionResult {
    /// Source file where the code sample was found
    pub source_path: String,
    /// Line number in the source file
    pub line: u32,
    /// Programming language of the code sample
    pub language: String,
    /// The actual code that was executed
    pub code: String,
    /// Execution status (Success, Failed, or Skipped)
    pub status: CodeExecutionStatus,
    /// Exit code from execution
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error output
    pub stderr: String,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
    /// Error message if execution failed
    pub error: Option<String>,
    /// Build metadata for reproducibility
    pub metadata: Option<CodeExecutionMetadata>,
}

/// Build metadata captured during code execution for reproducibility
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct CodeExecutionMetadata {
    /// Rust compiler version (from `rustc --version --verbose`)
    pub rustc_version: String,
    /// Cargo version (from `cargo --version`)
    pub cargo_version: String,
    /// Target triple (e.g., "x86_64-unknown-linux-gnu")
    pub target: String,
    /// Build timestamp (ISO 8601 format)
    pub timestamp: String,
    /// Whether shared target cache was used (vs fresh build)
    pub cache_hit: bool,
    /// Platform (e.g., "linux", "macos", "windows")
    pub platform: String,
    /// CPU architecture (e.g., "x86_64", "aarch64")
    pub arch: String,
    /// Dependencies with exact resolved versions
    pub dependencies: Vec<ResolvedDependencyInfo>,
}

/// A resolved dependency with exact version info
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct ResolvedDependencyInfo {
    /// Crate name
    pub name: String,
    /// Exact version
    pub version: String,
    /// Source of the dependency
    pub source: DependencySourceInfo,
}

/// Source information for a resolved dependency
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
#[repr(C)]
pub enum DependencySourceInfo {
    /// crates.io registry
    CratesIo,
    /// Git repository with URL and commit hash
    Git { url: String, commit: String },
    /// Local path dependency
    Path { path: String },
}

/// Result of checking an external link
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExternalLinkStatus {
    /// Link is valid (2xx or 3xx response)
    Ok,
    /// Link returned an error status code
    Error(u16),
    /// Network or other error
    Failed(String),
}

/// A single image variant (one format, one size)
#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
pub struct ImageVariant {
    /// The encoded image data
    pub data: Vec<u8>,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
}

/// Result of processing an image into responsive formats (JXL + WebP)
#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
pub struct ProcessedImages {
    /// Original image width
    pub original_width: u32,
    /// Original image height
    pub original_height: u32,
    /// Thumbhash as base64 data URL (tiny PNG placeholder)
    pub thumbhash_data_url: String,
    /// JXL variants at different widths (sorted by width ascending)
    pub jxl_variants: Vec<ImageVariant>,
    /// WebP variants at different widths (sorted by width ascending)
    pub webp_variants: Vec<ImageVariant>,
}

/// Output of processing a static file: cache-busted path and content
#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
pub struct StaticFileOutput {
    /// Cache-busted path (e.g., "fonts/Inter.a1b2c3d4.woff2")
    pub cache_busted_path: String,
    /// Processed content (subsetted font, optimized SVG, etc.)
    pub content: Vec<u8>,
}

/// Output of compiling CSS: cache-busted path and rewritten content
#[derive(Debug, Clone, PartialEq, Eq, Hash, facet::Facet)]
pub struct CssOutput {
    /// Cache-busted path (e.g., "main.a1b2c3d4.css")
    pub cache_busted_path: String,
    /// CSS content with rewritten asset URLs
    pub content: String,
}

/// All rendered HTML content (for font analysis)
/// This is a global cache - rendering all pages once for font char extraction
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct AllRenderedHtml {
    /// Map of route -> rendered HTML (before URL rewriting)
    pub pages: std::collections::HashMap<Route, String>,
}

/// The picante database for dodeca
#[picante::db(
    inputs(
        SourceFile,
        TemplateFile,
        SassFile,
        StaticFile,
        DataFile,
        TemplateRegistry,
        SassRegistry,
        StaticRegistry,
        DataRegistry,
        SourceRegistry,
    ),
    interned(CharSet, crate::queries::DataValuePath,),
    tracked(
        crate::queries::load_template,
        crate::queries::load_all_templates,
        crate::queries::build_template_lookup,
        crate::queries::load_sass,
        crate::queries::load_all_sass,
        crate::queries::load_all_data_raw,
        crate::queries::data_file_lookup,
        crate::queries::list_data_file_keys,
        crate::queries::load_and_parse_data_file,
        crate::queries::resolve_data_value,
        crate::queries::data_keys_at_path,
        crate::queries::compile_sass,
        crate::queries::parse_file,
        crate::queries::build_tree,
        crate::queries::render_page,
        crate::queries::render_section,
        crate::queries::load_static,
        crate::queries::optimize_svg,
        crate::queries::load_all_static,
        crate::queries::decompress_font,
        crate::queries::subset_font,
        crate::queries::image_metadata,
        crate::queries::image_input_hash,
        crate::queries::process_image,
        crate::queries::build_site,
        crate::queries::all_rendered_html,
        crate::queries::font_char_analysis,
        crate::queries::static_file_output,
        crate::queries::css_output,
        crate::queries::serve_html,
    ),
    db_trait(Db)
)]
pub struct Database {
    /// Optional stats tracking
    pub stats: Option<Arc<QueryStats>>,
}
