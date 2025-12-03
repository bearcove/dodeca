use crate::types::{
    DataContent, DataPath, HtmlBody, Route, SassContent, SassPath, SourceContent, SourcePath,
    StaticPath, TemplateContent, TemplatePath, Title,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The Salsa database trait for dodeca
#[salsa::db]
pub trait Db: salsa::Database {}

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

/// The concrete database implementation
#[salsa::db]
#[derive(Clone)]
pub struct Database {
    storage: salsa::Storage<Self>,
}

#[salsa::db]
impl salsa::Database for Database {}

#[salsa::db]
impl Db for Database {}

impl Default for Database {
    fn default() -> Self {
        Self::new_without_stats()
    }
}

impl Database {
    /// Create a new database without stats tracking
    pub fn new_without_stats() -> Self {
        Self {
            storage: salsa::Storage::new(None),
        }
    }

    /// Create a new database with stats tracking
    pub fn new_with_stats(stats: Arc<QueryStats>) -> Self {
        let callback = Box::new(move |event: salsa::Event| {
            use salsa::EventKind;
            match event.kind {
                EventKind::WillExecute { .. } => {
                    stats.executed.fetch_add(1, Ordering::Relaxed);
                }
                EventKind::DidValidateMemoizedValue { .. } => {
                    stats.reused.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
        });

        Self {
            storage: salsa::Storage::new(Some(callback)),
        }
    }

    /// Create a new database (alias for default)
    pub fn new() -> Self {
        Self::default()
    }
}

/// Input: A source file with its content
#[salsa::input]
pub struct SourceFile {
    /// The path to this file (relative to content dir)
    #[returns(ref)]
    pub path: SourcePath,

    /// The raw content of the file
    #[returns(ref)]
    pub content: SourceContent,

    /// Last modification time as Unix timestamp (seconds since epoch)
    pub last_modified: i64,
}

/// Input: A template file with its content
#[salsa::input]
pub struct TemplateFile {
    /// The path to this file (relative to templates dir)
    #[returns(ref)]
    pub path: TemplatePath,

    /// The raw content of the template
    #[returns(ref)]
    pub content: TemplateContent,
}

/// Input: A Sass/SCSS file with its content
#[salsa::input]
pub struct SassFile {
    /// The path to this file (relative to sass dir)
    #[returns(ref)]
    pub path: SassPath,

    /// The raw content of the Sass file
    #[returns(ref)]
    pub content: SassContent,
}

/// Input template registry - allows Salsa to track template set as a whole
/// This is an INPUT (not interned) so that updating it invalidates dependent queries
#[salsa::input]
pub struct TemplateRegistry {
    #[returns(ref)]
    pub templates: Vec<TemplateFile>,
}

/// Input sass registry - allows Salsa to track sass file set as a whole
/// This is an INPUT (not interned) so that updating it invalidates dependent queries
#[salsa::input]
pub struct SassRegistry {
    #[returns(ref)]
    pub files: Vec<SassFile>,
}

/// Input: A static file with its binary content
#[salsa::input(persist)]
pub struct StaticFile {
    /// The path to this file (relative to static dir)
    #[returns(ref)]
    pub path: StaticPath,

    /// The binary content of the file
    #[returns(ref)]
    pub content: Vec<u8>,
}

/// Input static file registry - allows Salsa to track static files as a whole
/// This is an INPUT (not interned) so that updating it invalidates dependent queries
#[salsa::input]
pub struct StaticRegistry {
    #[returns(ref)]
    pub files: Vec<StaticFile>,
}

/// Input: A data file (KDL, JSON, TOML, YAML) with its content
#[salsa::input]
pub struct DataFile {
    /// The path to this file (relative to data dir, e.g., "versions.toml")
    #[returns(ref)]
    pub path: DataPath,

    /// The raw content of the file
    #[returns(ref)]
    pub content: DataContent,
}

/// Input data file registry - allows Salsa to track data files as a whole
/// This is an INPUT (not interned) so that updating it invalidates dependent queries
#[salsa::input]
pub struct DataRegistry {
    #[returns(ref)]
    pub files: Vec<DataFile>,
}

/// Input source registry - allows Salsa to track all source files as a whole
/// This is an INPUT (not interned) so that updating it invalidates dependent queries
#[salsa::input]
pub struct SourceRegistry {
    #[returns(ref)]
    pub sources: Vec<SourceFile>,
}

/// Interned character set for font subsetting
/// Using a sorted Vec<char> for deterministic hashing
#[salsa::interned]
pub struct CharSet<'db> {
    #[returns(ref)]
    pub chars: Vec<char>,
}

/// A heading extracted from page/section content
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Heading {
    /// The heading text
    pub title: String,
    /// The anchor ID (for linking)
    pub id: String,
    /// The heading level (1-6)
    pub level: u8,
}

/// A section in the site tree (corresponds to _index.md files)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Section {
    pub route: Route,
    pub title: Title,
    pub description: Option<String>,
    pub weight: i32,
    pub body_html: HtmlBody,
    /// Headings extracted from content
    pub headings: Vec<Heading>,
    /// Last modification time as Unix timestamp (seconds since epoch)
    pub last_updated: i64,
}

/// A page in the site tree (non-index .md files)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Page {
    pub route: Route,
    pub title: Title,
    pub weight: i32,
    pub body_html: HtmlBody,
    pub section_route: Route,
    /// Headings extracted from content
    pub headings: Vec<Heading>,
    /// Last modification time as Unix timestamp (seconds since epoch)
    pub last_updated: i64,
}

/// The complete site tree - sections and pages
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteTree {
    pub sections: std::collections::BTreeMap<Route, Section>,
    pub pages: std::collections::BTreeMap<Route, Page>,
}

/// Rendered HTML output for a page or section
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenderedHtml(pub String);

/// Output of parsing: contains all the data needed for tree building
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Last modification time as Unix timestamp (seconds since epoch)
    pub last_updated: i64,
}

/// A single output file to be written to disk
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OutputFile {
    /// HTML page output: route -> html content
    Html { route: Route, content: String },
    /// CSS output from compiled SASS (path includes cache-bust hash)
    Css { path: StaticPath, content: String },
    /// Static file: relative path -> binary content (path includes cache-bust hash)
    Static { path: StaticPath, content: Vec<u8> },
}

/// Complete site output - all files that need to be written
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteOutput {
    pub files: Vec<OutputFile>,
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ImageVariant {
    /// The encoded image data
    pub data: Vec<u8>,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
}

/// Result of processing an image into responsive formats (JXL + WebP)
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StaticFileOutput {
    /// Cache-busted path (e.g., "fonts/Inter.a1b2c3d4.woff2")
    pub cache_busted_path: String,
    /// Processed content (subsetted font, optimized SVG, etc.)
    pub content: Vec<u8>,
}

/// Output of compiling CSS: cache-busted path and rewritten content
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CssOutput {
    /// Cache-busted path (e.g., "main.a1b2c3d4.css")
    pub cache_busted_path: String,
    /// CSS content with rewritten asset URLs
    pub content: String,
}

/// All rendered HTML content (for font analysis)
/// This is a global cache - rendering all pages once for font char extraction
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllRenderedHtml {
    /// Map of route -> rendered HTML (before URL rewriting)
    pub pages: std::collections::HashMap<Route, String>,
}
