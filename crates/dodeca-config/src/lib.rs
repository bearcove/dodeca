//! Configuration types for dodeca static site generator.
//!
//! This crate contains the configuration structs that are parsed from
//! `.config/dodeca.styx`.

use std::collections::HashMap;

use facet::Facet;

// Re-export code execution config
pub use cell_code_execution_proto::CodeExecutionConfig;
// Re-export Schema for build step param types
pub use facet_styx::Schema;
use facet_styx::{
    DefaultSchema, DeprecatedSchema, Documented, FloatConstraints, IntConstraints, ObjectKey,
    RawStyx, StringConstraints,
};

/// Dodeca configuration from `.config/dodeca.styx`
#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "snake_case")]
pub struct DodecaConfig {
    /// Base URL for the site (e.g., `https://example.com`)
    /// Used to generate permalinks. Defaults to "/" for local development.
    #[facet(default)]
    pub base_url: Option<String>,

    /// Content directory (relative to project root). A leaf project sets this;
    /// it is equivalent to a single source mounted at `/`. Aggregator configs
    /// omit it and use `sources` instead. Exactly one of `content` / `sources`
    /// must be present.
    #[facet(default)]
    pub content: Option<String>,

    /// Output directory (relative to project root)
    pub output: String,

    /// Multiple content sources merged into one site, each mounted under a URL
    /// prefix. When present, `content` must be omitted (and vice versa). This is
    /// what lets one Dodeca site assemble several repos (the KB plus specs).
    #[facet(default)]
    pub sources: Option<Vec<SourceDef>>,

    /// Link checking configuration
    #[facet(default)]
    pub link_check: Option<LinkCheckConfig>,

    /// Assets that should be served at their original paths (no cache-busting)
    /// e.g., favicon.svg, robots.txt, og-image.png
    #[facet(default)]
    pub stable_assets: Option<Vec<String>>,

    /// Code execution configuration
    #[facet(default)]
    pub code_execution: Option<CodeExecutionConfig>,

    /// Syntax highlighting theme configuration
    #[facet(default)]
    pub syntax_highlight: Option<SyntaxHighlightConfig>,

    /// Build steps - parameterized commands invoked from templates.
    /// Keys are step names, values define params and command.
    #[facet(default)]
    pub build_steps: Option<HashMap<String, BuildStepDef>>,

    /// First-class frontmatter schemas keyed by page type.
    #[facet(default, alias = "page-types")]
    pub page_types: Option<HashMap<String, PageTypeSchema>>,
}

/// A frontmatter schema type.
///
/// This mirrors `facet_styx::Schema`, while adding Dodeca's `@link(@PageType)`
/// constructor for typed cross-page references.
#[derive(Facet, Debug, Clone)]
#[facet(rename_all = "lowercase")]
#[repr(u8)]
pub enum PageTypeSchema {
    String(Option<StringConstraints>),
    Int(Option<IntConstraints>),
    Float(Option<FloatConstraints>),
    Bool,
    Unit,
    Any,
    Object(PageObjectSchema),
    Seq(PageSeqSchema),
    Tuple(PageTupleSchema),
    Map(PageMapSchema),
    Union(PageUnionSchema),
    Optional(PageOptionalSchema),
    Enum(PageEnumSchema),
    #[facet(rename = "one-of")]
    OneOf(PageOneOfSchema),
    Flatten(PageFlattenSchema),
    Default(PageDefaultSchema),
    Deprecated(PageDeprecatedSchema),
    Link(PageLinkSchema),
    Literal(String),
    #[facet(other)]
    Type {
        #[facet(tag)]
        name: Option<String>,
    },
}

#[derive(Facet, Debug, Clone)]
#[repr(transparent)]
pub struct PageObjectSchema(pub HashMap<Documented<ObjectKey>, PageTypeSchema>);

#[derive(Facet, Debug, Clone)]
#[repr(transparent)]
pub struct PageSeqSchema(pub (Documented<Box<PageTypeSchema>>,));

#[derive(Facet, Debug, Clone)]
#[repr(transparent)]
pub struct PageTupleSchema(pub Vec<Documented<PageTypeSchema>>);

#[derive(Facet, Debug, Clone)]
#[repr(transparent)]
pub struct PageMapSchema(pub Vec<Documented<PageTypeSchema>>);

#[derive(Facet, Debug, Clone)]
#[repr(transparent)]
pub struct PageUnionSchema(pub Vec<Documented<PageTypeSchema>>);

#[derive(Facet, Debug, Clone)]
#[repr(transparent)]
pub struct PageOptionalSchema(pub (Documented<Box<PageTypeSchema>>,));

#[derive(Facet, Debug, Clone)]
#[repr(transparent)]
pub struct PageEnumSchema(pub HashMap<Documented<String>, PageTypeSchema>);

#[derive(Facet, Debug, Clone)]
#[repr(transparent)]
pub struct PageOneOfSchema(pub (Documented<Box<PageTypeSchema>>, Vec<RawStyx>));

#[derive(Facet, Debug, Clone)]
#[repr(transparent)]
pub struct PageFlattenSchema(pub (Documented<Box<PageTypeSchema>>,));

#[derive(Facet, Debug, Clone)]
#[repr(transparent)]
pub struct PageDefaultSchema(pub (RawStyx, Documented<Box<PageTypeSchema>>));

#[derive(Facet, Debug, Clone)]
#[repr(transparent)]
pub struct PageDeprecatedSchema(pub (String, Documented<Box<PageTypeSchema>>));

#[derive(Facet, Debug, Clone)]
#[repr(transparent)]
pub struct PageLinkSchema(pub (Documented<Box<PageTypeSchema>>,));

impl PageTypeSchema {
    /// Lower this Dodeca schema to a plain Styx schema for structural validation.
    pub fn to_styx_schema(&self) -> Schema {
        match self {
            PageTypeSchema::String(c) => Schema::String(c.clone()),
            PageTypeSchema::Int(c) => Schema::Int(c.clone()),
            PageTypeSchema::Float(c) => Schema::Float(c.clone()),
            PageTypeSchema::Bool => Schema::Bool,
            PageTypeSchema::Unit => Schema::Unit,
            PageTypeSchema::Any => Schema::Any,
            PageTypeSchema::Object(schema) => Schema::Object(facet_styx::ObjectSchema(
                schema
                    .0
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_styx_schema()))
                    .collect(),
            )),
            PageTypeSchema::Seq(schema) => Schema::Seq(facet_styx::SeqSchema((
                documented_box_to_styx(&schema.0.0),
            ))),
            PageTypeSchema::Tuple(schema) => Schema::Tuple(facet_styx::TupleSchema(
                schema.0.iter().map(documented_schema_to_styx).collect(),
            )),
            PageTypeSchema::Map(schema) => Schema::Map(facet_styx::MapSchema(
                schema.0.iter().map(documented_schema_to_styx).collect(),
            )),
            PageTypeSchema::Union(schema) => Schema::Union(facet_styx::UnionSchema(
                schema.0.iter().map(documented_schema_to_styx).collect(),
            )),
            PageTypeSchema::Optional(schema) => Schema::Optional(facet_styx::OptionalSchema((
                documented_box_to_styx(&schema.0.0),
            ))),
            PageTypeSchema::Enum(schema) => Schema::Enum(facet_styx::EnumSchema(
                schema
                    .0
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_styx_schema()))
                    .collect(),
            )),
            PageTypeSchema::OneOf(schema) => Schema::OneOf(facet_styx::OneOfSchema((
                documented_box_to_styx(&schema.0.0),
                schema.0.1.clone(),
            ))),
            PageTypeSchema::Flatten(schema) => Schema::Flatten(facet_styx::FlattenSchema((
                documented_box_to_styx(&schema.0.0),
            ))),
            PageTypeSchema::Default(schema) => Schema::Default(DefaultSchema((
                schema.0.0.clone(),
                documented_box_to_styx(&schema.0.1),
            ))),
            PageTypeSchema::Deprecated(schema) => Schema::Deprecated(DeprecatedSchema((
                schema.0.0.clone(),
                documented_box_to_styx(&schema.0.1),
            ))),
            PageTypeSchema::Link(_) => Schema::String(None),
            PageTypeSchema::Literal(value) => Schema::Literal(value.clone()),
            PageTypeSchema::Type { name } => Schema::Type { name: name.clone() },
        }
    }

    pub fn link_target_type(&self) -> Option<&str> {
        let PageTypeSchema::Link(link) = self else {
            return None;
        };
        match link.0.0.value.as_ref() {
            PageTypeSchema::Type { name: Some(name) } => Some(name.as_str()),
            _ => None,
        }
    }
}

fn documented_box_to_styx(value: &Documented<Box<PageTypeSchema>>) -> Documented<Box<Schema>> {
    Documented {
        value: Box::new(value.value.to_styx_schema()),
        doc: value.doc.clone(),
    }
}

fn documented_schema_to_styx(value: &Documented<PageTypeSchema>) -> Documented<Schema> {
    Documented {
        value: value.value.to_styx_schema(),
        doc: value.doc.clone(),
    }
}

/// A single content source mounted into the site at a URL prefix.
///
/// A leaf project omits `sources` and uses the top-level `content`; that is
/// equivalent to one source mounted at `/`. An aggregator config (e.g. the
/// site repo) lists several sources, each pointing at a content directory — a
/// sibling repo checkout — and mounted under a URL namespace.
///
/// Example in `.config/dodeca.styx`:
/// ```styx
/// sources {
///   { name kb     mount /            local content }
///   { name build  mount /spec/build  local ../vixen/docs/content }
/// }
/// ```
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct SourceDef {
    /// Stable identity of this source, used to link to it from other sources
    /// (`[[<name>:slug]]`) and to label its search hits — independent of where
    /// it is mounted. A source served at `/` still has a name. Required.
    pub name: String,

    /// URL namespace this source mounts under, e.g. `/` or `/spec/build`.
    pub mount: String,

    /// Path to this source's content directory, relative to the project root
    /// (e.g. `content`, or a sibling path like `../vixen/docs/content`). When
    /// absent the source is only reachable via `git`, which is not yet
    /// implemented — the resolver rejects a source with neither.
    #[facet(default)]
    pub local: Option<String>,

    /// Remote + ref to fetch when a local sibling checkout is absent (e.g. on a
    /// deploy). Carried in the schema for forward-compatibility; resolution is
    /// deferred to the render-as-a-service work, so the resolver currently
    /// errors on a git-only source rather than silently producing nothing.
    #[facet(default)]
    pub git: Option<String>,
}

/// Syntax highlighting theme configuration
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct SyntaxHighlightConfig {
    /// Light theme name (e.g., "github-light", "catppuccin-latte")
    #[facet(default)]
    pub light_theme: Option<String>,

    /// Dark theme name (e.g., "tokyo-night", "catppuccin-mocha")
    #[facet(default)]
    pub dark_theme: Option<String>,
}

/// What to check.
///
/// `Full` (default) walks every internal link and probes every external one;
/// `Internal` skips external HTTP probes (fast, no network); `None` skips
/// link checking entirely. Set via `link_check.mode` in
/// `.config/dodeca.styx` or `--link-check` on the CLI (CLI wins).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Facet)]
#[facet(rename_all = "snake_case")]
#[repr(u8)]
pub enum LinkCheckMode {
    None,
    Internal,
    #[default]
    Full,
}

/// Link checking configuration
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct LinkCheckConfig {
    /// What to check. Defaults to `full`. Override at the CLI with
    /// `ddc build --link-check none|internal|full`.
    #[facet(default)]
    pub mode: Option<LinkCheckMode>,

    /// Domains to skip checking (anti-bot policies, known flaky, etc.)
    #[facet(default)]
    pub skip_domains: Option<Vec<String>>,

    /// Minimum delay between requests to the same domain (milliseconds)
    /// Default: 1000ms (1 second)
    #[facet(default)]
    pub rate_limit_ms: Option<u64>,
}

/// A build step definition.
///
/// Build steps are parameterized commands that can be invoked from templates.
/// Parameters can be typed (e.g., `@file`, `@int`, `@string`) and `@file` params
/// are tracked for caching - the step re-runs when file contents change.
///
/// Example in `.config/dodeca.styx`:
/// ```styx
/// build_steps {
///   styx_to_json {
///     params {
///       file @file
///     }
///     command (styx --json "{file}")
///   }
/// }
/// ```
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct BuildStepDef {
    /// Typed parameters for this build step.
    /// Keys are parameter names, values are Styx schema types.
    /// Use `@file` for file paths that should be tracked for caching.
    #[facet(default)]
    pub params: Option<HashMap<String, Schema>>,

    /// Command to execute as a sequence of arguments.
    /// Use `{param_name}` for interpolation.
    /// If absent, the step reads the file specified by the first `@file` param.
    #[facet(default)]
    pub command: Option<Vec<String>>,
}

impl BuildStepDef {
    /// Check if a parameter is a tracked file type.
    pub fn is_file_param(&self, param_name: &str) -> bool {
        self.params
            .as_ref()
            .and_then(|p| p.get(param_name))
            .map(|schema| matches!(schema, Schema::Type { name: Some(n) } if n == "file"))
            .unwrap_or(false)
    }

    /// Get all file-typed parameter names.
    pub fn file_params(&self) -> Vec<&str> {
        self.params
            .as_ref()
            .map(|p| {
                p.iter()
                    .filter(|(_, schema)| {
                        matches!(schema, Schema::Type { name: Some(n) } if n == "file")
                    })
                    .map(|(name, _)| name.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }
}
