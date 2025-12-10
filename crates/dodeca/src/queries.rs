use crate::db::{
    AllRenderedHtml, CharSet, CodeExecutionMetadata, CodeExecutionResult, CssOutput, DataRegistry,
    Db, DependencySourceInfo, Heading, ImageVariant, OutputFile, Page, ParsedData, ProcessedImages,
    RenderedHtml, ResolvedDependencyInfo, SassFile, SassRegistry, Section, SiteOutput, SiteTree,
    SourceFile, SourceRegistry, StaticFile, StaticFileOutput, StaticRegistry, TemplateFile,
    TemplateRegistry,
};

use crate::image::{self, InputFormat, OutputFormat, add_width_suffix};
use crate::types::{HtmlBody, Route, SassContent, StaticPath, TemplateContent, Title};
use crate::url_rewrite::rewrite_urls_in_css;
use facet::Facet;
use facet_value::Value;
use pulldown_cmark::{Options, Parser, html};
use std::collections::{BTreeMap, HashMap};

/// Load a template file's content - tracked by Salsa for dependency tracking
#[salsa::tracked]
pub fn load_template(db: &dyn Db, template: TemplateFile) -> TemplateContent {
    let content = template.content(db).clone();
    let content_hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.as_str().hash(&mut hasher);
        hasher.finish()
    };
    tracing::debug!(
        "🔄 load_template: {} (content hash: {:016x})",
        template.path(db).as_str(),
        content_hash
    );
    content
}

/// Load all templates and return a map of path -> content
/// This tracked query records dependencies on all template files
#[salsa::tracked]
pub fn load_all_templates<'db>(
    db: &'db dyn Db,
    registry: TemplateRegistry,
) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for template in registry.templates(db) {
        let path = template.path(db).as_str().to_string();
        let content = load_template(db, *template);
        result.insert(path, content.as_str().to_string());
    }
    result
}

/// Build a lookup table from template path to TemplateFile
#[salsa::tracked]
pub fn build_template_lookup<'db>(
    db: &'db dyn Db,
    registry: TemplateRegistry,
) -> HashMap<String, TemplateFile> {
    let mut lookup = HashMap::new();
    for template in registry.templates(db) {
        let path = template.path(db).as_str().to_string();
        lookup.insert(path, *template);
    }
    lookup
}

/// A template loader that uses Salsa tracked queries for fine-grained dependency tracking.
/// When a template is loaded, Salsa records it as a dependency of the current query,
/// enabling incremental rebuilds that only re-render pages whose templates changed.
pub struct SalsaTemplateLoader<'db> {
    db: &'db dyn Db,
    lookup: HashMap<String, TemplateFile>,
}

impl<'db> SalsaTemplateLoader<'db> {
    /// Create a new loader for the given template registry
    pub fn new(db: &'db dyn Db, registry: TemplateRegistry) -> Self {
        Self {
            db,
            lookup: build_template_lookup(db, registry),
        }
    }
}

impl gingembre::TemplateLoader for SalsaTemplateLoader<'_> {
    fn load(&self, name: &str) -> Option<String> {
        let template = self.lookup.get(name)?;
        // This call to load_template is tracked by Salsa!
        // The current query (e.g., render_page) will depend on this specific template.
        let content = load_template(self.db, *template);
        Some(content.as_str().to_string())
    }
}

/// Load a sass file's content - tracked by Salsa for dependency tracking
#[salsa::tracked]
pub fn load_sass(db: &dyn Db, sass: SassFile) -> SassContent {
    sass.content(db).clone()
}

/// Load all sass files and return a map of path -> content
/// This tracked query records dependencies on all sass files
#[salsa::tracked]
pub fn load_all_sass<'db>(db: &'db dyn Db, registry: SassRegistry) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for sass in registry.files(db) {
        let path = sass.path(db).as_str().to_string();
        let content = load_sass(db, *sass);
        result.insert(path, content.as_str().to_string());
    }
    result
}

/// Load all data files and return their raw content
/// This tracked query records dependencies on all data files
/// The conversion to template Value happens at render time
#[salsa::tracked]
pub fn load_all_data_raw<'db>(db: &'db dyn Db, registry: DataRegistry) -> Vec<(String, String)> {
    registry
        .files(db)
        .iter()
        .map(|file| {
            (
                file.path(db).as_str().to_string(),
                file.content(db).as_str().to_string(),
            )
        })
        .collect()
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
#[salsa::interned]
pub struct DataValuePath<'db> {
    #[returns(ref)]
    pub segments: Vec<String>,
}

/// Build a lookup table from data key (filename without extension) to DataFile.
/// This is tracked so changes to the registry invalidate the lookup.
#[salsa::tracked]
pub fn data_file_lookup<'db>(db: &'db dyn Db, registry: DataRegistry) -> HashMap<String, DataFile> {
    registry
        .files(db)
        .iter()
        .map(|f| {
            let path = f.path(db).as_str();
            let key = extract_filename_without_extension(path);
            (key, *f)
        })
        .collect()
}

/// Get all data file keys (filenames without extension).
/// Used for iteration over `data`.
#[salsa::tracked]
pub fn data_file_keys<'db>(db: &'db dyn Db, registry: DataRegistry) -> Vec<String> {
    registry
        .files(db)
        .iter()
        .map(|f| extract_filename_without_extension(f.path(db).as_str()))
        .collect()
}

/// Load and parse a single data file.
/// Each file load is individually tracked by Salsa.
#[salsa::tracked]
pub fn load_and_parse_data_file(db: &dyn Db, file: DataFile) -> Option<Value> {
    let path = file.path(db).as_str();
    let content = file.content(db).as_str();

    let format = DataFormat::from_extension(path)?;
    parse_data_file(content, format).ok()
}

/// Resolve a value at a specific path through the data tree.
///
/// THIS IS THE KEY QUERY - each unique path is tracked separately by Salsa!
/// When a path is resolved, Salsa records it as a dependency of the current query.
#[salsa::tracked]
pub fn resolve_data_value<'db>(
    db: &'db dyn Db,
    registry: DataRegistry,
    path: DataValuePath<'db>,
) -> Option<Value> {
    let segments = path.segments(db);

    if segments.is_empty() {
        // Root path - can't return a single value, caller should use keys
        return None;
    }

    // First segment is the file key (filename without extension)
    let file_key = &segments[0];
    let lookup = data_file_lookup(db, registry);
    let file = lookup.get(file_key)?;

    // Load and parse the file (this is tracked!)
    let parsed = load_and_parse_data_file(db, *file)?;

    // Navigate to the specific path within the parsed value
    let mut current = parsed;
    for segment in segments.iter().skip(1) {
        current = match current.destructure_ref() {
            DestructuredRef::Object(obj) => obj.get(segment.as_str())?.clone(),
            DestructuredRef::Array(arr) => {
                let idx: usize = segment.parse().ok()?;
                arr.get(idx)?.clone()
            }
            _ => return None,
        };
    }

    Some(current)
}

/// Get child keys at a path (for iteration).
/// Returns the keys at that path if it's an object, or indices if it's an array.
#[salsa::tracked]
pub fn data_keys_at_path<'db>(
    db: &'db dyn Db,
    registry: DataRegistry,
    path: DataValuePath<'db>,
) -> Vec<String> {
    let segments = path.segments(db);

    if segments.is_empty() {
        // Root path - return all data file keys
        return data_file_keys(db, registry);
    }

    // First, resolve the value at this path
    let value = match resolve_data_value(db, registry, path) {
        Some(v) => v,
        None => return Vec::new(),
    };

    // Return keys based on the value type
    match value.destructure_ref() {
        DestructuredRef::Object(obj) => obj.keys().map(|k| k.to_string()).collect(),
        DestructuredRef::Array(arr) => (0..arr.len()).map(|i| i.to_string()).collect(),
        _ => Vec::new(),
    }
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

/// A data resolver backed by Salsa queries.
///
/// Each path resolution becomes a tracked dependency, enabling fine-grained
/// incremental recomputation. Change `versions.toml`? Only pages that access
/// `data.versions.*` are re-rendered.
///
/// # Safety
///
/// This struct uses a raw pointer to the database to work around lifetime
/// constraints with `Arc<dyn DataResolver>`. The resolver MUST NOT outlive
/// the database reference it was created from.
///
/// This is safe within Salsa tracked queries because:
/// - The resolver is created at the start of the query
/// - The resolver is only used during the query's render call
/// - The resolver is dropped before the query returns
/// - The database reference is valid for the entire query duration
pub struct SalsaDataResolver {
    // Raw pointer to avoid lifetime issues with Arc<dyn DataResolver>
    // SAFETY: Must not outlive the database
    db: *const dyn Db,
    registry: DataRegistry,
}

// SAFETY: The Salsa database is Send + Sync, and we ensure the resolver
// doesn't outlive the database by only using it within tracked queries.
unsafe impl Send for SalsaDataResolver {}
unsafe impl Sync for SalsaDataResolver {}

impl SalsaDataResolver {
    /// Create a new data resolver for the given data registry.
    ///
    /// # Safety
    ///
    /// The returned resolver must not outlive `db`. This is typically ensured
    /// by only using the resolver within the same Salsa tracked query where
    /// the database reference is valid.
    pub fn new(db: &dyn Db, registry: DataRegistry) -> Self {
        Self {
            db: db as *const dyn Db,
            registry,
        }
    }

    /// Create an Arc-wrapped resolver (for use with gingembre Context).
    ///
    /// # Safety
    ///
    /// The Arc MUST be dropped before the database reference becomes invalid.
    /// This is safe within Salsa tracked queries when the Arc is only used
    /// for a single render call.
    pub fn new_arc(db: &dyn Db, registry: DataRegistry) -> Arc<dyn DataResolver> {
        Arc::new(Self::new(db, registry))
    }

    fn db(&self) -> &dyn Db {
        // SAFETY: Caller ensures the resolver doesn't outlive the database
        unsafe { &*self.db }
    }

    fn intern_path(&self, path: &DataPath) -> DataValuePath<'_> {
        DataValuePath::new(self.db(), path.segments().to_vec())
    }
}

impl DataResolver for SalsaDataResolver {
    fn resolve(&self, path: &DataPath) -> Option<Value> {
        let path_id = self.intern_path(path);
        resolve_data_value(self.db(), self.registry, path_id)
    }

    fn keys_at(&self, path: &DataPath) -> Option<Vec<String>> {
        let path_id = self.intern_path(path);
        let keys = data_keys_at_path(self.db(), self.registry, path_id);
        if keys.is_empty() && !path.segments().is_empty() {
            // Empty keys for a non-root path means the path doesn't exist or isn't iterable
            None
        } else {
            Some(keys)
        }
    }

    fn len_at(&self, path: &DataPath) -> Option<usize> {
        self.keys_at(path).map(|k| k.len())
    }
}

/// Compiled CSS output
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompiledCss(pub String);

/// Compile SASS to CSS - tracked by Salsa for dependency tracking
/// Returns None if compilation fails
#[salsa::tracked]
#[tracing::instrument(skip_all, name = "compile_sass")]
pub fn compile_sass<'db>(db: &'db dyn Db, registry: SassRegistry) -> Option<CompiledCss> {
    // Load all sass files - creates dependency on each
    let sass_map = load_all_sass(db, registry);

    // Skip compilation if no main.scss entry point exists
    if !sass_map.contains_key("main.scss") {
        if !sass_map.is_empty() {
            tracing::debug!("SCSS files found but no main.scss entry point, skipping compilation");
        }
        return None;
    }

    // Compile via plugin
    match crate::plugins::compile_sass_plugin(&sass_map) {
        Ok(css) => Some(CompiledCss(css)),
        Err(e) => {
            tracing::error!("SASS compilation failed: {}", e);
            None
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

/// Parse a source file into ParsedData
/// This is the main tracked function - Salsa memoizes the result
#[salsa::tracked]
pub fn parse_file(db: &dyn Db, source: SourceFile) -> ParsedData {
    let content = source.content(db);
    let path = source.path(db);
    let last_modified = source.last_modified(db);

    // Split frontmatter and body
    let (frontmatter_str, markdown) = split_frontmatter(content.as_str());

    // Parse frontmatter as Value first, then convert to Frontmatter
    // This allows unknown fields to be silently ignored
    let frontmatter: Frontmatter = if frontmatter_str.is_empty() {
        Frontmatter::default()
    } else {
        match facet_toml::from_str::<Value>(&frontmatter_str) {
            Ok(value) => facet_value::from_value(value).unwrap_or_default(),
            Err(e) => {
                eprintln!("Failed to parse frontmatter for {path:?}: {e:?}");
                eprintln!("Frontmatter was:\n{frontmatter_str}");
                Frontmatter::default()
            }
        }
    };

    // Convert markdown to HTML and extract headings
    let (html, headings) = render_markdown(&markdown);
    let body_html = HtmlBody::new(html);

    // Determine if this is a section (_index.md)
    let is_section = path.is_section_index();

    // Compute URL route
    let route = path.to_route();

    ParsedData {
        source_path: path.clone(),
        route,
        title: Title::new(frontmatter.title),
        description: frontmatter.description,
        weight: frontmatter.weight,
        body_html,
        is_section,
        headings,
        last_updated: last_modified,
        extra: frontmatter.extra,
    }
}

/// Build the site tree from all source files
/// This tracked query depends on all parse_file results
#[salsa::tracked]
pub fn build_tree<'db>(db: &'db dyn Db, sources: SourceRegistry) -> SiteTree {
    let mut sections: BTreeMap<Route, Section> = BTreeMap::new();
    let mut pages: BTreeMap<Route, Page> = BTreeMap::new();

    // Parse all files - this creates dependencies on each parse_file
    let parsed: Vec<ParsedData> = sources
        .sources(db)
        .iter()
        .map(|source| parse_file(db, *source))
        .collect();

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
                last_updated: data.last_updated,
                extra: data.extra.clone(),
            },
        );
    }

    SiteTree { sections, pages }
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
#[salsa::tracked]
#[tracing::instrument(skip_all, name = "render_page")]
pub fn render_page<'db>(
    db: &'db dyn Db,
    route: Route,
    sources: SourceRegistry,
    templates: TemplateRegistry,
    data: DataRegistry,
) -> RenderedHtml {
    use crate::render::render_page_with_resolver;

    // Build tree (cached by Salsa)
    let site_tree = build_tree(db, sources);

    // Create a lazy template loader that tracks dependencies via Salsa
    let loader = SalsaTemplateLoader::new(db, templates);

    // Create a lazy data resolver - each data path access becomes a tracked dependency
    let resolver = SalsaDataResolver::new_arc(db, data);

    // Find the page
    let page = site_tree
        .pages
        .get(&route)
        .expect("Page not found for route");

    // Render to HTML - template and data loads are tracked as dependencies
    let html = render_page_with_resolver(page, &site_tree, loader, resolver);
    RenderedHtml(html)
}

/// Render a single section to HTML
/// This tracked query depends on the section content, templates actually used, data files, and site tree.
/// Template dependencies are tracked lazily - only templates loaded during rendering are recorded.
/// Data dependencies are also tracked lazily - only data paths actually accessed become dependencies.
#[salsa::tracked]
#[tracing::instrument(skip_all, name = "render_section")]
pub fn render_section<'db>(
    db: &'db dyn Db,
    route: Route,
    sources: SourceRegistry,
    templates: TemplateRegistry,
    data: DataRegistry,
) -> RenderedHtml {
    use crate::render::render_section_with_resolver;

    // Build tree (cached by Salsa)
    let site_tree = build_tree(db, sources);

    // Create a lazy template loader that tracks dependencies via Salsa
    let loader = SalsaTemplateLoader::new(db, templates);

    // Create a lazy data resolver - each data path access becomes a tracked dependency
    let resolver = SalsaDataResolver::new_arc(db, data);

    // Find the section
    let section = site_tree
        .sections
        .get(&route)
        .expect("Section not found for route");

    // Render to HTML - template and data loads are tracked as dependencies
    let html = render_section_with_resolver(section, &site_tree, loader, resolver);
    RenderedHtml(html)
}

/// Load a single static file's content - tracked by Salsa
#[salsa::tracked]
pub fn load_static(db: &dyn Db, file: StaticFile) -> Vec<u8> {
    let content = file.content(db).clone();
    tracing::debug!(path = %file.path(db).as_str(), size = content.len(), "load_static called");
    content
}

/// Process an SVG file - tracked by Salsa
/// Currently passes through unchanged (optimization disabled)
#[salsa::tracked]
pub fn optimize_svg(db: &dyn Db, file: StaticFile) -> Vec<u8> {
    let content = file.content(db);

    // Try to parse as UTF-8 string
    let Ok(svg_str) = std::str::from_utf8(content) else {
        return content.clone();
    };

    // Process SVG (currently passthrough)
    match crate::svg::optimize_svg(svg_str) {
        Some(optimized) => optimized.into_bytes(),
        None => content.clone(),
    }
}

/// Load all static files - returns map of path -> content
#[salsa::tracked]
pub fn load_all_static<'db>(db: &'db dyn Db, registry: StaticRegistry) -> HashMap<String, Vec<u8>> {
    let mut result = HashMap::new();
    for file in registry.files(db) {
        let path = file.path(db).as_str().to_string();
        let content = load_static(db, *file);
        result.insert(path, content);
    }
    result
}

/// Decompress a font file (WOFF2/WOFF1 -> TTF/OTF)
/// Results are cached in the CAS to avoid repeated decompression
#[salsa::tracked]
#[tracing::instrument(skip_all, name = "decompress_font")]
pub fn decompress_font(db: &dyn Db, font_file: StaticFile) -> Option<Vec<u8>> {
    use crate::cas::{
        font_content_hash, get_cached_decompressed_font, put_cached_decompressed_font,
    };
    use crate::plugins::decompress_font_plugin;

    let font_data = font_file.content(db);
    let content_hash = font_content_hash(font_data);

    // Check CAS cache first
    if let Some(cached) = get_cached_decompressed_font(&content_hash) {
        tracing::debug!(
            "Font decompression cache hit for {}",
            font_file.path(db).as_str()
        );
        return Some(cached);
    }

    // Decompress the font via plugin
    match decompress_font_plugin(font_data) {
        Some(decompressed) => {
            // Cache the result
            put_cached_decompressed_font(&content_hash, &decompressed);
            tracing::debug!(
                "Decompressed font {} ({} -> {} bytes)",
                font_file.path(db).as_str(),
                font_data.len(),
                decompressed.len()
            );
            Some(decompressed)
        }
        None => {
            tracing::warn!("Failed to decompress font {}", font_file.path(db).as_str());
            None
        }
    }
}

/// Subset a font file to only include specified characters
/// Returns WOFF2 compressed bytes, or None if subsetting fails
#[salsa::tracked]
#[tracing::instrument(skip_all, name = "subset_font")]
pub fn subset_font<'db>(
    db: &'db dyn Db,
    font_file: StaticFile,
    chars: CharSet<'db>,
) -> Option<Vec<u8>> {
    use crate::plugins::{compress_to_woff2_plugin, subset_font_plugin};

    // First, decompress the font (handles WOFF2/WOFF1 -> TTF)
    let decompressed = decompress_font(db, font_file)?;

    let char_vec: Vec<char> = chars.chars(db).to_vec();

    // Subset the decompressed TTF via plugin
    let subsetted = match subset_font_plugin(&decompressed, &char_vec) {
        Some(data) => data,
        None => {
            tracing::warn!("Failed to subset font {}", font_file.path(db).as_str());
            return None;
        }
    };

    // Compress back to WOFF2 via plugin
    match compress_to_woff2_plugin(&subsetted) {
        Some(woff2) => {
            tracing::debug!(
                "Subsetted font {} ({} chars, {} -> {} bytes)",
                font_file.path(db).as_str(),
                char_vec.len(),
                decompressed.len(),
                woff2.len()
            );
            Some(woff2)
        }
        None => {
            tracing::warn!(
                "Failed to compress font {} to WOFF2",
                font_file.path(db).as_str()
            );
            None
        }
    }
}

/// Get image metadata (dimensions, thumbhash, variant widths) without full processing
/// This is fast - only decodes the image, doesn't encode to JXL/WebP
#[salsa::tracked]
pub fn image_metadata(db: &dyn Db, image_file: StaticFile) -> Option<image::ImageMetadata> {
    let path = image_file.path(db);
    let input_format = InputFormat::from_extension(path.as_str())?;
    let data = image_file.content(db);
    image::get_image_metadata(data, input_format)
}

/// Get the input hash for an image file (for cache-busted URLs)
#[salsa::tracked]
pub fn image_input_hash(db: &dyn Db, image_file: StaticFile) -> crate::cas::InputHash {
    use crate::cas::content_hash_32;
    let data = image_file.content(db);
    content_hash_32(data)
}

/// Process an image file into responsive formats (JXL + WebP) with multiple widths
/// Returns None if the image cannot be processed or is not a supported format
///
/// Uses CAS (Content-Addressable Storage) to cache processed images across restarts.
/// The cache key is a 32-byte hash of the input image content.
#[salsa::tracked] // No persist - CAS handles caching, don't bloat Salsa DB with image bytes
#[tracing::instrument(skip_all, name = "process_image")]
pub fn process_image(db: &dyn Db, image_file: StaticFile) -> Option<ProcessedImages> {
    use crate::cas::{content_hash_32, get_cached_image, put_cached_image};

    let path = image_file.path(db);
    let input_format = InputFormat::from_extension(path.as_str())?;
    let data = image_file.content(db);

    // Compute content hash for cache lookup
    let content_hash = content_hash_32(data);

    // Check CAS cache first
    if let Some(cached) = get_cached_image(&content_hash) {
        tracing::debug!("Image cache hit for {}", path.as_str());
        return Some(cached);
    }

    tracing::debug!("Image cache miss for {}", path.as_str());

    let processed = image::process_image(data, input_format)?;

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

    Some(result)
}

/// Build the complete site - THE top-level query
/// This produces all output files that need to be written to disk.
/// Fonts are automatically subsetted, all assets are cache-busted.
///
/// This reuses the same queries as the serve pipeline (serve_html, css_output,
/// static_file_output) to ensure consistency between `ddc build` and `ddc serve`.
#[salsa::tracked]
pub fn build_site<'db>(
    db: &'db dyn Db,
    sources: SourceRegistry,
    templates: TemplateRegistry,
    sass: SassRegistry,
    static_files: StaticRegistry,
    data: DataRegistry,
) -> SiteOutput {
    let mut files = Vec::new();

    // Build the site tree to get all routes
    let site_tree = build_tree(db, sources);

    // --- Phase 1: Render all HTML pages using serve_html ---
    // This reuses the exact same pipeline as `ddc serve`, ensuring consistency
    for route in site_tree.sections.keys() {
        if let Some(html) = serve_html(
            db,
            route.clone(),
            sources,
            templates,
            sass,
            static_files,
            data,
        ) {
            files.push(OutputFile::Html {
                route: route.clone(),
                content: html,
            });
        }
    }

    for route in site_tree.pages.keys() {
        if let Some(html) = serve_html(
            db,
            route.clone(),
            sources,
            templates,
            sass,
            static_files,
            data,
        ) {
            files.push(OutputFile::Html {
                route: route.clone(),
                content: html,
            });
        }
    }

    // --- Phase 2: Add CSS output ---
    if let Some(css) = css_output(db, sources, templates, sass, static_files, data) {
        files.push(OutputFile::Css {
            path: StaticPath::new(css.cache_busted_path),
            content: css.content,
        });
    }

    // --- Phase 3: Process static files ---
    for file in static_files.files(db) {
        let path = file.path(db).as_str();

        // Check if this is a processable image (PNG, JPG, GIF, WebP, JXL)
        if InputFormat::is_processable(path) {
            // Process the image into JXL and WebP variants at multiple widths
            if let Some(processed) = process_image(db, *file) {
                use crate::cas::ImageVariantKey;

                let input_hash = image_input_hash(db, *file);

                // Output each JXL variant
                for variant in &processed.jxl_variants {
                    let base_path = image::change_extension(path, OutputFormat::Jxl.extension());
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
                    let base_path = image::change_extension(path, OutputFormat::WebP.extension());
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
        let output = static_file_output(db, *file, sources, templates, sass, static_files, data);
        files.push(OutputFile::Static {
            path: StaticPath::new(output.cache_busted_path),
            content: output.content,
        });
    }

    // --- Phase 4: Execute code samples for validation ---
    let code_execution_results = execute_all_code_samples(db, sources);

    SiteOutput {
        files,
        code_execution_results,
    }
}

// ============================================================================
// Lazy serve queries - for on-demand page rendering
// ============================================================================

/// Render all pages and sections to HTML (without URL rewriting)
/// This is cached globally and used for font character analysis
#[salsa::tracked]
pub fn all_rendered_html<'db>(
    db: &'db dyn Db,
    sources: SourceRegistry,
    templates: TemplateRegistry,
    data: DataRegistry,
) -> AllRenderedHtml {
    tracing::info!("🔄 all_rendered_html: EXECUTING (not cached)");
    let site_tree = build_tree(db, sources);
    let template_map = load_all_templates(db, templates);

    // Load data files and convert to template Value
    let raw_data = load_all_data_raw(db, data);
    let data_value = crate::data::parse_raw_data_files(&raw_data);

    let mut pages = HashMap::new();

    for (route, section) in &site_tree.sections {
        let html = crate::render::render_section_to_html(
            section,
            &site_tree,
            &template_map,
            Some(data_value.clone()),
        );
        pages.insert(route.clone(), html);
    }

    for (route, page) in &site_tree.pages {
        let html = crate::render::render_page_to_html(
            page,
            &site_tree,
            &template_map,
            Some(data_value.clone()),
        );
        pages.insert(route.clone(), html);
    }

    AllRenderedHtml { pages }
}

/// Analyze all rendered HTML for font character usage
/// Returns the FontAnalysis needed for font subsetting
#[salsa::tracked]
pub fn font_char_analysis<'db>(
    db: &'db dyn Db,
    sources: SourceRegistry,
    templates: TemplateRegistry,
    sass: SassRegistry,
    static_files: StaticRegistry,
    data: DataRegistry,
) -> crate::plugins::FontAnalysis {
    let all_html = all_rendered_html(db, sources, templates, data);
    let sass_css = compile_sass(db, sass);
    let sass_str = sass_css.as_ref().map(|c| c.0.as_str()).unwrap_or("");

    // Collect CSS from static files
    let mut static_css_parts = Vec::new();
    for file in static_files.files(db) {
        let path = file.path(db).as_str();
        if path.to_lowercase().ends_with(".css") {
            let content = load_static(db, *file);
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
    let inline_css = crate::plugins::extract_css_from_html_plugin(&combined_html);
    let all_css = format!("{sass_str}\n{static_css}\n{inline_css}");

    crate::plugins::analyze_fonts_plugin(&combined_html, &all_css)
}

/// Process a single static file and return its cache-busted output
/// For fonts, this triggers global font analysis for subsetting
/// For CSS files, URLs are rewritten to cache-busted versions
#[salsa::tracked]
pub fn static_file_output<'db>(
    db: &'db dyn Db,
    file: StaticFile,
    sources: SourceRegistry,
    templates: TemplateRegistry,
    sass: SassRegistry,
    static_files: StaticRegistry,
    data: DataRegistry,
) -> StaticFileOutput {
    use crate::cache_bust::{cache_busted_path, content_hash};

    let path = file.path(db).as_str();
    // Get processed content based on file type
    let content = if is_font_file(path) {
        // Font file - need to subset based on char analysis
        let analysis = font_char_analysis(db, sources, templates, sass, static_files, data);
        if let Some(chars) = find_chars_for_font_file(path, &analysis) {
            if !chars.is_empty() {
                let mut sorted_chars: Vec<char> = chars.into_iter().collect();
                sorted_chars.sort();
                let char_set = CharSet::new(db, sorted_chars);

                if let Some(subsetted) = subset_font(db, file, char_set) {
                    subsetted
                } else {
                    load_static(db, file)
                }
            } else {
                load_static(db, file)
            }
        } else {
            load_static(db, file)
        }
    } else if path.to_lowercase().ends_with(".svg") {
        // SVG - process
        optimize_svg(db, file)
    } else if path.to_lowercase().ends_with(".css") {
        // CSS file - rewrite URLs to cache-busted versions
        let raw_content = load_static(db, file);
        let css_str = String::from_utf8_lossy(&raw_content);

        // Build path map for non-CSS, non-image static files
        let mut path_map: HashMap<String, String> = HashMap::new();
        for other_file in static_files.files(db) {
            let other_path = other_file.path(db).as_str();
            // Skip CSS files (would cause recursion) and images (different handling)
            if !other_path.to_lowercase().ends_with(".css")
                && !InputFormat::is_processable(other_path)
            {
                // Recursively get the output for this file (safe - no CSS files)
                let other_output = static_file_output(
                    db,
                    *other_file,
                    sources,
                    templates,
                    sass,
                    static_files,
                    data,
                );
                path_map.insert(
                    format!("/{other_path}"),
                    format!("/{}", other_output.cache_busted_path),
                );
            }
        }

        // Rewrite URLs in CSS
        let rewritten = rewrite_urls_in_css(&css_str, &path_map);
        rewritten.into_bytes()
    } else {
        // Other static files - just load
        load_static(db, file)
    };

    // Hash and create cache-busted path
    let hash = content_hash(&content);
    let cache_busted = cache_busted_path(path, &hash);

    StaticFileOutput {
        cache_busted_path: cache_busted,
        content,
    }
}

/// Compile CSS and return cache-busted output with rewritten URLs
#[salsa::tracked]
pub fn css_output<'db>(
    db: &'db dyn Db,
    sources: SourceRegistry,
    templates: TemplateRegistry,
    sass: SassRegistry,
    static_files: StaticRegistry,
    data: DataRegistry,
) -> Option<CssOutput> {
    use crate::cache_bust::{cache_busted_path, content_hash};
    use crate::url_rewrite::rewrite_urls_in_css;

    let css_content = compile_sass(db, sass)?;

    // Build static path map for URL rewriting
    let mut static_path_map: HashMap<String, String> = HashMap::new();
    for file in static_files.files(db) {
        let original_path = file.path(db).as_str();
        // Skip images - they get transcoded to different formats
        if !InputFormat::is_processable(original_path) {
            let output =
                static_file_output(db, *file, sources, templates, sass, static_files, data);
            static_path_map.insert(
                format!("/{original_path}"),
                format!("/{}", output.cache_busted_path),
            );
        }
    }

    // Rewrite URLs in CSS
    let rewritten_css = rewrite_urls_in_css(&css_content.0, &static_path_map);

    // Hash and create cache-busted path
    let hash = content_hash(rewritten_css.as_bytes());
    let cache_busted = cache_busted_path("main.css", &hash);

    Some(CssOutput {
        cache_busted_path: cache_busted,
        content: rewritten_css,
    })
}

/// Serve a single page or section with full URL rewriting and minification
/// This is the main entry point for lazy page serving
#[salsa::tracked]
#[tracing::instrument(skip_all, name = "serve_html")]
pub fn serve_html<'db>(
    db: &'db dyn Db,
    route: Route,
    sources: SourceRegistry,
    templates: TemplateRegistry,
    sass: SassRegistry,
    static_files: StaticRegistry,
    data: DataRegistry,
) -> Option<String> {
    use crate::url_rewrite::{
        ResponsiveImageInfo, rewrite_urls_in_html, transform_images_to_picture,
    };

    let site_tree = build_tree(db, sources);

    // Check if route exists in site tree
    let route_exists =
        site_tree.sections.contains_key(&route) || site_tree.pages.contains_key(&route);
    if !route_exists {
        return None;
    }

    // Get the raw HTML for this route
    let all_html = all_rendered_html(db, sources, templates, data);
    let raw_html = all_html.pages.get(&route).cloned()?;

    // Build the full URL rewrite map
    let mut path_map: HashMap<String, String> = HashMap::new();

    // Add CSS path
    if let Some(css) = css_output(db, sources, templates, sass, static_files, data) {
        path_map.insert(
            "/main.css".to_string(),
            format!("/{}", css.cache_busted_path),
        );
    }

    // Add static file paths (non-images)
    for file in static_files.files(db) {
        let original_path = file.path(db).as_str();
        if !InputFormat::is_processable(original_path) {
            let output =
                static_file_output(db, *file, sources, templates, sass, static_files, data);
            path_map.insert(
                format!("/{original_path}"),
                format!("/{}", output.cache_busted_path),
            );
        }
    }

    // Build image variants map for <picture> transformation
    // Uses image_metadata (fast decode) + input-based hashes (no encoding needed)
    let mut image_variants: HashMap<String, ResponsiveImageInfo> = HashMap::new();
    for file in static_files.files(db) {
        let path = file.path(db).as_str();
        if InputFormat::is_processable(path) {
            if let Some(metadata) = image_metadata(db, *file) {
                use crate::cas::ImageVariantKey;

                let input_hash = image_input_hash(db, *file);
                let mut jxl_srcset = Vec::new();
                let mut webp_srcset = Vec::new();

                // Build JXL srcset using input-based hashes
                for &width in &metadata.variant_widths {
                    let base_path =
                        image::change_extension(path, image::OutputFormat::Jxl.extension());
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
                        image::change_extension(path, image::OutputFormat::WebP.extension());
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
                        jxl_srcset,
                        webp_srcset,
                        original_width: metadata.width,
                        original_height: metadata.height,
                        thumbhash_data_url: metadata.thumbhash_data_url.clone(),
                    },
                );
            }
        }
    }

    // Rewrite URLs in HTML
    let rewritten_html = rewrite_urls_in_html(&raw_html, &path_map);

    // Transform <img> to <picture> for responsive images
    let transformed_html = transform_images_to_picture(&rewritten_html, &image_variants);

    // Minify HTML (but skip for error pages to preserve the error marker comment)
    let final_html = if raw_html.contains(crate::render::RENDER_ERROR_MARKER) {
        transformed_html
    } else {
        crate::svg::minify_html(&transformed_html)
    };

    Some(final_html)
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
    analysis: &crate::plugins::FontAnalysis,
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

/// Split content into frontmatter and body
fn split_frontmatter(content: &str) -> (String, String) {
    let content = content.trim_start();

    // Check for +++ delimiters (TOML frontmatter)
    if let Some(rest) = content.strip_prefix("+++") {
        if let Some(end) = rest.find("+++") {
            let frontmatter = rest[..end].trim().to_string();
            let body = rest[end + 3..].trim_start().to_string();
            return (frontmatter, body);
        }
    }

    // No frontmatter found
    (String::new(), content.to_string())
}

/// Render markdown to HTML, resolving internal links and extracting headings
fn render_markdown(markdown: &str) -> (String, Vec<Heading>) {
    use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Tag};

    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_HEADING_ATTRIBUTES;

    let parser = Parser::new_ext(markdown, options);

    // Collect headings while processing
    let mut headings = Vec::new();
    let mut current_heading: Option<(u8, String, String)> = None; // (level, id, text)

    // Track code block state for syntax highlighting
    let mut in_code_block = false;
    let mut code_block_lang = String::new();
    let mut code_block_content = String::new();

    // Transform events to resolve @/ links, extract headings, and handle code blocks
    let mut output_events: Vec<Event> = Vec::new();

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(ref lang))) => {
                in_code_block = true;
                code_block_lang = lang.to_string();
                code_block_content.clear();
                // Don't emit start event - we'll emit raw HTML instead
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)) => {
                in_code_block = true;
                code_block_lang.clear();
                code_block_content.clear();
            }
            Event::End(pulldown_cmark::TagEnd::CodeBlock) => {
                if in_code_block {
                    // Apply syntax highlighting
                    let highlighted_html =
                        highlight_code_block(&code_block_content, &code_block_lang);
                    output_events.push(Event::Html(highlighted_html.into()));
                    in_code_block = false;
                    code_block_lang.clear();
                    code_block_content.clear();
                }
            }
            Event::Text(ref text) if in_code_block => {
                code_block_content.push_str(text);
            }
            Event::Start(Tag::Heading { level, ref id, .. }) => {
                let level_num = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                current_heading = Some((
                    level_num,
                    id.as_ref().map(|s| s.to_string()).unwrap_or_default(),
                    String::new(),
                ));
                output_events.push(event);
            }
            Event::End(pulldown_cmark::TagEnd::Heading(_)) => {
                if let Some((level, id, text)) = current_heading.take() {
                    // Generate ID from text if not provided
                    let id = if id.is_empty() { slugify(&text) } else { id };
                    headings.push(Heading {
                        title: text,
                        id,
                        level,
                    });
                }
                output_events.push(event);
            }
            Event::Text(ref text) | Event::Code(ref text) => {
                if let Some((_, _, ref mut heading_text)) = current_heading {
                    heading_text.push_str(text);
                }
                output_events.push(event);
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let resolved = resolve_internal_link(&dest_url);
                output_events.push(Event::Start(Tag::Link {
                    link_type,
                    dest_url: resolved.into(),
                    title,
                    id,
                }));
            }
            other => {
                output_events.push(other);
            }
        }
    }

    let mut html_output = String::new();
    html::push_html(&mut html_output, output_events.into_iter());

    // Also extract headings from any inline HTML in the output
    let html_headings = extract_html_headings(&html_output);

    // Merge: add HTML headings that aren't duplicates (by id)
    for h in html_headings {
        if !headings.iter().any(|existing| existing.id == h.id) {
            headings.push(h);
        }
    }

    // Inject id attributes into headings that don't have them
    let html_output = inject_heading_ids(&html_output, &headings);

    (html_output, headings)
}

/// Highlight a code block using the syntax highlighting plugin
fn highlight_code_block(code: &str, language: &str) -> String {
    use crate::plugins::highlight_code_rapace;

    // Try to use the syntax highlighting plugin
    if let Some(result) = highlight_code_rapace(code, language) {
        // Wrap in pre/code tags with language class
        let lang_class = if language.is_empty() {
            String::new()
        } else {
            format!(" class=\"language-{}\"", html_escape_attr(language))
        };
        format!("<pre><code{}>{}</code></pre>", lang_class, result.html)
    } else {
        // Fallback: escape HTML and wrap in pre/code
        let lang_class = if language.is_empty() {
            String::new()
        } else {
            format!(" class=\"language-{}\"", html_escape_attr(language))
        };
        format!(
            "<pre><code{}>{}</code></pre>",
            lang_class,
            html_escape_content(code)
        )
    }
}

/// Escape HTML attribute value
fn html_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape HTML content
fn html_escape_content(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Convert text to a URL-safe slug for heading IDs
fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Extract headings from HTML content (for inline HTML headings)
fn extract_html_headings(html: &str) -> Vec<Heading> {
    use regex::Regex;

    let mut headings = Vec::new();

    // Match <h1> through <h6> tags with optional id attribute
    // Pattern: <h[1-6](?:\s+id="([^"]*)")?>([^<]*)</h[1-6]>
    let re = Regex::new(r#"<h([1-6])(?:\s[^>]*?id="([^"]*)"[^>]*)?>([^<]*)</h[1-6]>"#).unwrap();

    for cap in re.captures_iter(html) {
        let level: u8 = cap[1].parse().unwrap_or(1);
        let id = cap
            .get(2)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        let title = cap[3].trim().to_string();

        if !title.is_empty() {
            let id = if id.is_empty() { slugify(&title) } else { id };
            headings.push(Heading { title, id, level });
        }
    }

    headings
}

/// Inject id attributes into HTML headings that don't have them
fn inject_heading_ids(html: &str, headings: &[Heading]) -> String {
    use regex::Regex;

    let strip_tags = Regex::new(r"<[^>]+>").unwrap();
    let mut result = html.to_string();

    // Process each heading level separately (h1 through h6)
    for level in 1..=6 {
        let pattern = format!(r#"<h{level}(\s[^>]*)?>(.*?)</h{level}>"#);
        let re = Regex::new(&pattern).unwrap();

        result = re
            .replace_all(&result, |caps: &regex::Captures| {
                let attrs = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let content = &caps[2];

                // Skip if already has an id attribute
                if attrs.contains("id=") {
                    return caps[0].to_string();
                }

                // Strip HTML tags from content to get plain text
                let plain_text = strip_tags.replace_all(content, "").to_string();

                // Look up the ID from our headings list by matching the plain text
                let id = headings
                    .iter()
                    .find(|h| h.title == plain_text.trim())
                    .map(|h| h.id.clone())
                    .unwrap_or_else(|| slugify(plain_text.trim()));

                format!(r#"<h{level} id="{id}"{attrs}>{content}</h{level}>"#)
            })
            .to_string();
    }

    result
}

/// Resolve Zola-style @/ internal links to URL paths
fn resolve_internal_link(link: &str) -> String {
    if let Some(path) = link.strip_prefix("@/") {
        // Split off fragment (e.g., #anchor) before processing
        let (path_part, fragment) = match path.find('#') {
            Some(idx) => (&path[..idx], Some(&path[idx..])),
            None => (path, None),
        };

        // Convert @/learn/_index.md -> /learn
        // Convert @/learn/page.md -> /learn/page
        let mut path = path_part.to_string();

        // Remove .md extension
        if path.ends_with(".md") {
            path = path[..path.len() - 3].to_string();
        }

        // Handle _index -> parent directory
        if path.ends_with("/_index") {
            path = path[..path.len() - 7].to_string();
        } else if path == "_index" {
            path = String::new();
        }

        // Ensure leading slash, no trailing slash (except for root)
        let base = if path.is_empty() {
            "/".to_string()
        } else {
            format!("/{path}")
        };

        // Re-append fragment if present
        match fragment {
            Some(frag) => format!("{base}{frag}"),
            None => base,
        }
    } else {
        link.to_string()
    }
}

// ============================================================================
// Code execution integration
// ============================================================================

/// Execute code samples from all source files and return results
/// This is called during the build process to validate code samples
pub fn execute_all_code_samples(db: &dyn Db, sources: SourceRegistry) -> Vec<CodeExecutionResult> {
    use crate::plugins::{execute_code_samples_plugin, extract_code_samples_plugin};

    let mut all_results = Vec::new();

    // Create default configuration for code execution
    let config = dodeca_code_execution_types::CodeExecutionConfig::default();

    // Extract and execute code samples from all source files
    for source in sources.sources(db) {
        let content = source.content(db);
        let source_path = source.path(db).as_str();

        // Extract code samples from this source file
        if let Some(samples) = extract_code_samples_plugin(content.as_str(), source_path) {
            if !samples.is_empty() {
                tracing::info!("Found {} code samples in {}", samples.len(), source_path);

                // Execute the code samples
                if let Some(execution_results) =
                    execute_code_samples_plugin(samples, config.clone())
                {
                    // Convert plugin results to our internal format
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
                            success: result.success,
                            exit_code: result.exit_code,
                            stdout: result.stdout,
                            stderr: result.stderr,
                            duration_ms: result.duration_ms,
                            error: result.error,
                            metadata,
                            skipped: result.skipped,
                        };
                        all_results.push(code_result);
                    }
                }
            }
        }
    }

    if !all_results.is_empty() {
        let success_count = all_results.iter().filter(|r| r.success).count();
        let failure_count = all_results.len() - success_count;
        tracing::info!(
            "Code execution results: {} successful, {} failed",
            success_count,
            failure_count
        );

        // Log failures for visibility
        for result in &all_results {
            if !result.success {
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

    all_results
}

/// Convert plugin DependencySource to db DependencySourceInfo
fn convert_dependency_source(
    source: dodeca_code_execution_types::DependencySource,
) -> DependencySourceInfo {
    use dodeca_code_execution_types::DependencySource;
    match source {
        DependencySource::CratesIo => DependencySourceInfo::CratesIo,
        DependencySource::Git { url, commit } => DependencySourceInfo::Git { url, commit },
        DependencySource::Path { path } => DependencySourceInfo::Path { path },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_frontmatter() {
        let content = r#"+++
title = "Hello"
weight = 10
+++

# Content here
"#;
        let (fm, body) = split_frontmatter(content);
        assert!(fm.contains("title"));
        assert!(body.contains("# Content"));
    }

    #[test]
    fn test_resolve_internal_link() {
        // Section index files
        assert_eq!(resolve_internal_link("@/learn/_index.md"), "/learn");
        assert_eq!(
            resolve_internal_link("@/learn/showcases/_index.md"),
            "/learn/showcases"
        );
        assert_eq!(resolve_internal_link("@/_index.md"), "/");

        // Regular pages
        assert_eq!(resolve_internal_link("@/learn/page.md"), "/learn/page");
        assert_eq!(
            resolve_internal_link("@/learn/migration/serde.md"),
            "/learn/migration/serde"
        );

        // External links unchanged
        assert_eq!(
            resolve_internal_link("https://example.com"),
            "https://example.com"
        );
        assert_eq!(resolve_internal_link("/some/path/"), "/some/path/");

        // Links with hash fragments
        assert_eq!(
            resolve_internal_link("@/guide/ecosystem.md#anchor"),
            "/guide/ecosystem#anchor"
        );
        assert_eq!(
            resolve_internal_link("@/guide/_index.md#section"),
            "/guide#section"
        );
        assert_eq!(resolve_internal_link("@/_index.md#top"), "/#top");
    }
}
