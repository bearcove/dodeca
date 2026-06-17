use crate::db::{
    AllRenderedHtml, CharSet, CodeExecutionMetadata, CodeExecutionResult, CssOutput, DataRegistry,
    Db, DependencySourceInfo, ExternalLinkStatus, Heading, ImageVariant, MarkdownRenderSettings,
    OutputFile, Page, ParsedData, ProcessedImages, RenderedHtml, RenderedMarkdown, ReqDefinition,
    ResolvedDependencyInfo, SassFile, SassRegistry, Section, SiteOutput, SiteTree, SourceFile,
    SourceKind, SourceMap, SourceMapEntry, SourceRegistry, StaticFile, StaticFileOutput,
    StaticRegistry, TemplateFile, TemplateRegistry,
};
use picante::PicanteResult;

use crate::cells::{MarkdownParseError, parse_and_render_markdown};
use crate::image::{self, InputFormat, OutputFormat, add_width_suffix};
use crate::types::{HtmlBody, Route, SassContent, StaticPath, TemplateContent, Title};
use crate::url_rewrite::{rewrite_string_literals_in_js, rewrite_urls_in_css};
use facet::Facet;
use facet_value::{DestructuredRef, Value};
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

/// Narrow the full (mount-prefixed) template registry down to just the source
/// serving `route`, re-keyed by bare names — so a mounted source renders with
/// its own chrome. Reads the global config for the source mounts; with no
/// config (some unit paths) or a single source, the map is returned unchanged.
pub(crate) fn templates_for_route(
    all: HashMap<String, String>,
    route: &str,
) -> HashMap<String, String> {
    match crate::config::global_config() {
        Some(cfg) => crate::build_context::templates_for_route(all, route, &cfg.sources),
        None => all,
    }
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
    Ok(parse_data_file(content.as_str(), format).await.ok())
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

    tracing::info!(num_files = sass_map.len(), "Compiling SASS");

    let mut load_paths = Vec::new();
    if let Some(cfg) = crate::config::global_config() {
        let content_parent = cfg.content_dir.parent().unwrap_or(&cfg.content_dir);
        load_paths.push(content_parent.join("node_modules").to_string());

        if let Some(vite_dir) = cfg.paths().vite {
            let vite_node_modules = vite_dir.join("node_modules").to_string();
            if !load_paths.contains(&vite_node_modules) {
                load_paths.push(vite_node_modules);
            }
        }
    }

    match crate::cells::compile_sass(&sass_map, &load_paths).await {
        Ok(cell_sass_proto::SassResult::Success { css }) => {
            tracing::info!(output_bytes = css.len(), "SASS compilation complete");
            Ok(Some(CompiledCss(css)))
        }
        Ok(cell_sass_proto::SassResult::Error { message }) => {
            tracing::error!("SASS compilation failed: {}", message);
            Ok(None)
        }
        Err(e) => {
            tracing::error!("SASS compilation error: {}", e);
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
    pub asset: Option<String>,
    pub data: Option<String>,
    /// Custom fields from the `[extra]` table in frontmatter
    #[facet(default)]
    pub extra: Value,
}

/// Result of parsing a source file
pub type ParseFileResult = Result<ParsedData, crate::cells::MarkdownParseError>;

/// Parse a source file into ParsedData
/// This is the main tracked function - memoizes the result
#[picante::tracked]
#[tracing::instrument(skip_all, name = "parse_file", fields(path))]
pub async fn parse_file<DB: Db>(db: &DB, source: SourceFile) -> PicanteResult<ParseFileResult> {
    use cell_markdown_proto::ParseResult;

    let content = source.content(db)?;
    let path = source.path(db)?;
    let last_modified = source.last_modified(db)?;

    tracing::Span::current().record("path", path.as_str());
    tracing::debug!(path = %path, "Parsing markdown");

    let source_maps = MarkdownRenderSettings::source_maps(db)?.unwrap_or(false);

    // Use the markdown cell to parse frontmatter and render markdown
    let parse_result =
        match parse_and_render_markdown(path.as_str(), content.as_str(), source_maps).await {
            Ok(p) => p,
            Err(e) => return Ok(Err(e)),
        };

    // Handle the enum result
    let (frontmatter, html_output, headings_raw, reqs_raw, head_injections, source_map_raw) =
        match parse_result {
            ParseResult::Success {
                frontmatter,
                html,
                headings,
                reqs,
                head_injections,
                source_map,
            } => (
                frontmatter,
                html,
                headings,
                reqs,
                head_injections,
                source_map,
            ),
            ParseResult::Error { message } => {
                return Ok(Err(MarkdownParseError { message }));
            }
        };

    // Convert frontmatter from cell type
    let extra: Value = frontmatter.extra.clone();

    // Convert headings from cell type to internal type
    let headings: Vec<Heading> = headings_raw
        .into_iter()
        .map(|h| Heading {
            title: h.title,
            id: h.id,
            level: h.level,
        })
        .collect();

    // Convert rules from cell type to internal type
    let reqs: Vec<ReqDefinition> = reqs_raw
        .into_iter()
        .map(|r| ReqDefinition {
            id: r.id,
            anchor_id: r.anchor_id,
        })
        .collect();
    let source_map = convert_source_map(*source_map_raw);

    let body_html = HtmlBody::new(html_output);

    // Determine if this is a section (_index.md)
    let is_section = path.is_section_index();

    // Compute URL route
    let route = path.to_route();

    let title = if frontmatter.title.trim().is_empty() {
        default_title_from_source_path(path.as_str())
    } else {
        frontmatter.title
    };

    Ok(Ok(ParsedData {
        source_path: (*path).clone(),
        route,
        title: Title::new(title),
        description: frontmatter.description,
        weight: frontmatter.weight,
        body_html,
        is_section,
        headings,
        reqs,
        source_map,
        head_injections,
        last_updated: last_modified,
        extra,
        template: frontmatter.template,
    }))
}

pub fn default_title_from_source_path(path: &str) -> String {
    let path = path.strip_suffix(".md").unwrap_or(path);
    let slug = if path == "_index" {
        "home"
    } else if let Some(section_path) = path.strip_suffix("/_index") {
        section_path.rsplit('/').next().unwrap_or("home")
    } else {
        path.rsplit('/').next().unwrap_or("home")
    };

    title_case_slug(slug)
}

fn title_case_slug(slug: &str) -> String {
    let mut title = String::new();
    let mut capitalize_next = true;

    for ch in slug.chars() {
        if ch == '-' || ch == '_' {
            if !title.is_empty() && !title.ends_with(' ') {
                title.push(' ');
            }
            capitalize_next = true;
        } else if capitalize_next {
            title.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            title.push(ch);
        }
    }

    if title.trim().is_empty() {
        "Home".to_string()
    } else {
        title.trim().to_string()
    }
}

fn convert_source_kind(kind: cell_markdown_proto::SourceKind) -> SourceKind {
    match kind {
        cell_markdown_proto::SourceKind::Heading => SourceKind::Heading,
        cell_markdown_proto::SourceKind::Paragraph => SourceKind::Paragraph,
        cell_markdown_proto::SourceKind::BlockQuote => SourceKind::BlockQuote,
        cell_markdown_proto::SourceKind::List => SourceKind::List,
        cell_markdown_proto::SourceKind::ListItem => SourceKind::ListItem,
        cell_markdown_proto::SourceKind::DefinitionList => SourceKind::DefinitionList,
        cell_markdown_proto::SourceKind::DefinitionListTitle => SourceKind::DefinitionListTitle,
        cell_markdown_proto::SourceKind::DefinitionListDefinition => {
            SourceKind::DefinitionListDefinition
        }
        cell_markdown_proto::SourceKind::ThematicBreak => SourceKind::ThematicBreak,
        cell_markdown_proto::SourceKind::Table => SourceKind::Table,
        cell_markdown_proto::SourceKind::TableHead => SourceKind::TableHead,
        cell_markdown_proto::SourceKind::TableRow => SourceKind::TableRow,
        cell_markdown_proto::SourceKind::TableCell => SourceKind::TableCell,
        cell_markdown_proto::SourceKind::Image => SourceKind::Image,
    }
}

fn convert_source_map(source_map: cell_markdown_proto::SourceMap) -> SourceMap {
    SourceMap {
        source_path: source_map.source_path,
        entries: source_map
            .entries
            .into_iter()
            .map(|entry| SourceMapEntry {
                id: entry.id,
                kind: convert_source_kind(entry.kind),
                line_start: entry.line_start,
                line_end: entry.line_end,
                byte_start: entry.byte_start,
                byte_end: entry.byte_end,
            })
            .collect(),
    }
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

/// Error during template rendering
#[derive(Debug, Clone, facet::Facet)]
pub struct RenderError {
    pub route: crate::types::Route,
    pub error: cell_gingembre_proto::TemplateRenderError,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Error rendering {}: {}", self.route, self.error.message)
    }
}

impl std::error::Error for RenderError {}

/// Error when resolving wiki-style markdown links.
#[derive(Debug, Clone, facet::Facet)]
pub struct WikiLinkError {
    pub route: Route,
    pub target: String,
    pub reason: WikiLinkErrorReason,
}

#[derive(Debug, Clone, facet::Facet)]
#[repr(u8)]
pub enum WikiLinkErrorReason {
    Missing,
    Ambiguous {
        candidates: Vec<String>,
    },
    /// The target names a configured source whose content directory is absent —
    /// a sibling repo that isn't checked out locally.
    SourceNotCheckedOut {
        source: String,
        path: String,
        git: Option<String>,
    },
}

#[derive(Debug, Clone, facet::Facet)]
pub struct WikiLinkBuildError {
    pub errors: Vec<WikiLinkError>,
}

impl std::fmt::Display for WikiLinkBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Failed to resolve {} wiki link(s):", self.errors.len())?;
        for err in &self.errors {
            match &err.reason {
                WikiLinkErrorReason::Missing => {
                    writeln!(f, "  - {}: [[{}]] target not found", err.route, err.target)?;
                }
                WikiLinkErrorReason::Ambiguous { candidates } => {
                    writeln!(
                        f,
                        "  - {}: [[{}]] is ambiguous; candidates: {}",
                        err.route,
                        err.target,
                        candidates.join(", ")
                    )?;
                }
                WikiLinkErrorReason::SourceNotCheckedOut { source, path, git } => {
                    write!(
                        f,
                        "  - {}: [[{}]] → source `{source}` is not checked out (expected at {path})",
                        err.route, err.target,
                    )?;
                    match git {
                        Some(git) => writeln!(f, "; run `git clone {git} {path}`")?,
                        None => writeln!(f)?,
                    }
                }
            }
        }
        Ok(())
    }
}

impl std::error::Error for WikiLinkBuildError {}

/// Errors that can occur during site generation
#[derive(Debug, Clone, facet::Facet)]
#[repr(u8)]
pub enum SiteError {
    /// Errors during markdown parsing
    Parse(BuildError),
    /// Error during template rendering
    Render(RenderError),
    /// Error resolving wiki-style markdown links
    WikiLinks(WikiLinkBuildError),
}

impl std::fmt::Display for SiteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SiteError::Parse(e) => write!(f, "{}", e),
            SiteError::Render(e) => write!(f, "{}", e),
            SiteError::WikiLinks(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for SiteError {}

impl From<BuildError> for SiteError {
    fn from(e: BuildError) -> Self {
        SiteError::Parse(e)
    }
}

impl From<RenderError> for SiteError {
    fn from(e: RenderError) -> Self {
        SiteError::Render(e)
    }
}

impl From<WikiLinkBuildError> for SiteError {
    fn from(e: WikiLinkBuildError) -> Self {
        SiteError::WikiLinks(e)
    }
}

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

    let frontmatter_schema_errors = crate::frontmatter_schema::validate(&parsed);
    if !frontmatter_schema_errors.is_empty() {
        return Ok(Err(frontmatter_schema_errors
            .into_iter()
            .map(|error| SourceParseError {
                path: error.source_path,
                error: MarkdownParseError {
                    message: error.message,
                },
            })
            .collect()));
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
                source_map: data.source_map.clone(),
                head_injections: data.head_injections.clone(),
                last_updated: data.last_updated,
                extra: data.extra.clone(),
                template: data.template.clone(),
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
        source_map: SourceMap::default(),
        head_injections: Vec::new(),
        last_updated: 0,
        extra: Value::default(),
        template: None,
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
                source_map: data.source_map.clone(),
                head_injections: data.head_injections.clone(),
                last_updated: data.last_updated,
                extra: data.extra.clone(),
                template: data.template.clone(),
            },
        );
    }

    Ok(Ok(SiteTree { sections, pages }))
}

/// Build a mapping from source paths to routes.
///
/// This is used to resolve `@/` links in markdown. When a page contains `@/guide/intro.md`,
/// we look up "guide/intro.md" in this map to get the actual route (which may differ due to
/// custom slugs).
///
/// This query depends on all parse_file results, so changing any source file will invalidate
/// any page that links to it (via picante's dependency tracking).
#[picante::tracked]
pub async fn source_to_route_map<DB: Db>(db: &DB) -> PicanteResult<HashMap<String, String>> {
    let sources = SourceRegistry::sources(db)?.unwrap_or_default();
    let mut map = HashMap::new();

    for source in sources.iter() {
        // Calling parse_file creates a dependency on this source
        if let Ok(data) = parse_file(db, *source).await? {
            // Map source path to route
            // e.g., "guide/intro.md" -> "/guide/intro/"
            map.insert(data.source_path.to_string(), data.route.to_string());
        }
    }

    Ok(map)
}

#[derive(Debug, Clone, Default)]
struct ResolveMap {
    resolved: HashMap<String, String>,
    ambiguous: HashMap<String, Vec<String>>,
}

impl ResolveMap {
    fn from_candidates(candidates: HashMap<String, Vec<String>>) -> Self {
        let mut resolved = HashMap::new();
        let mut ambiguous = HashMap::new();
        for (key, mut routes) in candidates {
            routes.sort();
            routes.dedup();
            if routes.len() == 1 {
                resolved.insert(key, routes.remove(0));
            } else {
                ambiguous.insert(key, routes);
            }
        }
        Self {
            resolved,
            ambiguous,
        }
    }
}

/// Wiki-link resolution that respects source provenance.
///
/// A bare `[[overview]]` resolves within the linking page's *own* source
/// (`local`), so a source authored standalone keeps its internal links when
/// mounted — even if another source shares the slug. Cross-source links must be
/// qualified (`[[spec/build/overview]]` / `[[build/overview]]`) and resolve via
/// the mount-qualified `global` namespace. A single-source site has one local
/// namespace at the root mount and behaves exactly as before.
#[derive(Debug, Clone, Default)]
struct WikiLinkIndex {
    /// Mount segment (`""` = root) → that source's local namespace.
    local: HashMap<String, ResolveMap>,
    /// Mount-qualified namespace for explicit cross-source links.
    global: ResolveMap,
}

impl WikiLinkIndex {
    fn build(site_tree: &SiteTree) -> Self {
        let mut local: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
        let mut global: HashMap<String, Vec<String>> = HashMap::new();

        for section in site_tree.sections.values() {
            add_provenance_candidates(
                &mut local,
                &mut global,
                section.title.as_str(),
                &section.route,
            );
        }
        for page in site_tree.pages.values() {
            add_provenance_candidates(&mut local, &mut global, page.title.as_str(), &page.route);
        }

        WikiLinkIndex {
            local: local
                .into_iter()
                .map(|(name, c)| (name, ResolveMap::from_candidates(c)))
                .collect(),
            global: ResolveMap::from_candidates(global),
        }
    }

    /// Resolve a wiki-link key for the page identified by `source_path`. Tries
    /// the page's own source namespace first (bare/local links), then the
    /// name-qualified namespace (explicit cross-source links).
    fn resolve(&self, key: &str, source_path: &str) -> Option<&String> {
        let name = source_of(source_path)
            .map(|s| s.name.as_str())
            .unwrap_or("");
        self.local
            .get(name)
            .and_then(|m| m.resolved.get(key))
            .or_else(|| self.global.resolved.get(key))
    }

    /// The effective flat resolved map for a page in `source` (a route or source
    /// path): the name-qualified globals as a base, with the page's local
    /// namespace overriding (bare links resolve locally). Used by the HTML-level
    /// wiki-link pass, which resolves against a single map per page.
    fn resolved_for(&self, source: &str) -> HashMap<String, String> {
        let name = source_of(source).map(|s| s.name.as_str()).unwrap_or("");
        let mut map = self.global.resolved.clone();
        if let Some(local) = self.local.get(name) {
            for (key, route) in &local.resolved {
                map.insert(key.clone(), route.clone());
            }
        }
        map
    }

    /// Ambiguity candidates for a key as seen from `source` (page's local
    /// namespace first, then global) — for the "ambiguous vs missing" diagnostic.
    fn ambiguity(&self, source: &str, key: &str) -> Option<&Vec<String>> {
        let name = source_of(source).map(|s| s.name.as_str()).unwrap_or("");
        self.local
            .get(name)
            .and_then(|m| m.ambiguous.get(key))
            .or_else(|| self.global.ambiguous.get(key))
    }
}

/// Strip a mount segment from a trimmed route path, yielding the source-relative
/// path (`spec/build/overview` with seg `spec/build` → `overview`; the source's
/// own root section → ``).
fn strip_mount_segment<'a>(trimmed_route: &'a str, seg: &str) -> &'a str {
    if seg.is_empty() {
        return trimmed_route;
    }
    match trimmed_route.strip_prefix(seg) {
        Some("") => "",
        Some(rest) => rest.strip_prefix('/').unwrap_or(trimmed_route),
        None => trimmed_route,
    }
}

/// Register a route's wiki-link candidates in both namespaces: source-relative
/// identifiers (title, leaf slug, the source-relative path and its suffixes) in
/// the page's own `local` namespace (keyed by source name), and the
/// **name-qualified** path + suffixes in `global` (so a source is linkable by
/// name regardless of where it is mounted — `[[<name>:slug]]`).
fn add_provenance_candidates(
    local: &mut HashMap<String, HashMap<String, Vec<String>>>,
    global: &mut HashMap<String, Vec<String>>,
    title: &str,
    route: &Route,
) {
    let source = source_of(route.as_str());
    let name = source.map(|s| s.name.clone()).unwrap_or_default();
    let seg = source
        .map(|s| s.mount.trim_matches('/').to_string())
        .unwrap_or_default();
    let trimmed = route.as_str().trim_matches('/');
    let local_path = strip_mount_segment(trimmed, &seg).to_string();

    let local_map = local.entry(name.clone()).or_default();
    add_wiki_route_candidates(local_map, title, route);
    add_wiki_route_candidates(local_map, &local_path, route);
    if let Some(slug) = route_leaf_slug(route) {
        add_wiki_route_candidates(local_map, &slug, route);
    }
    for suffix in route_path_suffixes(&local_path) {
        add_wiki_route_candidates(local_map, &suffix, route);
    }

    // Name-qualified global identifiers — only for named (non-root-degenerate)
    // sources. `<name>/<source-relative path>`, its suffixes, and `<name>/<leaf>`.
    if !name.is_empty() {
        add_wiki_route_candidates(global, &format!("{name}/{local_path}"), route);
        if let Some(slug) = route_leaf_slug(route) {
            add_wiki_route_candidates(global, &format!("{name}/{slug}"), route);
        }
        for suffix in route_path_suffixes(&local_path) {
            add_wiki_route_candidates(global, &format!("{name}/{suffix}"), route);
        }
    }
}

fn add_wiki_route_candidates(
    candidates: &mut HashMap<String, Vec<String>>,
    target: &str,
    route: &Route,
) {
    if let Some(key) = wiki_link_key(target) {
        candidates
            .entry(key)
            .or_default()
            .push(wiki_link_route(route));
    }
}

/// The multi-segment path suffixes of a route, longest first
/// (`/spec/build/overview/` → `["spec/build/overview", "build/overview"]`).
/// The bare leaf (`overview`) and full route are already added elsewhere; these
/// are the in-between qualifiers used to disambiguate a colliding leaf slug.
fn route_path_suffixes(route: &str) -> Vec<String> {
    let segs: Vec<&str> = route
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let mut out = Vec::new();
    for start in 0..segs.len() {
        if segs.len() - start >= 2 {
            out.push(segs[start..].join("/"));
        }
    }
    out
}

fn wiki_link_route(route: &Route) -> String {
    let route = route.as_str();
    if route == "/" || route.ends_with('/') {
        route.to_string()
    } else {
        format!("{route}/")
    }
}

fn route_to_markdown_url(route: &str) -> String {
    let route = route.trim_end_matches('/');
    if route.is_empty() {
        "/index.md".to_string()
    } else {
        format!("{route}.md")
    }
}

fn markdown_route_map(site_tree: &SiteTree) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for route in site_tree.sections.keys().chain(site_tree.pages.keys()) {
        let markdown_url = route_to_markdown_url(route.as_str());
        let normalized = route.as_str().trim_end_matches('/');
        if normalized.is_empty() {
            map.insert("/".to_string(), markdown_url.clone());
            map.insert("/index".to_string(), markdown_url.clone());
        } else {
            map.insert(normalized.to_string(), markdown_url.clone());
            map.insert(format!("{normalized}/"), markdown_url);
        }
    }
    map
}

fn rewrite_markdown_blocks(
    blocks: &mut [marq::Block],
    source_path: &str,
    source_route_map: &HashMap<String, String>,
    route_markdown_map: &HashMap<String, String>,
    wiki_link_index: &WikiLinkIndex,
) {
    for block in blocks {
        match block {
            marq::Block::Paragraph(inlines) => rewrite_markdown_inlines(
                inlines,
                source_path,
                source_route_map,
                route_markdown_map,
                wiki_link_index,
            ),
            marq::Block::Heading { content, .. } => rewrite_markdown_inlines(
                content,
                source_path,
                source_route_map,
                route_markdown_map,
                wiki_link_index,
            ),
            marq::Block::BlockQuote(inner) => rewrite_markdown_blocks(
                inner,
                source_path,
                source_route_map,
                route_markdown_map,
                wiki_link_index,
            ),
            marq::Block::List { items, .. } => {
                for item in items {
                    rewrite_markdown_blocks(
                        item,
                        source_path,
                        source_route_map,
                        route_markdown_map,
                        wiki_link_index,
                    );
                }
            }
            marq::Block::Table { header, rows, .. } => {
                for cell in header {
                    rewrite_markdown_inlines(
                        cell,
                        source_path,
                        source_route_map,
                        route_markdown_map,
                        wiki_link_index,
                    );
                }
                for row in rows {
                    for cell in row {
                        rewrite_markdown_inlines(
                            cell,
                            source_path,
                            source_route_map,
                            route_markdown_map,
                            wiki_link_index,
                        );
                    }
                }
            }
            marq::Block::CodeBlock { .. }
            | marq::Block::ThematicBreak
            | marq::Block::HtmlBlock(_) => {}
        }
    }
}

fn rewrite_markdown_inlines(
    inlines: &mut Vec<marq::Inline>,
    source_path: &str,
    source_route_map: &HashMap<String, String>,
    route_markdown_map: &HashMap<String, String>,
    wiki_link_index: &WikiLinkIndex,
) {
    for inline in inlines {
        match inline {
            marq::Inline::Emphasis(inner)
            | marq::Inline::Strong(inner)
            | marq::Inline::Strikethrough(inner) => rewrite_markdown_inlines(
                inner,
                source_path,
                source_route_map,
                route_markdown_map,
                wiki_link_index,
            ),
            marq::Inline::Link { url, content, .. } => {
                rewrite_markdown_inlines(
                    content,
                    source_path,
                    source_route_map,
                    route_markdown_map,
                    wiki_link_index,
                );
                *url = rewrite_markdown_url(
                    url,
                    source_path,
                    source_route_map,
                    route_markdown_map,
                    wiki_link_index,
                );
            }
            marq::Inline::WikiLink { target, label } => {
                let key = wiki_link_key(target);
                rewrite_markdown_inlines(
                    label,
                    source_path,
                    source_route_map,
                    route_markdown_map,
                    wiki_link_index,
                );
                if let Some(route) = key.and_then(|key| wiki_link_index.resolve(&key, source_path))
                {
                    let content = if label.is_empty() {
                        vec![marq::Inline::Text(target.clone())]
                    } else {
                        label.clone()
                    };
                    *inline = marq::Inline::Link {
                        url: route_to_markdown_url(route),
                        title: String::new(),
                        content,
                    };
                }
            }
            marq::Inline::Image { url, alt, .. } => {
                rewrite_markdown_inlines(
                    alt,
                    source_path,
                    source_route_map,
                    route_markdown_map,
                    wiki_link_index,
                );
                *url = rewrite_markdown_url(
                    url,
                    source_path,
                    source_route_map,
                    route_markdown_map,
                    wiki_link_index,
                );
            }
            marq::Inline::Text(_)
            | marq::Inline::Code(_)
            | marq::Inline::SoftBreak
            | marq::Inline::HardBreak
            | marq::Inline::Html(_) => {}
        }
    }
}

fn rewrite_markdown_url(
    url: &str,
    source_path: &str,
    source_route_map: &HashMap<String, String>,
    route_markdown_map: &HashMap<String, String>,
    wiki_link_index: &WikiLinkIndex,
) -> String {
    if url.starts_with('#') || has_scheme(url) {
        return url.to_string();
    }

    let (path, suffix) = split_link_suffix(url);

    if let Some(key) = path.strip_prefix("dodeca-wiki:")
        && let Some(route) = wiki_link_index.resolve(key, source_path)
    {
        return format!("{}{}", route_to_markdown_url(route), suffix);
    }

    if let Some(source) = path.strip_prefix("@/") {
        if let Some(route) = source_route_map.get(source) {
            return format!("{}{}", route_to_markdown_url(route), suffix);
        }
        return url.to_string();
    }

    if path.starts_with('/') {
        let route_key = path.trim_end_matches('/');
        if let Some(markdown_url) = route_markdown_map.get(route_key) {
            return format!("{markdown_url}{suffix}");
        }
        return url.to_string();
    }

    if path.ends_with(".md") {
        let source = normalize_relative_source_path(source_path, path);
        if let Some(route) = source_route_map.get(&source) {
            return format!("{}{}", route_to_markdown_url(route), suffix);
        }
    }

    url.to_string()
}

fn has_scheme(url: &str) -> bool {
    let Some(colon) = url.find(':') else {
        return false;
    };
    !url[..colon].contains('/')
}

fn split_link_suffix(url: &str) -> (&str, &str) {
    let split_at = url
        .find('#')
        .into_iter()
        .chain(url.find('?'))
        .min()
        .unwrap_or(url.len());
    url.split_at(split_at)
}

fn normalize_relative_source_path(source_path: &str, relative: &str) -> String {
    let mut parts = Vec::new();
    if let Some((parent, _)) = source_path.rsplit_once('/') {
        parts.extend(parent.split('/').filter(|part| !part.is_empty()));
    }
    parts.extend(relative.split('/'));

    let mut normalized = Vec::new();
    for part in parts {
        match part {
            "" | "." => {}
            ".." => {
                normalized.pop();
            }
            part => normalized.push(part),
        }
    }

    normalized.join("/")
}

fn route_leaf_slug(route: &Route) -> Option<String> {
    route
        .as_str()
        .trim_matches('/')
        .rsplit('/')
        .next()
        .filter(|slug| !slug.is_empty())
        .map(str::to_string)
}

fn wiki_link_key(target: &str) -> Option<String> {
    let mut key = String::new();
    let mut last_was_dash = true;

    for c in target.chars() {
        if c.is_alphanumeric() {
            for lower in c.to_lowercase() {
                key.push(lower);
            }
            last_was_dash = false;
        } else if !last_was_dash {
            key.push('-');
            last_was_dash = true;
        }
    }

    while key.ends_with('-') {
        key.pop();
    }

    if key.is_empty() { None } else { Some(key) }
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
#[tracing::instrument(skip_all, name = "render_page", fields(route = %route))]
pub async fn render_page<DB: Db>(
    db: &DB,
    route: Route,
    can_edit: bool,
) -> PicanteResult<Result<RenderedHtml, SiteError>> {
    use crate::render::render_page_template;

    tracing::debug!(route = %route, can_edit, "Rendering page");

    // Build tree (cached)
    let site_tree = match build_tree(db).await? {
        Ok(tree) => tree,
        Err(errors) => return Ok(Err(BuildError { errors }.into())),
    };

    // Pre-load all templates, then narrow to the source serving this route so a
    // mounted source renders with its own chrome (`{% extends %}` resolves
    // within the source's own template set). Single-source sites are unchanged.
    let templates = templates_for_route(load_all_templates(db).await?, route.as_str());

    // Find the page
    let page = site_tree
        .pages
        .get(&route)
        .expect("Page not found for route");

    // Render via the statically linked gingembre renderer.
    match render_page_template(page, &site_tree, templates, can_edit).await {
        Ok(html) => Ok(Ok(RenderedHtml(html))),
        Err(error) => Ok(Err(RenderError {
            route: route.clone(),
            error,
        }
        .into())),
    }
}

/// Render a single page or section back to markdown.
///
/// The body is parsed through marq's markdown AST, then Dodeca-specific links
/// are rewritten against the same route and wiki-link indexes used by HTML.
#[picante::tracked]
#[tracing::instrument(skip_all, name = "render_page_markdown", fields(route = %route))]
pub async fn render_page_markdown<DB: Db>(
    db: &DB,
    route: Route,
) -> PicanteResult<Result<RenderedMarkdown, SiteError>> {
    tracing::debug!(route = %route, "Rendering page markdown");

    let site_tree = match build_tree(db).await? {
        Ok(tree) => tree,
        Err(errors) => return Ok(Err(BuildError { errors }.into())),
    };

    let sources = SourceRegistry::sources(db)?.unwrap_or_default();
    let mut selected_source = None;

    for source in sources.iter() {
        let data = match parse_file(db, *source).await? {
            Ok(data) => data,
            Err(error) => {
                return Ok(Err(BuildError {
                    errors: vec![SourceParseError {
                        path: source.path(db)?.to_string(),
                        error,
                    }],
                }
                .into()));
            }
        };

        if data.route == route {
            selected_source = Some(*source);
            break;
        }
    }

    let source = selected_source.expect("Source not found for route");
    let source_path = source.path(db)?.to_string();
    let content = source.content(db)?;
    let stripped = marq::strip_frontmatter(content.as_str());

    let mut blocks = marq::parse_ast(stripped.body);
    let source_route_map = source_to_route_map(db).await?;
    let route_markdown_map = markdown_route_map(&site_tree);
    let wiki_link_index = WikiLinkIndex::build(&site_tree);

    rewrite_markdown_blocks(
        &mut blocks,
        &source_path,
        &source_route_map,
        &route_markdown_map,
        &wiki_link_index,
    );

    let mut markdown = String::new();
    if let (Some(raw), Some(format)) = (stripped.raw, stripped.format) {
        let delimiter = match format {
            marq::FrontmatterFormat::Toml => "+++",
            marq::FrontmatterFormat::Yaml => "---",
        };
        markdown.push_str(delimiter);
        markdown.push('\n');
        markdown.push_str(raw);
        markdown.push('\n');
        markdown.push_str(delimiter);
        markdown.push_str("\n\n");
    }
    markdown.push_str(&marq::render_to_markdown(&blocks));

    Ok(Ok(RenderedMarkdown(markdown)))
}

/// Render a single section to HTML
/// This tracked query depends on the section content, templates actually used, data files, and site tree.
/// Template dependencies are tracked lazily - only templates loaded during rendering are recorded.
/// Data dependencies are also tracked lazily - only data paths actually accessed become dependencies.
#[picante::tracked]
#[tracing::instrument(skip_all, name = "render_section", fields(route = %route))]
pub async fn render_section<DB: Db>(
    db: &DB,
    route: Route,
    can_edit: bool,
) -> PicanteResult<Result<RenderedHtml, SiteError>> {
    use crate::render::render_section_template;

    tracing::debug!(route = %route, can_edit, "Rendering section");

    // Build tree (cached)
    let site_tree = match build_tree(db).await? {
        Ok(tree) => tree,
        Err(errors) => return Ok(Err(BuildError { errors }.into())),
    };

    // Pre-load all templates, then narrow to the source serving this route so a
    // mounted source renders with its own chrome (see `render_page`).
    let templates = templates_for_route(load_all_templates(db).await?, route.as_str());

    // Find the section
    let section = site_tree
        .sections
        .get(&route)
        .expect("Section not found for route");

    // Render via the statically linked gingembre renderer.
    match render_section_template(section, &site_tree, templates, can_edit).await {
        Ok(html) => Ok(Ok(RenderedHtml(html))),
        Err(error) => Ok(Err(RenderError {
            route: route.clone(),
            error,
        }
        .into())),
    }
}

/// Load a single static file's content - tracked
#[picante::tracked]
pub async fn load_static<DB: Db>(db: &DB, file: StaticFile) -> PicanteResult<Vec<u8>> {
    let content = file.content(db)?.clone();
    tracing::debug!(path = %file.path(db)?.as_str(), size = content.len(), "load_static called");
    Ok(content)
}

/// A single entry in the Vite manifest
#[derive(Facet, Default, Clone)]
struct ViteManifestEntry {
    /// Output file path (e.g., "assets/main-BhKl2bGh.js")
    file: String,
    /// Source file path (e.g., "src/main.ts")
    #[facet(default)]
    src: Option<String>,
    /// Whether this is an entry point
    #[facet(rename = "isEntry", default)]
    is_entry: Option<bool>,
    /// CSS files imported by this entry
    #[facet(default)]
    css: Option<Vec<String>>,
    /// Other chunks this entry imports (for transitive CSS resolution)
    #[facet(default)]
    imports: Option<Vec<String>>,
}

/// Parse Vite manifest and return source → cache-busted output path mappings
///
/// The manifest at `.vite/manifest.json` maps source files to their built outputs:
/// ```json
/// {
///   "src/main.ts": {
///     "file": "assets/main-BhKl2bGh.js",
///     "src": "src/main.ts",
///     "isEntry": true
///   }
/// }
/// ```
///
/// Returns a HashMap mapping `/src/main.ts` → `/assets/main-BhKl2bGh.xxx.js`
/// (chained through dodeca's cache-busting for files without vite hashes)
#[picante::tracked]
pub async fn vite_manifest_map<DB: Db>(db: &DB) -> PicanteResult<HashMap<String, String>> {
    let mut result = HashMap::new();

    // Look for .vite/manifest.json in static files
    let static_files = StaticRegistry::files(db)?.unwrap_or_default();
    let manifest_file = static_files.iter().find(|f| {
        f.path(db)
            .ok()
            .map(|p| p.as_str() == ".vite/manifest.json")
            .unwrap_or(false)
    });

    let Some(manifest_file) = manifest_file else {
        return Ok(result);
    };

    let content = manifest_file.content(db)?;
    let Ok(manifest_str) = std::str::from_utf8(&content) else {
        tracing::warn!("Vite manifest is not valid UTF-8");
        return Ok(result);
    };

    // Parse as JSON - the manifest is { "src/file.ts": { "file": "assets/out.js", ... }, ... }
    let manifest: HashMap<String, ViteManifestEntry> = match facet_json::from_str(manifest_str) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to parse Vite manifest: {}", e);
            return Ok(result);
        }
    };

    // Build a map of vite output path → static file for cache-bust lookups
    let static_file_map: HashMap<String, StaticFile> = static_files
        .iter()
        .filter_map(|f| {
            let path = f.path(db).ok()?.as_str().to_string();
            Some((path, *f))
        })
        .collect();

    for (src, entry) in manifest {
        let vite_output = &entry.file;

        // Look up the static file to get its cache-busted path
        let final_path = if let Some(static_file) = static_file_map.get(vite_output) {
            // Get the cache-busted output path
            let output = static_file_output(db, *static_file).await?;
            format!("/{}", output.cache_busted_path)
        } else {
            // File not found in static files, use raw vite output
            tracing::warn!(vite_output = %vite_output, "Vite output file not found in static files");
            format!("/{vite_output}")
        };

        let from = format!("/{src}");
        tracing::trace!(from = %from, to = %final_path, "vite manifest mapping");
        result.insert(from, final_path);

        // Also map any CSS files this entry imports
        if let Some(css_files) = entry.css {
            for css in css_files {
                let css_final = if let Some(static_file) = static_file_map.get(&css) {
                    let output = static_file_output(db, *static_file).await?;
                    format!("/{}", output.cache_busted_path)
                } else {
                    format!("/{css}")
                };
                result.insert(format!("/{css}"), css_final);
            }
        }
    }

    if !result.is_empty() {
        tracing::debug!(count = result.len(), "Loaded Vite manifest mappings");
    }

    Ok(result)
}

/// Collect transitive CSS files for a manifest entry by following its imports chain
fn collect_transitive_css(
    manifest: &HashMap<String, ViteManifestEntry>,
    entry_key: &str,
    visited: &mut std::collections::HashSet<String>,
) -> Vec<String> {
    // Prevent infinite loops
    if visited.contains(entry_key) {
        return Vec::new();
    }
    visited.insert(entry_key.to_string());

    let Some(entry) = manifest.get(entry_key) else {
        return Vec::new();
    };

    let mut css_files = Vec::new();

    // Collect direct CSS
    if let Some(css) = &entry.css {
        css_files.extend(css.iter().cloned());
    }

    // Recursively collect CSS from imported chunks
    if let Some(imports) = &entry.imports {
        for import_key in imports {
            css_files.extend(collect_transitive_css(manifest, import_key, visited));
        }
    }

    css_files
}

/// Returns a map of Vite entry source paths to their required CSS files (including transitive imports)
///
/// When HTML contains `<script src="/src/monaco/main.ts">`, we need to inject `<link>` tags
/// for any CSS files that this entry point (and its imports) require.
#[picante::tracked]
pub async fn vite_css_for_entries<DB: Db>(db: &DB) -> PicanteResult<HashMap<String, Vec<String>>> {
    let mut result: HashMap<String, Vec<String>> = HashMap::new();

    // Look for .vite/manifest.json in static files
    let static_files = StaticRegistry::files(db)?.unwrap_or_default();
    let manifest_file = static_files.iter().find(|f| {
        f.path(db)
            .ok()
            .map(|p| p.as_str() == ".vite/manifest.json")
            .unwrap_or(false)
    });

    let Some(manifest_file) = manifest_file else {
        return Ok(result);
    };

    let content = manifest_file.content(db)?;
    let Ok(manifest_str) = std::str::from_utf8(&content) else {
        return Ok(result);
    };

    let manifest: HashMap<String, ViteManifestEntry> = match facet_json::from_str(manifest_str) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to parse Vite manifest for CSS collection: {}", e);
            return Ok(result);
        }
    };

    // Build a map of vite output path → static file for cache-bust lookups
    let static_file_map: HashMap<String, StaticFile> = static_files
        .iter()
        .filter_map(|f| {
            let path = f.path(db).ok()?.as_str().to_string();
            Some((path, *f))
        })
        .collect();

    // For each entry point, collect all transitive CSS
    for (src, entry) in &manifest {
        // Skip non-entry points and chunk keys (start with _)
        if !entry.is_entry.unwrap_or(false) || src.starts_with('_') {
            continue;
        }

        let mut visited = std::collections::HashSet::new();
        let css_files = collect_transitive_css(&manifest, src, &mut visited);

        if css_files.is_empty() {
            continue;
        }

        // Convert CSS paths to cache-busted URLs
        let mut css_urls = Vec::new();
        for css in css_files {
            let css_url = if let Some(static_file) = static_file_map.get(&css) {
                let output = static_file_output(db, *static_file).await?;
                format!("/{}", output.cache_busted_path)
            } else {
                format!("/{css}")
            };
            // Avoid duplicates
            if !css_urls.contains(&css_url) {
                css_urls.push(css_url);
            }
        }

        let source_path = format!("/{src}");
        let built_path = format!("/{}", entry.file);
        let cache_busted_built_path = if let Some(static_file) = static_file_map.get(&entry.file) {
            let output = static_file_output(db, *static_file).await?;
            Some(format!("/{}", output.cache_busted_path))
        } else {
            None
        };

        tracing::debug!(
            source = %source_path,
            css_count = css_urls.len(),
            "Vite entry CSS dependencies"
        );
        result.insert(source_path, css_urls.clone());
        result.insert(built_path, css_urls.clone());
        if let Some(cache_busted) = cache_busted_built_path {
            result.insert(cache_busted, css_urls);
        }
    }

    Ok(result)
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
    use crate::cells::decompress_font;

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

    match decompress_font(font_data.clone()).await {
        Ok(decompressed) => {
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
        Err(e) => {
            tracing::warn!(
                "Failed to decompress font {}: {}",
                font_file.path(db)?.as_str(),
                e
            );
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
    use crate::cells::{compress_to_woff2, subset_font as subset_font_direct};

    let path = font_file.path(db)?.as_str().to_string();
    let num_chars = chars.chars(db).map(|c| c.len()).unwrap_or(0);
    tracing::info!(font = %path, num_chars, "Subsetting font");

    // First, decompress the font (handles WOFF2/WOFF1 -> TTF)
    let Some(decompressed) = decompress_font(db, font_file).await? else {
        return Ok(None);
    };

    let char_vec: Vec<char> = chars.chars(db)?.to_vec();

    let input = cell_fonts_proto::SubsetFontInput {
        data: decompressed.clone(),
        chars: char_vec.clone(),
    };
    let subsetted = match subset_font_direct(input).await {
        Ok(cell_fonts_proto::FontResult::SubsetSuccess { data }) => data,
        Ok(other) => {
            tracing::warn!("Unexpected font result: {:?}", other);
            return Ok(None);
        }
        Err(e) => {
            tracing::warn!(
                "Failed to subset font {}: {}",
                font_file.path(db)?.as_str(),
                e
            );
            return Ok(None);
        }
    };

    match compress_to_woff2(subsetted.clone()).await {
        Ok(woff2) => {
            tracing::debug!(
                "Subsetted font {} ({} chars, {} -> {} bytes)",
                font_file.path(db)?.as_str(),
                char_vec.len(),
                decompressed.len(),
                woff2.len()
            );
            Ok(Some(woff2))
        }
        Err(e) => {
            tracing::warn!(
                "Failed to compress font {} to WOFF2: {}",
                font_file.path(db)?.as_str(),
                e
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
        tracing::debug!(image = %path, "Image cache hit");
        return Ok(Some(cached));
    }

    tracing::debug!(image = %path, bytes = data.len(), "Processing image");

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
pub async fn build_site<DB: Db>(db: &DB) -> PicanteResult<Result<SiteOutput, SiteError>> {
    tracing::debug!("build_site: starting");
    let mut files = Vec::new();

    // Build the site tree to get all routes
    let site_tree = match build_tree(db).await? {
        Ok(tree) => tree,
        Err(errors) => return Ok(Err(BuildError { errors }.into())),
    };

    // --- Phase 1: Render all HTML pages using serve_html ---
    // This reuses the exact same pipeline as `ddc serve`, ensuring consistency.
    // The static build is viewer-independent, so render anonymously (`can_edit = false`).
    for route in site_tree.sections.keys() {
        match serve_html(db, route.clone(), false).await? {
            Ok(Some(served)) => {
                // Extract links using HTML cell (proper parser, not regex)
                let extracted = crate::cells::extract_links_from_html(served.html.clone())
                    .await
                    .unwrap_or_default();
                files.push(OutputFile::Html {
                    route: route.clone(),
                    content: served.html,
                    head_injections: served.head_injections,
                    hrefs: extracted.hrefs,
                    element_ids: extracted.element_ids,
                });
            }
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    for route in site_tree.pages.keys() {
        match serve_html(db, route.clone(), false).await? {
            Ok(Some(served)) => {
                // Extract links using HTML cell (proper parser, not regex)
                let extracted = crate::cells::extract_links_from_html(served.html.clone())
                    .await
                    .unwrap_or_default();
                files.push(OutputFile::Html {
                    route: route.clone(),
                    content: served.html,
                    head_injections: served.head_injections,
                    hrefs: extracted.hrefs,
                    element_ids: extracted.element_ids,
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
    tracing::debug!(
        count = static_files.len(),
        "build_site: processing static files"
    );
    for file in static_files.iter() {
        let path = file.path(db)?.as_str().to_string();
        tracing::trace!(path = %path, "build_site: processing static file");

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

    // --- Phase 5: Full-text search — runtime assets + content index ---
    files.extend(crate::search::runtime_output_files());
    files.extend(crate::search::search_index_files(db).await?);

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
///
/// Internal `@/` links are resolved using the source-to-route map, which creates
/// picante dependencies: if a linked page changes, the linking page is invalidated.
#[picante::tracked]
pub async fn all_rendered_html<DB: Db>(
    db: &DB,
) -> PicanteResult<Result<AllRenderedHtml, SiteError>> {
    use crate::render::{render_page_template, render_section_template};
    use crate::url_rewrite::{resolve_internal_links, resolve_relative_links, resolve_wiki_links};

    let site_tree = match build_tree(db).await? {
        Ok(tree) => tree,
        Err(errors) => return Ok(Err(BuildError { errors }.into())),
    };
    let template_map = load_all_templates(db).await?;

    // Get the source-to-route map for internal link resolution
    // This creates dependencies on all source files via parse_file
    let source_route_map = source_to_route_map(db).await?;
    let wiki_link_index = WikiLinkIndex::build(&site_tree);
    let mut unresolved_wiki_links = Vec::new();

    let mut pages = HashMap::new();

    for (route, section) in &site_tree.sections {
        // Anonymous (`can_edit = false`): this aggregate feeds global font
        // subsetting and must stay viewer-independent and shared. Narrow to the
        // source serving this route so a mounted source renders with its own
        // chrome here too (this path bypasses `render_section`).
        let templates = templates_for_route(template_map.clone(), route.as_str());
        let html = match render_section_template(section, &site_tree, templates, false).await {
            Ok(html) => html,
            Err(error) => {
                return Ok(Err(RenderError {
                    route: route.clone(),
                    error,
                }
                .into()));
            }
        };
        // Resolve relative links based on section route, then @/ links
        let html = resolve_relative_links(&html, route.as_str()).await;
        let html = resolve_internal_links(&html, &source_route_map).await;
        let resolved =
            resolve_wiki_links(&html, &wiki_link_index.resolved_for(route.as_str())).await;
        collect_wiki_link_errors(
            &mut unresolved_wiki_links,
            route,
            &resolved.unresolved_wiki_links,
            &wiki_link_index,
        );
        let html = resolved.html;
        pages.insert(route.clone(), html);
    }

    for (route, page) in &site_tree.pages {
        // Anonymous (`can_edit = false`): viewer-independent aggregate (font
        // subsetting). Narrow to the source serving this route for own-chrome.
        let templates = templates_for_route(template_map.clone(), route.as_str());
        let html = match render_page_template(page, &site_tree, templates, false).await {
            Ok(html) => html,
            Err(error) => {
                return Ok(Err(RenderError {
                    route: route.clone(),
                    error,
                }
                .into()));
            }
        };
        // Resolve relative links based on the page's section route, then @/ links
        let html = resolve_relative_links(&html, page.section_route.as_str()).await;
        let html = resolve_internal_links(&html, &source_route_map).await;
        let resolved =
            resolve_wiki_links(&html, &wiki_link_index.resolved_for(route.as_str())).await;
        collect_wiki_link_errors(
            &mut unresolved_wiki_links,
            route,
            &resolved.unresolved_wiki_links,
            &wiki_link_index,
        );
        let html = resolved.html;
        pages.insert(route.clone(), html);
    }

    if !unresolved_wiki_links.is_empty() {
        tracing::warn!(
            "{}",
            WikiLinkBuildError {
                errors: unresolved_wiki_links
            }
        );
    }

    Ok(Ok(AllRenderedHtml { pages }))
}

/// If a wiki-link target's leading name token (`kb` in `kb:overview`) is a
/// configured source whose content directory is absent — a sibling repo not
/// checked out — return that source, for the "not checked out" diagnostic.
fn absent_source_for_target(target: &str) -> Option<&'static crate::config::ResolvedSource> {
    let name = target
        .split([':', '/'])
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let sources = &crate::config::global_config()?.sources;
    sources
        .iter()
        .find(|s| s.name == name && !s.content_dir.exists())
}

fn collect_wiki_link_errors(
    errors: &mut Vec<WikiLinkError>,
    source_route: &Route,
    unresolved_links: &[cell_html_proto::WikiLinkRef],
    index: &WikiLinkIndex,
) {
    for link in unresolved_links {
        let reason = if let Some(candidates) = index.ambiguity(source_route.as_str(), &link.key) {
            WikiLinkErrorReason::Ambiguous {
                candidates: candidates.clone(),
            }
        } else if let Some(absent) = absent_source_for_target(&link.target) {
            WikiLinkErrorReason::SourceNotCheckedOut {
                source: absent.name.clone(),
                path: absent.content_dir.to_string(),
                git: absent.git.clone(),
            }
        } else {
            WikiLinkErrorReason::Missing
        };

        errors.push(WikiLinkError {
            route: source_route.clone(),
            target: link.target.clone(),
            reason,
        });
    }
}

/// Extract text content from HTML by stripping tags
///
/// This is a simple extraction that gets all visible text characters.
/// Used for font subsetting - we collect all characters used across the site.
fn extract_text_from_html(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut in_script_or_style = false;
    let mut tag_name = String::new();

    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            in_tag = true;
            tag_name.clear();
        } else if c == '>' && in_tag {
            in_tag = false;
            let tag_lower = tag_name.to_lowercase();
            if tag_lower.starts_with("script") || tag_lower.starts_with("style") {
                in_script_or_style = true;
            } else if tag_lower.starts_with("/script") || tag_lower.starts_with("/style") {
                in_script_or_style = false;
            }
        } else if in_tag {
            if !c.is_whitespace() && tag_name.len() < 20 {
                tag_name.push(c);
            }
        } else if !in_script_or_style {
            // Decode common HTML entities
            if c == '&' {
                let mut entity = String::new();
                while let Some(&next) = chars.peek() {
                    if next == ';' {
                        chars.next();
                        break;
                    }
                    if entity.len() > 10 {
                        // Not a valid entity, just output what we have
                        result.push('&');
                        result.push_str(&entity);
                        break;
                    }
                    entity.push(chars.next().unwrap());
                }
                match entity.as_str() {
                    "amp" => result.push('&'),
                    "lt" => result.push('<'),
                    "gt" => result.push('>'),
                    "quot" => result.push('"'),
                    "apos" => result.push('\''),
                    "nbsp" => result.push(' '),
                    s if s.starts_with('#') => {
                        // Numeric entity like &#39; or &#x27;
                        let num_str = &s[1..];
                        let code = if num_str.starts_with('x') || num_str.starts_with('X') {
                            u32::from_str_radix(&num_str[1..], 16).ok()
                        } else {
                            num_str.parse().ok()
                        };
                        if let Some(code) = code {
                            if let Some(ch) = char::from_u32(code) {
                                result.push(ch);
                            }
                        }
                    }
                    _ => {
                        // Unknown entity, skip it
                    }
                }
            } else {
                result.push(c);
            }
        }
    }
    result
}

/// Collect all unique characters used across the entire site
///
/// This is used for font subsetting - all fonts are subsetted to include
/// the same global character set. This is simpler than per-font analysis
/// and produces nearly identical results in practice.
#[picante::tracked]
pub async fn global_char_set<DB: Db>(db: &DB) -> PicanteResult<CharSet> {
    let all_html = all_rendered_html(db)
        .await?
        .expect("build errors should be caught before font analysis");

    let mut chars = std::collections::HashSet::new();

    // Extract text from all rendered HTML pages
    for html in all_html.pages.values() {
        let text = extract_text_from_html(html);
        for c in text.chars() {
            chars.insert(c);
        }
    }

    // Always include the Latin blocks (Basic Latin, Latin-1 Supplement, Latin
    // Extended-A) regardless of content. Subsetting exists to trim *enormous*
    // fonts (CJK / full-Unicode coverage); Latin is a rounding error by
    // comparison. Including it unconditionally keeps font subsetting purely on
    // the anonymous content render (no per-viewer coupling) while still covering
    // per-viewer UI chrome — e.g. the editor's "Edit"/"Save" text, which never
    // appears in that anonymous render — plus any Western-European text added
    // later. The subsetter simply ignores codepoints a given font lacks.
    for c in ('\u{20}'..='\u{7e}')
        .chain('\u{a0}'..='\u{ff}')
        .chain('\u{100}'..='\u{17f}')
    {
        chars.insert(c);
    }

    // Sort for deterministic output
    let mut sorted: Vec<char> = chars.into_iter().collect();
    sorted.sort();

    tracing::info!(
        num_chars = sorted.len(),
        "Collected global character set for font subsetting"
    );

    CharSet::new(db, sorted)
}

/// Compute the cache-busted path for a static file based on its SOURCE content.
/// This is used to build path maps without triggering rewriting, avoiding recursion.
#[picante::tracked]
pub async fn static_file_cache_path<DB: Db>(db: &DB, file: StaticFile) -> PicanteResult<String> {
    use crate::cache_bust::{cache_busted_path, content_hash, has_existing_hash};

    let path = file.path(db)?.as_str().to_string();

    // If file already has a hash (e.g. Vite output), use as-is
    if has_existing_hash(&path) {
        return Ok(path);
    }

    // Hash the source content
    let content = load_static(db, file).await?;
    let hash = content_hash(&content);
    Ok(cache_busted_path(&path, &hash))
}

/// Build a path map from original paths to cache-busted paths for all static files.
/// This uses source content hashing (no rewriting) to avoid recursion for CSS/JS.
/// For fonts, we use static_file_output to get the subsetted content hash.
#[picante::tracked]
pub async fn static_path_map<DB: Db>(db: &DB) -> PicanteResult<HashMap<String, String>> {
    let static_files = StaticRegistry::files(db)?.unwrap_or_default();
    let mut path_map = HashMap::new();

    for file in static_files.iter() {
        let original_path = file.path(db)?.as_str().to_string();
        // Skip images - they get transcoded to different formats with different hashing
        if !InputFormat::is_processable(&original_path) {
            // For fonts, use static_file_output to get the subsetted content hash
            // (fonts are subsetted based on character analysis, so we need the final hash)
            let cache_busted = if is_font_file(&original_path) {
                let output = static_file_output(db, *file).await?;
                output.cache_busted_path
            } else {
                static_file_cache_path(db, *file).await?
            };
            path_map.insert(format!("/{original_path}"), format!("/{cache_busted}"));
        }
    }

    Ok(path_map)
}

/// Process a single static file and return its cache-busted output
/// For fonts, this triggers global font analysis for subsetting
/// For CSS/JS files, URLs/string literals are rewritten to cache-busted versions
#[picante::tracked]
pub async fn static_file_output<DB: Db>(
    db: &DB,
    file: StaticFile,
) -> PicanteResult<StaticFileOutput> {
    use crate::cache_bust::{cache_busted_path, content_hash};

    let path = file.path(db)?.as_str().to_string();
    tracing::debug!(file_path = %path, "static_file_output: processing");
    // Get processed content based on file type
    let content = if is_font_file(&path) {
        // Font file - subset to global character set
        tracing::trace!(font_path = %path, "static_file_output: processing font file");
        let char_set = global_char_set(db).await?;

        if let Some(subsetted) = subset_font(db, file, char_set).await? {
            subsetted
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

        // Use pre-computed path map (no recursion needed)
        let path_map = static_path_map(db).await?;

        // Rewrite URLs in CSS
        let rewritten = rewrite_urls_in_css(&css_str, &path_map).await;
        rewritten.into_bytes()
    } else if path.to_lowercase().ends_with(".js") {
        // JS file - rewrite string literals to cache-busted versions
        let raw_content = load_static(db, file).await?;
        let js_str = String::from_utf8_lossy(&raw_content);

        // Use pre-computed path map (no recursion needed)
        let path_map = static_path_map(db).await?;

        // Rewrite string literals in JS
        let rewritten = rewrite_string_literals_in_js(&js_str, &path_map).await;
        rewritten.into_bytes()
    } else {
        // Other static files - just load
        load_static(db, file).await?
    };

    // Hash and create cache-busted path (unless already hashed by bundler)
    use crate::cache_bust::has_existing_hash;
    let cache_busted = if has_existing_hash(&path) {
        // File already has cache-busting hash (e.g. Vite's main-B6eUmL6x.js)
        path.clone()
    } else {
        let hash = content_hash(&content);
        cache_busted_path(&path, &hash)
    };

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

    // Use pre-computed path map (no recursion needed)
    let path_map = static_path_map(db).await?;

    // Rewrite URLs in CSS
    let rewritten_css = rewrite_urls_in_css(&css_content.0, &path_map).await;

    // Hash and create cache-busted path
    let hash = content_hash(rewritten_css.as_bytes());
    let cache_busted = cache_busted_path("main.css", &hash);

    Ok(Some(CssOutput {
        cache_busted_path: cache_busted,
        content: rewritten_css,
    }))
}

/// Build a map of original static file paths to their cache-busted URLs
/// This is used by the `get_static_url` template function
#[picante::tracked]
pub async fn static_url_map<DB: Db>(db: &DB) -> PicanteResult<HashMap<String, String>> {
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
    tracing::trace!(
        count = static_files.len(),
        "build_static_path_map: adding static files"
    );
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

    Ok(path_map)
}

/// The source that owns a route or source path — the one whose mount segment is
/// the longest prefix (the root source, segment ``, is the fallback). Used to
/// recover both the mount (for asset prefixing) and the name (for wiki links).
fn source_of(path: &str) -> Option<&'static crate::config::ResolvedSource> {
    let sources = &crate::config::global_config()?.sources;
    let trimmed = path.trim_matches('/');
    let mut best: Option<&'static crate::config::ResolvedSource> = None;
    let mut best_len: Option<usize> = None;
    for source in sources {
        let seg = source.mount.trim_matches('/');
        let matches = seg.is_empty()
            || trimmed == seg
            || trimmed
                .strip_prefix(seg)
                .is_some_and(|rest| rest.starts_with('/'));
        if matches && best_len.is_none_or(|bl| seg.len() > bl) {
            best = Some(source);
            best_len = Some(seg.len());
        }
    }
    best
}

/// The name of the source a route or source path belongs to (empty for the
/// root/degenerate source). Public so the search indexer can tag each page.
pub fn source_name_of(path: &str) -> String {
    source_of(path).map(|s| s.name.clone()).unwrap_or_default()
}

/// The mount segment a route belongs to (`spec/build` for `/spec/build/exec/`),
/// or `None` for the root mount. Used to alias source-root-absolute asset refs
/// (`/img/x`) to their mount-prefixed, cache-busted output.
fn page_mount_segment(route: &str) -> Option<String> {
    source_of(route)
        .map(|s| s.mount.trim_matches('/').to_string())
        .filter(|seg| !seg.is_empty())
}

/// Serve a single page or section with full URL rewriting and minification
/// This is the main entry point for lazy page serving
#[picante::tracked]
#[tracing::instrument(skip(db), name = "serve_html")]
pub async fn serve_html<DB: Db>(
    db: &DB,
    route: Route,
    can_edit: bool,
) -> PicanteResult<Result<Option<crate::db::ServedHtml>, SiteError>> {
    tracing::debug!(route = %route.as_str(), can_edit, "serve_html: rendering");
    use crate::url_rewrite::ResponsiveImageInfo;

    let site_tree = match build_tree(db).await? {
        Ok(tree) => tree,
        Err(errors) => return Ok(Err(BuildError { errors }.into())),
    };

    // Render THIS route's raw HTML directly — one route, not the whole site.
    // The route set and link indexes are parse-derived (build_tree /
    // source_to_route_map), so a single page resolves its own links without any
    // other page being rendered. `base_route` is the page's section route (the
    // route itself for a section); it's what relative links resolve against.
    let (head_injections, raw_html, base_route) = if site_tree.sections.contains_key(&route) {
        let head = site_tree
            .sections
            .get(&route)
            .unwrap()
            .head_injections
            .clone();
        let html = match render_section(db, route.clone(), can_edit).await? {
            Ok(RenderedHtml(html)) => html,
            Err(e) => return Ok(Err(e)),
        };
        (head, html, route.as_str().to_string())
    } else if site_tree.pages.contains_key(&route) {
        let page = site_tree.pages.get(&route).unwrap();
        let head = page.head_injections.clone();
        let base = page.section_route.as_str().to_string();
        let html = match render_page(db, route.clone(), can_edit).await? {
            Ok(RenderedHtml(html)) => html,
            Err(e) => return Ok(Err(e)),
        };
        (head, html, base)
    } else {
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

    // Add Vite manifest mappings (source → built output)
    // This allows templates to reference /src/main.ts and have it rewritten to /assets/main-xxx.js
    let vite_map = vite_manifest_map(db).await?;
    path_map.extend(vite_map);

    // Collect CSS that needs to be injected for Vite entry points
    let vite_css_map = vite_css_for_entries(db).await?;

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

    // Mount-aware asset aliases: a page from a mounted source (e.g.
    // `/spec/build/…`) may reference its own assets with source-root-absolute
    // paths (`/img/x`) — the form it would use when built standalone. Alias
    // those to the mount-prefixed, cache-busted entries so they resolve without
    // the author knowing the mount. We only alias assets the mounted source
    // actually has, so a real cross-source/shared `/img/x` still falls through.
    if let Some(seg) = page_mount_segment(route.as_str()) {
        let prefix = format!("/{seg}/");
        for (key, value) in path_map.clone() {
            if let Some(rest) = key.strip_prefix(&prefix) {
                path_map.insert(format!("/{rest}"), value);
            }
        }
        for (key, value) in image_variants.clone() {
            if let Some(rest) = key.strip_prefix(&prefix) {
                image_variants.insert(format!("/{rest}"), value);
            }
        }
    }

    // Mount-aware internal links: a mounted source authored its page links as
    // source-root-absolute (`/exec/`, `/exec/#anchor`, `/` for home) — the form
    // it would use standalone. The html cell rewrites the *path* portion of
    // such links to the mount-prefixed route (`/wiki/exec/`), preserving any
    // `#fragment`/`?query`, and only when the target resolves to one of the
    // source's own routes (so a genuine cross-source link falls through). This
    // is trailing-slash tolerant, which an exact path_map alias is not.
    let mount = page_mount_segment(route.as_str()).map(|segment| {
        let routes: std::collections::HashSet<String> = site_tree
            .sections
            .keys()
            .chain(site_tree.pages.keys())
            .map(|r| r.as_str().to_string())
            .collect();
        cell_html_proto::MountLocalization { segment, routes }
    });

    // Process HTML in a single html-cell pass:
    // - Resolve @/ internal links, [[wiki]] links, and relative links
    // - Inject CSS links for Vite entry points
    // - Rewrite URLs (transforms /src/monaco/main.ts -> /monaco.xxx.js)
    // - Transform <img> to <picture> for responsive images
    // The link indexes are parse-derived: source_to_route_map (memoized) and a
    // per-page wiki map from WikiLinkIndex over the already-built site_tree.
    let source_to_route = source_to_route_map(db).await?;
    let wiki_to_route = WikiLinkIndex::build(&site_tree).resolved_for(route.as_str());
    let process_options = crate::url_rewrite::HtmlProcessOptions {
        path_map: Some(path_map),
        vite_css_map: Some(vite_css_map),
        image_variants: Some(image_variants),
        source_to_route: Some(source_to_route),
        wiki_to_route: Some(wiki_to_route),
        base_route: Some(base_route),
        mount,
        ..Default::default()
    };

    let transformed_html = match crate::url_rewrite::process_html(&raw_html, process_options).await
    {
        Ok(output) => output.html,
        Err(e) => {
            tracing::warn!(route = %route, error = %e, "HTML processing failed, using raw HTML");
            raw_html.clone()
        }
    };
    let transformed_has_doctype = transformed_html.contains("<!DOCTYPE");

    // Minify HTML (but skip for error pages to preserve the error marker comment)
    let final_html = if raw_html.contains(crate::render::RENDER_ERROR_MARKER) {
        transformed_html
    } else {
        crate::svg::minify_html(&transformed_html).await
    };

    // Log if any step lost the DOCTYPE
    let raw_has_doctype = raw_html.contains("<!DOCTYPE");
    if raw_has_doctype && !final_html.contains("<!DOCTYPE") {
        tracing::error!(
            route = %route,
            raw_has_doctype,
            transformed_has_doctype,
            final_has_doctype = final_html.contains("<!DOCTYPE"),
            final_len = final_html.len(),
            final_preview = %final_html.chars().take(200).collect::<String>(),
            "DOCTYPE lost during HTML processing!"
        );
    }

    Ok(Ok(Some(crate::db::ServedHtml {
        html: final_html,
        head_injections,
    })))
}

/// Check if a path is a font file
fn is_font_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".ttf")
        || lower.ends_with(".otf")
        || lower.ends_with(".woff")
        || lower.ends_with(".woff2")
}

// ============================================================================
// Code execution integration
// ============================================================================

/// Execute code samples from all source files and return results
/// This is called during the build process to validate code samples
pub async fn execute_all_code_samples<DB: Db>(db: &DB) -> PicanteResult<Vec<CodeExecutionResult>> {
    use crate::cells::{execute_code_samples, extract_code_samples};

    let mut all_results = Vec::new();

    // Create default configuration for code execution
    let config = cell_code_execution_proto::CodeExecutionConfig::default();

    // Extract and execute code samples from all source files
    let sources = SourceRegistry::sources(db)?.unwrap_or_default();
    for source in sources.iter() {
        let content = source.content(db)?;
        let source_path = source.path(db)?.as_str().to_string();

        // Extract code samples from this source file
        let extract_result = extract_code_samples(cell_code_execution_proto::ExtractSamplesInput {
            source_path: source_path.clone(),
            content: content.as_str().to_string(),
        })
        .await;

        let samples = match extract_result {
            Ok(cell_code_execution_proto::CodeExecutionResult::ExtractSuccess { output }) => {
                output.samples
            }
            Ok(cell_code_execution_proto::CodeExecutionResult::Error { message }) => {
                tracing::warn!(
                    "Failed to extract code samples from {}: {}",
                    source_path,
                    message
                );
                continue;
            }
            Ok(_) => continue,
            Err(e) => {
                tracing::warn!("Code sample extraction error from {}: {}", source_path, e);
                continue;
            }
        };

        if !samples.is_empty() {
            tracing::debug!("Found {} code samples in {}", samples.len(), source_path);

            // Execute the code samples
            let execute_result =
                execute_code_samples(cell_code_execution_proto::ExecuteSamplesInput {
                    samples,
                    config: config.clone(),
                })
                .await;

            let execution_results = match execute_result {
                Ok(cell_code_execution_proto::CodeExecutionResult::ExecuteSuccess { output }) => {
                    output.results
                }
                Ok(cell_code_execution_proto::CodeExecutionResult::Error { message }) => {
                    tracing::warn!(
                        "Failed to execute code samples from {}: {}",
                        source_path,
                        message
                    );
                    continue;
                }
                Ok(_) => continue,
                Err(e) => {
                    tracing::warn!("Code sample execution error from {}: {}", source_path, e);
                    continue;
                }
            };

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

/// Check a single external URL and return its status.
/// Cached by (url, day_bucket) - same URL on same day returns cached result.
/// Day bucket is YYYYMMDD as u32 (e.g., 20260116).
#[picante::tracked]
#[tracing::instrument(skip(db), name = "check_external_url")]
pub async fn check_external_url<DB: Db>(
    db: &DB,
    url: String,
    day_bucket: u32,
) -> PicanteResult<ExternalLinkStatus> {
    // Ignore db and day_bucket for the actual check - they're just for caching
    let _ = db;
    let _ = day_bucket;

    use crate::cells::{CheckOptions, check_urls};
    use cell_linkcheck_proto::LinkStatus;

    let options = CheckOptions {
        rate_limit_ms: 100, // Small delay between requests to same domain
        timeout_secs: 10,
    };

    let result = check_urls(vec![url.clone()], options).await;

    match result {
        Some(check_result) => match check_result.statuses.get(&url) {
            Some(LinkStatus::Ok) => Ok(ExternalLinkStatus::Ok),
            Some(LinkStatus::Skipped) => Ok(ExternalLinkStatus::Ok),
            Some(LinkStatus::HttpError { code, diagnostics }) => {
                Ok(ExternalLinkStatus::HttpError {
                    code: *code,
                    diagnostics: crate::db::HttpErrorDiagnostics {
                        request_headers: diagnostics.request_headers.clone(),
                        response_headers: diagnostics.response_headers.clone(),
                        response_body: diagnostics.response_body.clone(),
                    },
                })
            }
            Some(LinkStatus::Failed { message }) => Ok(ExternalLinkStatus::Failed(message.clone())),
            None => Ok(ExternalLinkStatus::Failed("URL not in results".to_string())),
        },
        None => Ok(ExternalLinkStatus::Failed("link check failed".to_string())),
    }
}

#[cfg(test)]
mod wiki_suffix_tests {
    use super::route_path_suffixes;

    #[test]
    fn yields_multi_segment_suffixes_longest_first() {
        assert_eq!(
            route_path_suffixes("/spec/build/overview/"),
            vec!["spec/build/overview", "build/overview"]
        );
    }

    #[test]
    fn single_segment_route_has_no_qualifiers() {
        assert!(route_path_suffixes("/overview/").is_empty());
        assert!(route_path_suffixes("/").is_empty());
    }

    #[test]
    fn two_segment_route_yields_only_the_full_path() {
        assert_eq!(
            route_path_suffixes("/spec/overview/"),
            vec!["spec/overview"]
        );
    }
}
