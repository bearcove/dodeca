use crate::db::{
    AllRenderedHtml, CharSet, CodeExecutionMetadata, CodeExecutionResult, CssOutput, DataRegistry,
    Db, DependencySourceInfo, Heading, ImageVariant, OutputFile, Page, ParsedData, ProcessedImages,
    RenderedHtml, ReqDefinition, ResolvedDependencyInfo, SassFile, SassRegistry, Section,
    SiteOutput, SiteTree, SourceFile, SourceRegistry, StaticFile, StaticFileOutput, StaticRegistry,
    TemplateFile, TemplateRegistry,
};
use picante::PicanteResult;

use crate::cells::parse_and_render_markdown_cell;
use crate::image::{self, InputFormat, OutputFormat, add_width_suffix};
use crate::types::{HtmlBody, Route, SassContent, StaticPath, TemplateContent, Title};
use crate::url_rewrite::rewrite_urls_in_css;
use facet::Facet;
use facet_value::Value;
use futures_util::future::BoxFuture;
use std::collections::{BTreeMap, HashMap};

/// Load a template file's content - tracked for dependency tracking
#[picante::tracked]
pub async fn load_template<DB: Db>(
    db: &DB,
    template: TemplateFile,
) -> PicanteResult<TemplateContent> {
    template.content(db)
}

/// Load all templates and return a map of path -> content
/// This tracked query records dependencies on all template files
#[picante::tracked]
pub async fn load_all_templates<DB: Db>(db: &DB) -> PicanteResult<HashMap<String, String>> {
    let mut result = HashMap::new();
    let templates = TemplateRegistry::templates(db)?.unwrap_or_default();
    for template in templates.iter() {
        let path = template.path(db)?.as_str().to_string();
        let content = load_template(db, *template).await?;
        result.insert(path, content.as_str().to_string());
    }
    Ok(result)
}

/// Build a lookup table from template path to TemplateFile
#[picante::tracked]
pub async fn build_template_lookup<DB: Db>(
    db: &DB,
) -> PicanteResult<HashMap<String, TemplateFile>> {
    let mut lookup = HashMap::new();
    let templates = TemplateRegistry::templates(db)?.unwrap_or_default();
    for template in templates.iter() {
        let path = template.path(db)?.as_str().to_string();
        lookup.insert(path, *template);
    }
    Ok(lookup)
}

/// A template loader that uses a pre-loaded template map for sync access.
/// Templates are loaded asynchronously before rendering, then this loader
/// provides sync access during template evaluation.
pub struct PicanteTemplateLoader {
    templates: HashMap<String, String>,
}

impl PicanteTemplateLoader {
    /// Create a new loader from pre-loaded templates
    pub fn new(templates: HashMap<String, String>) -> Self {
        Self { templates }
    }
}

impl gingembre::TemplateLoader for PicanteTemplateLoader {
    fn load(&self, name: &str) -> BoxFuture<'_, Option<String>> {
        let result = self.templates.get(name).cloned();
        Box::pin(async move { result })
    }
}

/// Load a sass file's content - tracked for dependency tracking
#[picante::tracked]
pub async fn load_sass<DB: Db>(db: &DB, sass: SassFile) -> PicanteResult<SassContent> {
    sass.content(db)
}

/// Load all sass files and return a map of path -> content
/// This tracked query records dependencies on all sass files
#[picante::tracked]
pub async fn load_all_sass<DB: Db>(db: &DB) -> PicanteResult<HashMap<String, String>> {
    let mut result = HashMap::new();
    let files = SassRegistry::files(db)?.unwrap_or_default();
    for sass in files.iter() {
        let path = sass.path(db)?.as_str().to_string();
        let content = load_sass(db, *sass).await?;
        result.insert(path, content.as_str().to_string());
    }
    Ok(result)
}

/// Load all data files and return their raw content
/// This tracked query records dependencies on all data files
/// The conversion to template Value happens at render time
#[picante::tracked]
pub async fn load_all_data_raw<DB: Db>(db: &DB) -> PicanteResult<Vec<(String, String)>> {
    let files = DataRegistry::files(db)?.unwrap_or_default();
    let mut result = Vec::new();
    for file in files.iter() {
        result.push((
            file.path(db)?.as_str().to_string(),
            file.content(db)?.as_str().to_string(),
        ));
    }
    Ok(result)
}

// ============================================================================
// LAZY DATA LOADING
// ============================================================================

use crate::data::{DataFormat, parse_data_file};
use crate::db::DataFile;
use facet_value::DestructuredRef;
use gingembre::{DataPath, DataResolver};
use std::sync::Arc;

/// An interned path through the data tree.
///
/// For example, `["versions", "dodeca", "version"]` represents `data.versions.dodeca.version`.
/// Interning ensures efficient comparison and hashing.
#[picante::interned]
pub struct DataValuePath {
    pub segments: Vec<String>,
}

/// Build a lookup table from data key (filename without extension) to DataFile.
/// This is tracked so changes to the registry invalidate the lookup.
#[picante::tracked]
pub async fn data_file_lookup<DB: Db>(db: &DB) -> PicanteResult<HashMap<String, DataFile>> {
    let files = DataRegistry::files(db)?.unwrap_or_default();
    let mut result = HashMap::new();
    for f in files.iter() {
        let path = f.path(db)?.as_str().to_string();
        let key = extract_filename_without_extension(&path);
        result.insert(key, *f);
    }
    Ok(result)
}

/// Get all data file keys (filenames without extension).
/// Used for iteration over `data`.
#[picante::tracked]
pub async fn list_data_file_keys<DB: Db>(db: &DB) -> PicanteResult<Vec<String>> {
    let files = DataRegistry::files(db)?.unwrap_or_default();
    let mut result = Vec::new();
    for f in files.iter() {
        result.push(extract_filename_without_extension(f.path(db)?.as_str()));
    }
    Ok(result)
}

/// Load and parse a single data file.
/// Each file load is individually tracked.
#[picante::tracked]
pub async fn load_and_parse_data_file<DB: Db>(
    db: &DB,
    file: DataFile,
) -> PicanteResult<Option<Value>> {
    let path = file.path(db)?;
    let content = file.content(db)?;

    let format = match DataFormat::from_extension(path.as_str()) {
        Some(f) => f,
        None => return Ok(None),
    };
    Ok(parse_data_file(content.as_str(), format).ok())
}

/// Resolve a value at a specific path through the data tree.
///
/// THIS IS THE KEY QUERY - each unique path is tracked separately!
/// When a path is resolved, it's recorded as a dependency of the current query.
#[picante::tracked]
pub async fn resolve_data_value<DB: Db>(
    db: &DB,
    path: DataValuePath,
) -> PicanteResult<Option<Value>> {
    let segments = path.segments(db)?;

    if segments.is_empty() {
        // Root path - can't return a single value, caller should use keys
        return Ok(None);
    }

    // First segment is the file key (filename without extension)
    let file_key = &segments[0];
    let lookup = data_file_lookup(db).await?;
    let file = match lookup.get(file_key) {
        Some(f) => *f,
        None => return Ok(None),
    };

    // Load and parse the file (this is tracked!)
    let parsed = match load_and_parse_data_file(db, file).await? {
        Some(v) => v,
        None => return Ok(None),
    };

    // Navigate to the specific path within the parsed value
    let mut current = parsed;
    for segment in segments.iter().skip(1) {
        current = match current.destructure_ref() {
            DestructuredRef::Object(obj) => match obj.get(segment.as_str()) {
                Some(v) => v.clone(),
                None => return Ok(None),
            },
            DestructuredRef::Array(arr) => {
                let idx: usize = match segment.parse() {
                    Ok(i) => i,
                    Err(_) => return Ok(None),
                };
                match arr.get(idx) {
                    Some(v) => v.clone(),
                    None => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
    }

    Ok(Some(current))
}

/// Get child keys at a path (for iteration).
/// Returns the keys at that path if it's an object, or indices if it's an array.
#[picante::tracked]
pub async fn data_keys_at_path<DB: Db>(db: &DB, path: DataValuePath) -> PicanteResult<Vec<String>> {
    let segments = path.segments(db)?;

    if segments.is_empty() {
        // Root path - return all data file keys
        return list_data_file_keys(db).await;
    }

    // First, resolve the value at this path
    let value = match resolve_data_value(db, path).await? {
        Some(v) => v,
        None => return Ok(Vec::new()),
    };

    // Return keys based on the value type
    Ok(match value.destructure_ref() {
        DestructuredRef::Object(obj) => obj.keys().map(|k| k.to_string()).collect(),
        DestructuredRef::Array(arr) => (0..arr.len()).map(|i| i.to_string()).collect(),
        _ => Vec::new(),
    })
}

/// Extract filename without extension from a path.
fn extract_filename_without_extension(path: &str) -> String {
    let filename = path.rsplit('/').next().unwrap_or(path);
    if let Some(dot_pos) = filename.rfind('.') {
        filename[..dot_pos].to_string()
    } else {
        filename.to_string()
    }
}

/// A simple sync data resolver that works with pre-loaded data.
/// Data is loaded asynchronously before rendering, then this resolver
/// provides sync access during template evaluation.
pub struct SyncDataResolver {
    data: Value,
}

impl SyncDataResolver {
    /// Create a new resolver from pre-loaded data
    #[allow(clippy::new_ret_no_self)] // Returns trait object Arc<dyn DataResolver>
    pub fn new(data: Value) -> Arc<dyn DataResolver> {
        Arc::new(Self { data })
    }
}

impl DataResolver for SyncDataResolver {
    fn resolve(
        &self,
        path: &DataPath,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Value>> + Send + '_>> {
        let mut current = &self.data;
        for segment in path.segments() {
            match current.destructure_ref() {
                DestructuredRef::Object(obj) => {
                    if let Some(v) = obj.get(segment) {
                        current = v;
                    } else {
                        return Box::pin(async { None });
                    }
                }
                DestructuredRef::Array(arr) => {
                    let idx: usize = match segment.parse() {
                        Ok(i) => i,
                        Err(_) => return Box::pin(async { None }),
                    };
                    if let Some(v) = arr.get(idx) {
                        current = v;
                    } else {
                        return Box::pin(async { None });
                    }
                }
                _ => return Box::pin(async { None }),
            }
        }
        let result = current.clone();
        Box::pin(async move { Some(result) })
    }

    fn keys_at(
        &self,
        path: &DataPath,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Vec<String>>> + Send + '_>> {
        let mut current = &self.data;
        for segment in path.segments() {
            match current.destructure_ref() {
                DestructuredRef::Object(obj) => {
                    if let Some(v) = obj.get(segment) {
                        current = v;
                    } else {
                        return Box::pin(async { None });
                    }
                }
                DestructuredRef::Array(arr) => {
                    let idx: usize = match segment.parse() {
                        Ok(i) => i,
                        Err(_) => return Box::pin(async { None }),
                    };
                    if let Some(v) = arr.get(idx) {
                        current = v;
                    } else {
                        return Box::pin(async { None });
                    }
                }
                _ => return Box::pin(async { None }),
            }
        }
        let result = match current.destructure_ref() {
            DestructuredRef::Object(obj) => Some(obj.iter().map(|(k, _)| k.to_string()).collect()),
            DestructuredRef::Array(arr) => Some((0..arr.len()).map(|i| i.to_string()).collect()),
            _ => None,
        };
        Box::pin(async move { result })
    }
    // len_at has default implementation in the trait
}

/// Compiled CSS output
#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
pub struct CompiledCss(pub String);

/// Compile SASS to CSS - tracked for dependency tracking
/// Returns None if compilation fails
#[picante::tracked]
#[tracing::instrument(skip_all, name = "compile_sass")]
pub async fn compile_sass<DB: Db>(db: &DB) -> PicanteResult<Option<CompiledCss>> {
    // Load all sass files - creates dependency on each
    let sass_map = load_all_sass(db).await?;

    // Skip compilation if no main.scss entry point exists
    if !sass_map.contains_key("main.scss") {
        if !sass_map.is_empty() {
            tracing::debug!("SCSS files found but no main.scss entry point, skipping compilation");
        }
        return Ok(None);
    }

    // Compile via cell
    match crate::cells::compile_sass_cell(&sass_map).await {
        Ok(css) => Ok(Some(CompiledCss(css))),
        Err(e) => {
            tracing::error!("SASS compilation failed: {}", e);
            Ok(None)
        }
    }
}

/// Frontmatter parsed from TOML
///
/// Known fields are extracted; the `extra` table is preserved as-is for template access.
#[derive(Debug, Clone, Default, Facet)]
#[allow(dead_code)] // Fields reserved for future template use
pub struct Frontmatter {
    #[facet(default)]
    pub title: String,
    #[facet(default)]
    pub weight: i32,
    pub description: Option<String>,
    pub template: Option<String>,
    /// Custom fields from the `[extra]` table in frontmatter
    #[facet(default)]
    pub extra: Value,
}

/// Result of parsing a source file
pub type ParseFileResult = Result<ParsedData, crate::cells::MarkdownParseError>;

/// Parse a source file into ParsedData
/// This is the main tracked function - memoizes the result
#[picante::tracked]
pub async fn parse_file<DB: Db>(db: &DB, source: SourceFile) -> PicanteResult<ParseFileResult> {
    let content = source.content(db)?;
    let path = source.path(db)?;
    let last_modified = source.last_modified(db)?;

    // Use the markdown cell to parse frontmatter and render markdown
    let parsed = match parse_and_render_markdown_cell(path.as_str(), content.as_str()).await {
        Ok(p) => p,
        Err(e) => return Ok(Err(e)),
    };

    // Convert frontmatter from cell type
    let extra: Value = parsed.frontmatter.extra.clone();

    // HTML is already fully rendered by marq with code blocks highlighted
    let html_output = parsed.html;

    // Convert headings from cell type to internal type
    let headings: Vec<Heading> = parsed
        .headings
        .into_iter()
        .map(|h| Heading {
            title: h.title,
            id: h.id,
            level: h.level,
        })
        .collect();

    // Convert rules from cell type to internal type
    let reqs: Vec<ReqDefinition> = parsed
        .reqs
        .into_iter()
        .map(|r| ReqDefinition {
            id: r.id,
            anchor_id: r.anchor_id,
        })
        .collect();

    let body_html = HtmlBody::new(html_output);

    // Determine if this is a section (_index.md)
    let is_section = path.is_section_index();

    // Compute URL route
    let route = path.to_route();

    Ok(Ok(ParsedData {
        source_path: (*path).clone(),
        route,
        title: Title::new(parsed.frontmatter.title),
        description: parsed.frontmatter.description,
        weight: parsed.frontmatter.weight,
        body_html,
        is_section,
        headings,
        reqs,
        last_updated: last_modified,
        extra,
    }))
}

/// A parse error with its source file path
#[derive(Debug, Clone, facet::Facet)]
pub struct SourceParseError {
    pub path: String,
    pub error: crate::cells::MarkdownParseError,
}

impl std::fmt::Display for SourceParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.error)
    }
}

/// Error when building site tree due to parse errors
#[derive(Debug, Clone, facet::Facet)]
pub struct BuildError {
    pub errors: Vec<SourceParseError>,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Failed to parse {} file(s):", self.errors.len())?;
        for err in &self.errors {
            writeln!(f, "  - {}", err)?;
        }
        Ok(())
    }
}

impl std::error::Error for BuildError {}

/// Result of building the site tree
pub type BuildTreeResult = Result<SiteTree, Vec<SourceParseError>>;

/// Build the site tree from all source files
/// This tracked query depends on all parse_file results
#[picante::tracked]
pub async fn build_tree<DB: Db>(db: &DB) -> PicanteResult<BuildTreeResult> {
    let mut sections: BTreeMap<Route, Section> = BTreeMap::new();
    let mut pages: BTreeMap<Route, Page> = BTreeMap::new();

    // Parse all files - this creates dependencies on each parse_file
    let sources = SourceRegistry::sources(db)?.unwrap_or_default();
    let mut parsed = Vec::new();
    let mut errors = Vec::new();

    for source in sources.iter() {
        let path = source.path(db)?;
        match parse_file(db, *source).await? {
            Ok(data) => parsed.push(data),
            Err(e) => errors.push(SourceParseError {
                path: path.to_string(),
                error: e,
            }),
        }
    }

    if !errors.is_empty() {
        return Ok(Err(errors));
    }

    // First pass: create all sections
    for data in parsed.iter().filter(|d| d.is_section) {
        sections.insert(
            data.route.clone(),
            Section {
                route: data.route.clone(),
                title: data.title.clone(),
                description: data.description.clone(),
                weight: data.weight,
                body_html: data.body_html.clone(),
                headings: data.headings.clone(),
                reqs: data.reqs.clone(),
                last_updated: data.last_updated,
                extra: data.extra.clone(),
            },
        );
    }

    // Ensure root section exists
    sections.entry(Route::root()).or_insert_with(|| Section {
        route: Route::root(),
        title: Title::from_static("Home"),
        description: None,
        weight: 0,
        body_html: HtmlBody::from_static(""),
        headings: Vec::new(),
        reqs: Vec::new(),
        last_updated: 0,
        extra: Value::default(),
    });

    // Second pass: create pages and assign to sections
    for data in parsed.iter().filter(|d| !d.is_section) {
        let section_route = find_parent_section(&data.route, &sections);
        pages.insert(
            data.route.clone(),
            Page {
                route: data.route.clone(),
                title: data.title.clone(),
                weight: data.weight,
                body_html: data.body_html.clone(),
                section_route,
                headings: data.headings.clone(),
                rules: data.reqs.clone(),
                last_updated: data.last_updated,
                extra: data.extra.clone(),
            },
        );
    }

    Ok(Ok(SiteTree { sections, pages }))
}

/// Find the nearest parent section for a route
fn find_parent_section(route: &Route, sections: &BTreeMap<Route, Section>) -> Route {
    let mut current = route.clone();

    loop {
        if sections.contains_key(&current) && current != *route {
            return current;
        }

        match current.parent() {
            Some(parent) => current = parent,
            None => return Route::root(),
        }
    }
}

/// Render a single page to HTML
/// This tracked query depends on the page content, templates actually used, data files, and site tree.
/// Template dependencies are tracked lazily - only templates loaded during rendering are recorded.
/// Data dependencies are also tracked lazily - only data paths actually accessed become dependencies.
#[picante::tracked]
#[tracing::instrument(skip_all, name = "render_page")]
pub async fn render_page<DB: Db>(
    db: &DB,
    route: Route,
) -> PicanteResult<Result<RenderedHtml, BuildError>> {
    use crate::render::{render_page_via_cell, render_page_with_resolver};

    // Build tree (cached)
    let site_tree = match build_tree(db).await? {
        Ok(tree) => tree,
        Err(errors) => return Ok(Err(BuildError { errors })),
    };

    // Pre-load all templates for sync access during rendering
    let templates = load_all_templates(db).await?;

    // Find the page
    let page = site_tree
        .pages
        .get(&route)
        .expect("Page not found for route");

    // Try cell-based rendering (falls back to direct if cell unavailable)
    let html = render_page_via_cell(page, &site_tree, templates.clone()).await;

    // Check if we got an error (cell unavailable) and need to fallback
    if html.contains(crate::render::RENDER_ERROR_MARKER) {
        // Fallback: use direct rendering with resolver
        let raw_data = load_all_data_raw(db).await?;
        let data_value = crate::data::parse_raw_data_files(&raw_data);
        let resolver = SyncDataResolver::new(data_value);
        let loader = PicanteTemplateLoader::new(templates);
        let html = render_page_with_resolver(page, &site_tree, loader, resolver).await;
        Ok(Ok(RenderedHtml(html)))
    } else {
        Ok(Ok(RenderedHtml(html)))
    }
}

/// Render a single section to HTML
/// This tracked query depends on the section content, templates actually used, data files, and site tree.
/// Template dependencies are tracked lazily - only templates loaded during rendering are recorded.
/// Data dependencies are also tracked lazily - only data paths actually accessed become dependencies.
#[picante::tracked]
#[tracing::instrument(skip_all, name = "render_section")]
pub async fn render_section<DB: Db>(
    db: &DB,
    route: Route,
) -> PicanteResult<Result<RenderedHtml, BuildError>> {
    use crate::render::{render_section_via_cell, render_section_with_resolver};

    // Build tree (cached)
    let site_tree = match build_tree(db).await? {
        Ok(tree) => tree,
        Err(errors) => return Ok(Err(BuildError { errors })),
    };

    // Pre-load all templates for sync access during rendering
    let templates = load_all_templates(db).await?;

    // Find the section
    let section = site_tree
        .sections
        .get(&route)
        .expect("Section not found for route");

    // Try cell-based rendering (falls back to direct if cell unavailable)
    let html = render_section_via_cell(section, &site_tree, templates.clone()).await;

    // Check if we got an error (cell unavailable) and need to fallback
    if html.contains(crate::render::RENDER_ERROR_MARKER) {
        // Fallback: use direct rendering with resolver
        let raw_data = load_all_data_raw(db).await?;
        let data_value = crate::data::parse_raw_data_files(&raw_data);
        let resolver = SyncDataResolver::new(data_value);
        let loader = PicanteTemplateLoader::new(templates);
        let html = render_section_with_resolver(section, &site_tree, loader, resolver).await;
        Ok(Ok(RenderedHtml(html)))
    } else {
        Ok(Ok(RenderedHtml(html)))
    }
}

/// Load a single static file's content - tracked
#[picante::tracked]
pub async fn load_static<DB: Db>(db: &DB, file: StaticFile) -> PicanteResult<Vec<u8>> {
    let content = file.content(db)?.clone();
    tracing::debug!(path = %file.path(db)?.as_str(), size = content.len(), "load_static called");
    Ok(content)
}

/// Process an SVG file - tracked
/// Currently passes through unchanged (optimization disabled)
#[picante::tracked]
pub async fn optimize_svg<DB: Db>(db: &DB, file: StaticFile) -> PicanteResult<Vec<u8>> {
    let content = file.content(db)?;

    // Try to parse as UTF-8 string
    let Ok(svg_str) = std::str::from_utf8(&content) else {
        return Ok(content.to_vec());
    };

    // Process SVG (currently passthrough)
    match crate::svg::optimize_svg(svg_str).await {
        Some(optimized) => Ok(optimized.into_bytes()),
        None => Ok(content.to_vec()),
    }
}

/// Load all static files - returns map of path -> content
#[picante::tracked]
pub async fn load_all_static<DB: Db>(db: &DB) -> PicanteResult<HashMap<String, Vec<u8>>> {
    let mut result = HashMap::new();
    let files = StaticRegistry::files(db)?.unwrap_or_default();
    for file in files.iter() {
        let path = file.path(db)?.as_str().to_string();
        let content = load_static(db, *file).await?;
        result.insert(path, content);
    }
    Ok(result)
}

/// Decompress a font file (WOFF2/WOFF1 -> TTF/OTF)
/// Results are cached in the CAS to avoid repeated decompression
#[picante::tracked]
#[tracing::instrument(skip_all, name = "decompress_font")]
pub async fn decompress_font<DB: Db>(
    db: &DB,
    font_file: StaticFile,
) -> PicanteResult<Option<Vec<u8>>> {
    use crate::cas::{
        font_content_hash, get_cached_decompressed_font, put_cached_decompressed_font,
    };
    use crate::cells::decompress_font_cell;

    let path = font_file.path(db)?.as_str().to_string();
    tracing::debug!(
        font_path = %path,
        "🟡 QUERY: decompress_font COMPUTING (picante cache miss)"
    );

    let font_data = font_file.content(db)?;
    let content_hash = font_content_hash(&font_data);

    // Check CAS cache first
    if let Some(cached) = get_cached_decompressed_font(&content_hash) {
        tracing::debug!(
            "Font decompression cache hit for {}",
            font_file.path(db)?.as_str()
        );
        return Ok(Some(cached));
    }

    // Decompress the font via cell
    match decompress_font_cell(&font_data).await {
        Some(decompressed) => {
            // Cache the result
            put_cached_decompressed_font(&content_hash, &decompressed);
            tracing::debug!(
                "Decompressed font {} ({} -> {} bytes)",
                font_file.path(db)?.as_str(),
                font_data.len(),
                decompressed.len()
            );
            Ok(Some(decompressed))
        }
        None => {
            tracing::warn!("Failed to decompress font {}", font_file.path(db)?.as_str());
            Ok(None)
        }
    }
}

/// Subset a font file to only include specified characters
/// Returns WOFF2 compressed bytes, or None if subsetting fails
#[picante::tracked]
#[tracing::instrument(skip_all, name = "subset_font")]
pub async fn subset_font<DB: Db>(
    db: &DB,
    font_file: StaticFile,
    chars: CharSet,
) -> PicanteResult<Option<Vec<u8>>> {
    use crate::cells::{compress_to_woff2_cell, subset_font_cell};

    let path = font_file.path(db)?.as_str().to_string();
    let num_chars = chars.chars(db).map(|c| c.len()).unwrap_or(0);
    tracing::debug!(
        font_path = %path,
        num_chars,
        "🟡 QUERY: subset_font COMPUTING (picante cache miss)"
    );

    // First, decompress the font (handles WOFF2/WOFF1 -> TTF)
    let Some(decompressed) = decompress_font(db, font_file).await? else {
        return Ok(None);
    };

    let char_vec: Vec<char> = chars.chars(db)?.to_vec();

    // Subset the decompressed TTF via cell
    let subsetted = match subset_font_cell(&decompressed, &char_vec).await {
        Some(data) => data,
        None => {
            tracing::warn!("Failed to subset font {}", font_file.path(db)?.as_str());
            return Ok(None);
        }
    };

    // Compress back to WOFF2 via cell
    match compress_to_woff2_cell(&subsetted).await {
        Some(woff2) => {
            tracing::debug!(
                "Subsetted font {} ({} chars, {} -> {} bytes)",
                font_file.path(db)?.as_str(),
                char_vec.len(),
                decompressed.len(),
                woff2.len()
            );
            Ok(Some(woff2))
        }
        None => {
            tracing::warn!(
                "Failed to compress font {} to WOFF2",
                font_file.path(db)?.as_str()
            );
            Ok(None)
        }
    }
}

/// Get image metadata (dimensions, thumbhash, variant widths) without full processing
/// This is fast - only decodes the image, doesn't encode to JXL/WebP
#[picante::tracked]
pub async fn image_metadata<DB: Db>(
    db: &DB,
    image_file: StaticFile,
) -> PicanteResult<Option<image::ImageMetadata>> {
    let path = image_file.path(db)?;
    let input_format = InputFormat::from_extension(path.as_str());
    let Some(input_format) = input_format else {
        return Ok(None);
    };
    let data = image_file.content(db)?;
    Ok(image::get_image_metadata(&data, input_format).await)
}

/// Get the input hash for an image file (for cache-busted URLs)
#[picante::tracked]
pub async fn image_input_hash<DB: Db>(
    db: &DB,
    image_file: StaticFile,
) -> PicanteResult<crate::cas::InputHash> {
    use crate::cas::content_hash_32;
    let data = image_file.content(db)?;
    Ok(content_hash_32(&data))
}

/// Process an image file into responsive formats (JXL + WebP) with multiple widths
/// Returns None if the image cannot be processed or is not a supported format
///
/// Uses CAS (Content-Addressable Storage) to cache processed images across restarts.
/// The cache key is a 32-byte hash of the input image content.
#[picante::tracked] // No persist - CAS handles caching, don't bloat DB with image bytes
#[tracing::instrument(skip_all, name = "process_image")]
pub async fn process_image<DB: Db>(
    db: &DB,
    image_file: StaticFile,
) -> PicanteResult<Option<ProcessedImages>> {
    use crate::cas::{content_hash_32, get_cached_image, put_cached_image};

    let path = image_file.path(db)?;
    let Some(input_format) = InputFormat::from_extension(path.as_str()) else {
        return Ok(None);
    };
    let data = image_file.content(db)?;

    // Compute content hash for cache lookup
    let content_hash = content_hash_32(&data);

    // Check CAS cache first
    if let Some(cached) = get_cached_image(&content_hash) {
        tracing::debug!("Image cache hit for {}", path.as_str());
        return Ok(Some(cached));
    }

    tracing::debug!("Image cache miss for {}", path.as_str());

    let Some(processed) = image::process_image(&data, input_format).await else {
        return Ok(None);
    };

    let result = ProcessedImages {
        original_width: processed.original_width,
        original_height: processed.original_height,
        thumbhash_data_url: processed.thumbhash_data_url,
        jxl_variants: processed
            .jxl_variants
            .into_iter()
            .map(|v| ImageVariant {
                data: v.data,
                width: v.width,
                height: v.height,
            })
            .collect(),
        webp_variants: processed
            .webp_variants
            .into_iter()
            .map(|v| ImageVariant {
                data: v.data,
                width: v.width,
                height: v.height,
            })
            .collect(),
    };

    // Store in CAS cache for next time
    put_cached_image(&content_hash, &result);

    Ok(Some(result))
}

/// Build the complete site - THE top-level query
/// This produces all output files that need to be written to disk.
/// Fonts are automatically subsetted, all assets are cache-busted.
///
/// This reuses the same queries as the serve pipeline (serve_html, css_output,
/// static_file_output) to ensure consistency between `ddc build` and `ddc serve`.
#[picante::tracked]
pub async fn build_site<DB: Db>(db: &DB) -> PicanteResult<Result<SiteOutput, BuildError>> {
    let mut files = Vec::new();

    // Build the site tree to get all routes
    let site_tree = match build_tree(db).await? {
        Ok(tree) => tree,
        Err(errors) => return Ok(Err(BuildError { errors })),
    };

    // --- Phase 1: Render all HTML pages using serve_html ---
    // This reuses the exact same pipeline as `ddc serve`, ensuring consistency
    for route in site_tree.sections.keys() {
        match serve_html(db, route.clone()).await? {
            Ok(Some(html)) => {
                files.push(OutputFile::Html {
                    route: route.clone(),
                    content: html,
                });
            }
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    for route in site_tree.pages.keys() {
        match serve_html(db, route.clone()).await? {
            Ok(Some(html)) => {
                files.push(OutputFile::Html {
                    route: route.clone(),
                    content: html,
                });
            }
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    // --- Phase 2: Add CSS output ---
    if let Some(css) = css_output(db).await? {
        files.push(OutputFile::Css {
            path: StaticPath::new(css.cache_busted_path),
            content: css.content,
        });
    }

    // --- Phase 3: Process static files ---
    let static_files = StaticRegistry::files(db)?.unwrap_or_default();
    for file in static_files.iter() {
        let path = file.path(db)?.as_str().to_string();

        // Check if this is a processable image (PNG, JPG, GIF, WebP, JXL)
        if InputFormat::is_processable(&path) {
            // Process the image into JXL and WebP variants at multiple widths
            if let Some(processed) = process_image(db, *file).await? {
                use crate::cas::ImageVariantKey;

                let input_hash = image_input_hash(db, *file).await?;

                // Output each JXL variant
                for variant in &processed.jxl_variants {
                    let base_path = image::change_extension(&path, OutputFormat::Jxl.extension());
                    let variant_path = if variant.width == processed.original_width {
                        base_path
                    } else {
                        add_width_suffix(&base_path, variant.width)
                    };
                    let key = ImageVariantKey {
                        input_hash,
                        format: OutputFormat::Jxl,
                        width: variant.width,
                    };
                    let cache_busted = format!(
                        "{}.{}.jxl",
                        variant_path.trim_end_matches(".jxl"),
                        key.url_hash()
                    );
                    files.push(OutputFile::Static {
                        path: StaticPath::new(cache_busted),
                        content: variant.data.clone(),
                    });
                }

                // Output each WebP variant
                for variant in &processed.webp_variants {
                    let base_path = image::change_extension(&path, OutputFormat::WebP.extension());
                    let variant_path = if variant.width == processed.original_width {
                        base_path
                    } else {
                        add_width_suffix(&base_path, variant.width)
                    };
                    let key = ImageVariantKey {
                        input_hash,
                        format: OutputFormat::WebP,
                        width: variant.width,
                    };
                    let cache_busted = format!(
                        "{}.{}.webp",
                        variant_path.trim_end_matches(".webp"),
                        key.url_hash()
                    );
                    files.push(OutputFile::Static {
                        path: StaticPath::new(cache_busted),
                        content: variant.data.clone(),
                    });
                }

                // Don't output the original image (replaced by JXL/WebP)
                continue;
            }
            // If processing failed, fall through to output the original
        }

        // Use static_file_output for all other static files (fonts, CSS, SVGs, etc.)
        // This handles font subsetting, CSS URL rewriting, and SVG optimization
        let output = static_file_output(db, *file).await?;
        files.push(OutputFile::Static {
            path: StaticPath::new(output.cache_busted_path),
            content: output.content,
        });
    }

    // --- Phase 4: Execute code samples for validation ---
    let code_execution_results = execute_all_code_samples(db).await?;

    Ok(Ok(SiteOutput {
        files,
        code_execution_results,
    }))
}

// ============================================================================
// Lazy serve queries - for on-demand page rendering
// ============================================================================

/// Render all pages and sections to HTML (without URL rewriting)
/// This is cached globally and used for font character analysis
#[picante::tracked]
pub async fn all_rendered_html<DB: Db>(
    db: &DB,
) -> PicanteResult<Result<AllRenderedHtml, BuildError>> {
    let site_tree = match build_tree(db).await? {
        Ok(tree) => tree,
        Err(errors) => return Ok(Err(BuildError { errors })),
    };
    let template_map = load_all_templates(db).await?;

    // Load data files and convert to template Value
    let raw_data = load_all_data_raw(db).await?;
    let data_value = crate::data::parse_raw_data_files(&raw_data);

    let mut pages = HashMap::new();

    for (route, section) in &site_tree.sections {
        let html = crate::render::render_section_to_html(
            section,
            &site_tree,
            &template_map,
            Some(data_value.clone()),
        )
        .await;
        pages.insert(route.clone(), html);
    }

    for (route, page) in &site_tree.pages {
        let html = crate::render::render_page_to_html(
            page,
            &site_tree,
            &template_map,
            Some(data_value.clone()),
        )
        .await;
        pages.insert(route.clone(), html);
    }

    Ok(Ok(AllRenderedHtml { pages }))
}

/// Local font-face representation for picante tracking
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct LocalFontFace {
    pub family: String,
    pub src: String,
    pub weight: Option<String>,
    pub style: Option<String>,
}

/// Local font analysis result for picante tracking
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct LocalFontAnalysis {
    pub chars_per_font: std::collections::HashMap<String, Vec<char>>,
    pub font_faces: Vec<LocalFontFace>,
}

/// Analyze all rendered HTML for font character usage
/// Returns the FontAnalysis needed for font subsetting
#[picante::tracked]
pub async fn font_char_analysis<DB: Db>(db: &DB) -> PicanteResult<LocalFontAnalysis> {
    let all_html = all_rendered_html(db)
        .await?
        .expect("build errors should be caught before font analysis");
    let sass_css = compile_sass(db).await?;
    let sass_str = sass_css.as_ref().map(|c| c.0.as_str()).unwrap_or("");

    // Collect CSS from static files
    let mut static_css_parts = Vec::new();
    let static_files = StaticRegistry::files(db)?.unwrap_or_default();
    for file in static_files.iter() {
        let path = file.path(db)?.as_str().to_string();
        if path.to_lowercase().ends_with(".css") {
            let content = load_static(db, *file).await?;
            if let Ok(css_str) = String::from_utf8(content) {
                static_css_parts.push(css_str);
            }
        }
    }
    let static_css = static_css_parts.join("\n");

    // Combine all HTML for analysis
    let combined_html: String = all_html
        .pages
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    // Missing cells are hard errors - this panic gives an actionable message.
    let inline_css = crate::cells::extract_css_from_html_cell(&combined_html)
        .await
        .expect(
            "fonts cell not available: ddc-cell-fonts binary missing or failed to start. \
             Check DODECA_CELL_PATH and ensure all cell binaries are built.",
        );
    let all_css = format!("{sass_str}\n{static_css}\n{inline_css}");

    let analysis = crate::cells::analyze_fonts_cell(&combined_html, &all_css)
        .await
        .expect(
            "fonts cell not available: ddc-cell-fonts binary missing or failed to start. \
             Check DODECA_CELL_PATH and ensure all cell binaries are built.",
        );

    let font_faces = analysis
        .font_faces
        .into_iter()
        .map(|face| LocalFontFace {
            family: face.family,
            src: face.src,
            weight: face.weight,
            style: face.style,
        })
        .collect();

    Ok(LocalFontAnalysis {
        chars_per_font: analysis.chars_per_font,
        font_faces,
    })
}

/// Process a single static file and return its cache-busted output
/// For fonts, this triggers global font analysis for subsetting
/// For CSS files, URLs are rewritten to cache-busted versions
#[picante::tracked]
pub async fn static_file_output<DB: Db>(
    db: &DB,
    file: StaticFile,
) -> PicanteResult<StaticFileOutput> {
    use crate::cache_bust::{cache_busted_path, content_hash};

    let path = file.path(db)?.as_str().to_string();
    tracing::debug!(
        file_path = %path,
        "🔵 QUERY: static_file_output COMPUTING (picante cache miss)"
    );
    // Get processed content based on file type
    let content = if is_font_file(&path) {
        // Font file - need to subset based on char analysis
        tracing::debug!(
            font_path = %path,
            "🟢 static_file_output: processing FONT file"
        );
        let analysis = font_char_analysis(db).await?;
        if let Some(chars) = find_chars_for_font_file(&path, &analysis) {
            if !chars.is_empty() {
                let mut sorted_chars: Vec<char> = chars.into_iter().collect();
                sorted_chars.sort();
                let char_set = CharSet::new(db, sorted_chars)?;

                if let Some(subsetted) = subset_font(db, file, char_set).await? {
                    subsetted
                } else {
                    load_static(db, file).await?
                }
            } else {
                load_static(db, file).await?
            }
        } else {
            load_static(db, file).await?
        }
    } else if path.to_lowercase().ends_with(".svg") {
        // SVG - process
        optimize_svg(db, file).await?
    } else if path.to_lowercase().ends_with(".css") {
        // CSS file - rewrite URLs to cache-busted versions
        let raw_content = load_static(db, file).await?;
        let css_str = String::from_utf8_lossy(&raw_content);

        // Build path map for non-CSS, non-image static files
        let mut path_map: HashMap<String, String> = HashMap::new();
        let static_files = StaticRegistry::files(db)?.unwrap_or_default();
        for other_file in static_files.iter() {
            let other_path = other_file.path(db)?.as_str().to_string();
            // Skip CSS files (would cause recursion) and images (different handling)
            if !other_path.to_lowercase().ends_with(".css")
                && !InputFormat::is_processable(&other_path)
            {
                // Recursively get the output for this file (safe - no CSS files)
                let other_output = static_file_output(db, *other_file).await?;
                path_map.insert(
                    format!("/{other_path}"),
                    format!("/{}", other_output.cache_busted_path),
                );
            }
        }

        // Rewrite URLs in CSS
        let rewritten = rewrite_urls_in_css(&css_str, &path_map).await;
        rewritten.into_bytes()
    } else {
        // Other static files - just load
        load_static(db, file).await?
    };

    // Hash and create cache-busted path
    let hash = content_hash(&content);
    let cache_busted = cache_busted_path(&path, &hash);

    Ok(StaticFileOutput {
        cache_busted_path: cache_busted,
        content,
    })
}

/// Compile CSS and return cache-busted output with rewritten URLs
#[picante::tracked]
pub async fn css_output<DB: Db>(db: &DB) -> PicanteResult<Option<CssOutput>> {
    use crate::cache_bust::{cache_busted_path, content_hash};
    use crate::url_rewrite::rewrite_urls_in_css;

    let Some(css_content) = compile_sass(db).await? else {
        return Ok(None);
    };

    // Build static path map for URL rewriting
    let mut static_path_map: HashMap<String, String> = HashMap::new();
    let static_files = StaticRegistry::files(db)?.unwrap_or_default();
    for file in static_files.iter() {
        let original_path = file.path(db)?.as_str().to_string();
        // Skip images - they get transcoded to different formats
        if !InputFormat::is_processable(&original_path) {
            let output = static_file_output(db, *file).await?;
            static_path_map.insert(
                format!("/{original_path}"),
                format!("/{}", output.cache_busted_path),
            );
        }
    }

    // Rewrite URLs in CSS
    let rewritten_css = rewrite_urls_in_css(&css_content.0, &static_path_map).await;

    // Hash and create cache-busted path
    let hash = content_hash(rewritten_css.as_bytes());
    let cache_busted = cache_busted_path("main.css", &hash);

    Ok(Some(CssOutput {
        cache_busted_path: cache_busted,
        content: rewritten_css,
    }))
}

/// Serve a single page or section with full URL rewriting and minification
/// This is the main entry point for lazy page serving
#[picante::tracked]
#[tracing::instrument(skip(db), name = "serve_html")]
pub async fn serve_html<DB: Db>(
    db: &DB,
    route: Route,
) -> PicanteResult<Result<Option<String>, BuildError>> {
    use crate::url_rewrite::{
        ResponsiveImageInfo, rewrite_urls_in_html, transform_images_to_picture,
    };

    let site_tree = match build_tree(db).await? {
        Ok(tree) => tree,
        Err(errors) => return Ok(Err(BuildError { errors })),
    };

    // Check if route exists in site tree
    let route_exists =
        site_tree.sections.contains_key(&route) || site_tree.pages.contains_key(&route);
    if !route_exists {
        return Ok(Ok(None));
    }

    // Get the raw HTML for this route
    let all_html = match all_rendered_html(db).await? {
        Ok(html) => html,
        Err(e) => return Ok(Err(e)),
    };
    let Some(raw_html) = all_html.pages.get(&route).cloned() else {
        return Ok(Ok(None));
    };

    // Build the full URL rewrite map
    let mut path_map: HashMap<String, String> = HashMap::new();

    // Add CSS path
    if let Some(css) = css_output(db).await? {
        path_map.insert(
            "/main.css".to_string(),
            format!("/{}", css.cache_busted_path),
        );
    }

    // Add static file paths (non-images)
    let static_files = StaticRegistry::files(db)?.unwrap_or_default();
    for file in static_files.iter() {
        let original_path = file.path(db)?.as_str().to_string();
        if !InputFormat::is_processable(&original_path) {
            let output = static_file_output(db, *file).await?;
            path_map.insert(
                format!("/{original_path}"),
                format!("/{}", output.cache_busted_path),
            );
        }
    }

    // Build image variants map for <picture> transformation
    // Uses image_metadata (fast decode) + input-based hashes (no encoding needed)
    let mut image_variants: HashMap<String, ResponsiveImageInfo> = HashMap::new();
    for file in static_files.iter() {
        let path = file.path(db)?.as_str().to_string();
        if InputFormat::is_processable(&path) {
            if let Some(metadata) = image_metadata(db, *file).await? {
                use crate::cas::ImageVariantKey;

                let input_hash = image_input_hash(db, *file).await?;
                let mut jxl_srcset = Vec::new();
                let mut webp_srcset = Vec::new();

                // Build JXL srcset using input-based hashes
                for &width in &metadata.variant_widths {
                    let base_path =
                        image::change_extension(&path, image::OutputFormat::Jxl.extension());
                    let variant_path = if width == metadata.width {
                        base_path
                    } else {
                        add_width_suffix(&base_path, width)
                    };
                    let key = ImageVariantKey {
                        input_hash,
                        format: image::OutputFormat::Jxl,
                        width,
                    };
                    let cache_busted = format!(
                        "{}.{}",
                        variant_path.trim_end_matches(".jxl"),
                        key.url_hash()
                    ) + ".jxl";
                    jxl_srcset.push((format!("/{cache_busted}"), width));
                }

                // Build WebP srcset using input-based hashes
                for &width in &metadata.variant_widths {
                    let base_path =
                        image::change_extension(&path, image::OutputFormat::WebP.extension());
                    let variant_path = if width == metadata.width {
                        base_path
                    } else {
                        add_width_suffix(&base_path, width)
                    };
                    let key = ImageVariantKey {
                        input_hash,
                        format: image::OutputFormat::WebP,
                        width,
                    };
                    let cache_busted = format!(
                        "{}.{}",
                        variant_path.trim_end_matches(".webp"),
                        key.url_hash()
                    ) + ".webp";
                    webp_srcset.push((format!("/{cache_busted}"), width));
                }

                image_variants.insert(
                    format!("/{path}"),
                    ResponsiveImageInfo {
                        jxl_srcset: jxl_srcset.clone(),
                        webp_srcset: webp_srcset.clone(),
                        original_width: metadata.width,
                        original_height: metadata.height,
                        thumbhash_data_url: metadata.thumbhash_data_url.clone(),
                    },
                );

                // Also add to path_map for non-<img> contexts (like <link rel="icon">)
                // Map original path to full-size WebP variant
                if let Some((webp_url, _)) = webp_srcset.last() {
                    path_map.insert(format!("/{path}"), webp_url.clone());
                }
            }
        }
    }

    // Rewrite URLs in HTML
    let rewritten_html = rewrite_urls_in_html(&raw_html, &path_map).await;

    // Transform <img> to <picture> for responsive images
    let transformed_html = transform_images_to_picture(&rewritten_html, &image_variants);

    // Minify HTML (but skip for error pages to preserve the error marker comment)
    let final_html = if raw_html.contains(crate::render::RENDER_ERROR_MARKER) {
        transformed_html
    } else {
        crate::svg::minify_html(&transformed_html).await
    };

    Ok(Ok(Some(final_html)))
}

/// Check if a path is a font file
fn is_font_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".ttf")
        || lower.ends_with(".otf")
        || lower.ends_with(".woff")
        || lower.ends_with(".woff2")
}

/// Find the character set needed for a font file based on @font-face analysis
fn find_chars_for_font_file(
    path: &str,
    analysis: &LocalFontAnalysis,
) -> Option<std::collections::HashSet<char>> {
    // Normalize path: remove leading slash and 'static/' prefix
    // File paths are like "static/fonts/Foo.woff2"
    // CSS URLs are like "/fonts/Foo.woff2"
    let normalized = path.trim_start_matches('/').trim_start_matches("static/");

    // Find @font-face rules that reference this font file
    for face in &analysis.font_faces {
        let face_src = face.src.trim_start_matches('/');
        if face_src == normalized {
            // Found a match - return chars for this font-family
            // Convert Vec<char> to HashSet<char>
            return analysis
                .chars_per_font
                .get(&face.family)
                .map(|chars| chars.iter().copied().collect());
        }
    }

    None
}

// ============================================================================
// Code execution integration
// ============================================================================

/// Execute code samples from all source files and return results
/// This is called during the build process to validate code samples
pub async fn execute_all_code_samples<DB: Db>(db: &DB) -> PicanteResult<Vec<CodeExecutionResult>> {
    use crate::cells::{execute_code_samples_cell, extract_code_samples_cell};

    let mut all_results = Vec::new();

    // Create default configuration for code execution
    let config = cell_code_execution_proto::CodeExecutionConfig::default();

    // Extract and execute code samples from all source files
    let sources = SourceRegistry::sources(db)?.unwrap_or_default();
    for source in sources.iter() {
        let content = source.content(db)?;
        let source_path = source.path(db)?.as_str().to_string();

        // Extract code samples from this source file
        if let Some(samples) = extract_code_samples_cell(content.as_str(), &source_path).await
            && !samples.is_empty()
        {
            tracing::debug!("Found {} code samples in {}", samples.len(), source_path);

            // Execute the code samples
            if let Some(execution_results) =
                execute_code_samples_cell(samples, config.clone()).await
            {
                // Convert cell results to our internal format
                for (sample, result) in execution_results {
                    // Convert metadata if present
                    let metadata = result.metadata.map(|m| CodeExecutionMetadata {
                        rustc_version: m.rustc_version,
                        cargo_version: m.cargo_version,
                        target: m.target,
                        timestamp: m.timestamp,
                        cache_hit: m.cache_hit,
                        platform: m.platform,
                        arch: m.arch,
                        dependencies: m
                            .dependencies
                            .into_iter()
                            .map(|d| ResolvedDependencyInfo {
                                name: d.name,
                                version: d.version,
                                source: convert_dependency_source(d.source),
                            })
                            .collect(),
                    });

                    let code_result = CodeExecutionResult {
                        source_path: sample.source_path,
                        line: sample.line as u32,
                        language: sample.language,
                        code: sample.code,
                        status: match result.status {
                            cell_code_execution_proto::ExecutionStatus::Success => {
                                crate::db::CodeExecutionStatus::Success
                            }
                            cell_code_execution_proto::ExecutionStatus::Failed => {
                                crate::db::CodeExecutionStatus::Failed
                            }
                            cell_code_execution_proto::ExecutionStatus::Skipped => {
                                crate::db::CodeExecutionStatus::Skipped
                            }
                        },
                        exit_code: result.exit_code,
                        stdout: result.stdout,
                        stderr: result.stderr,
                        duration_ms: result.duration_ms,
                        error: result.error,
                        metadata,
                    };
                    all_results.push(code_result);
                }
            }
        }
    }

    if !all_results.is_empty() {
        let success_count = all_results
            .iter()
            .filter(|r| r.status == crate::db::CodeExecutionStatus::Success)
            .count();
        let failed_count = all_results
            .iter()
            .filter(|r| r.status == crate::db::CodeExecutionStatus::Failed)
            .count();
        let skipped_count = all_results
            .iter()
            .filter(|r| r.status == crate::db::CodeExecutionStatus::Skipped)
            .count();
        tracing::info!(
            "Code execution results: {} successful, {} failed, {} skipped",
            success_count,
            failed_count,
            skipped_count
        );

        // Log failures for visibility
        for result in &all_results {
            if result.status == crate::db::CodeExecutionStatus::Failed {
                tracing::warn!(
                    "Code execution failed in {}:{} ({}): {}",
                    result.source_path,
                    result.line,
                    result.language,
                    result.error.as_deref().unwrap_or("Unknown error")
                );
            }
        }
    }

    Ok(all_results)
}

/// Convert cell DependencySource to db DependencySourceInfo
fn convert_dependency_source(
    source: cell_code_execution_proto::DependencySource,
) -> DependencySourceInfo {
    use cell_code_execution_proto::DependencySource;
    match source {
        DependencySource::CratesIo => DependencySourceInfo::CratesIo,
        DependencySource::Git { url, commit } => DependencySourceInfo::Git { url, commit },
        DependencySource::Path { path } => DependencySourceInfo::Path { path },
    }
}

// Tests for split_frontmatter and resolve_internal_link moved to mod-markdown
