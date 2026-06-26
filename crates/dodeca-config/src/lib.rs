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

/// Dodeca configuration from `.config/dodeca.styx`.
///
/// Two sections carry the meaning, so you can see at a glance what travels with
/// a source vs. what belongs to the assembled site:
///
/// - [`source`](Self::source) — **composable**, source-scoped. When this content
///   is mounted into an aggregator, the aggregator adopts this block (its
///   `impls`, `page_types`, …) re-namespaced under the mount.
/// - [`site`](Self::site) — **not composable**, whole-site. When mounted, this is
///   dropped; the aggregator's `site` is authoritative.
/// - [`mounts`](Self::mounts) — additional sub-sources, each at a non-root URL
///   `path`, composing that source's `source {}` (read from its own config).
///
/// The top-level `source` is the content served at `/`; `mounts` add sources
/// beneath it (so a mount `path` may not be `/`). A leaf project sets `source` +
/// `site`; an aggregator adds `mounts`. At least one of `source` / `mounts` must
/// be present.
#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "snake_case")]
pub struct DodecaConfig {
    /// Composable, source-scoped config — present in a leaf project.
    #[facet(default)]
    pub source: Option<SourceConfig>,

    /// Whole-site config (output, base URL, link checking, code execution, …).
    /// Always the assembling site's; never composed from a mounted source.
    ///
    /// Optional in the schema so a *mount-only* sub-config (composed solely for
    /// its `source {}`) needn't carry a `site` it would never use. The config
    /// actually being built must have one — that's enforced at resolve time.
    #[facet(default)]
    pub site: Option<SiteConfig>,

    /// Aggregator: content sources merged into one site, each mounted under a
    /// URL `path`, composing that source's `source {}`.
    #[facet(default)]
    pub mounts: Option<Vec<MountDef>>,
}

/// Composable, source-scoped configuration: what a content collection *is* and
/// how to render / validate / execute it. Chrome (`templates`/`sass`/`static`)
/// is resolved by directory convention and so isn't listed here.
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct SourceConfig {
    /// Content directory, relative to the source's own root. Used when the
    /// source is built standalone; when mounted, the `mounts` entry's location
    /// is authoritative instead.
    #[facet(default)]
    pub content: Option<String>,

    /// Browsable repository URL (e.g.
    /// `https://github.com/facet-rs/facet/tree/main/figue`), exposed to
    /// templates for "view source" links. Travels with the source.
    #[facet(default)]
    pub repo: Option<String>,

    /// Code implementations whose source files are scanned for `r[verb rule.id]`
    /// references to compute coverage of this source's spec rules.
    #[facet(default)]
    pub impls: Vec<ImplDef>,

    /// First-class frontmatter schemas keyed by page type.
    #[facet(default, alias = "page-types")]
    pub page_types: Option<HashMap<String, PageTypeSchema>>,

    /// Build steps — parameterized commands invoked from this source's templates.
    #[facet(default)]
    pub build_steps: Option<HashMap<String, BuildStepDef>>,

    /// Domains to skip when link-checking this source's external links
    /// (anti-bot, known-flaky). Unioned into the assembled site's link check.
    #[facet(default)]
    pub skip_domains: Vec<String>,
}

/// Whole-site configuration: properties of the assembled, published site. Exactly
/// one applies to a build — the standalone leaf's, or the aggregator's. Never
/// composed from a mounted source.
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct SiteConfig {
    /// Output directory (relative to project root).
    pub output: String,

    /// Base URL for the site (e.g. `https://example.com`); permalinks. For a
    /// mounted source, the URL prefix comes from its `path`, not its `base_url`.
    #[facet(default)]
    pub base_url: Option<String>,

    /// Link checking policy for the assembled site (`mode`, `rate_limit_ms`).
    /// Per-source `skip_domains` are unioned in on top.
    #[facet(default)]
    pub link_check: Option<LinkCheckConfig>,

    /// Assets served at their original paths (no cache-busting): favicon.svg,
    /// robots.txt, og-image.png. One set for the whole site.
    #[facet(default)]
    pub stable_assets: Option<Vec<String>>,

    /// Code execution (per-language sub-configs), for the whole site — the site
    /// owns the policy of whether/how code samples run.
    #[facet(default)]
    pub code_execution: Option<CodeExecutionConfig>,

    /// Syntax highlighting theme, for visual consistency across the site.
    #[facet(default)]
    pub syntax_highlight: Option<SyntaxHighlightConfig>,

    /// Authentication (oauth2-proxy / Forgejo OIDC). Present → `/_dodeca/*` is
    /// gated; absent → open (local `ddc serve`).
    #[facet(default)]
    pub auth: Option<AuthConfig>,
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

/// One sub-source mounted into an aggregator at a non-root URL `path`.
///
/// The aggregator's *own* root content is the top-level [`source`](DodecaConfig::source)
/// at `/`; `mounts` are the additional sources beneath it, so a mount `path` may
/// **not** be `/`. The entry supplies the source's **location** and namespace;
/// its **behavior** (`impls`, `page_types`, …) is composed from the source's own
/// `source {}`, read from a `.config/dodeca.styx` at-or-above its content dir.
///
/// Example in `.config/dodeca.styx`:
/// ```styx
/// mounts (
///   {name vox   path /vox        local vox/docs/content}
///   {name build path /spec/build checkout ../vixen content docs/content
///                git code.vixen.rs/vixen/vixen.git}
/// )
/// ```
///
/// The location is either **local** (`local` = the content dir, no repo) or
/// **git-backed** (`checkout` = a repo dir to clone/pull, `content` = the content
/// path within it, `git` = the remote to clone if absent). Exactly one of
/// `local` / `checkout`.
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct MountDef {
    /// Stable identity of this source, used to link to it from other sources
    /// (`[[<name>:slug]]`) and to label its search hits — independent of where
    /// it is mounted. Required.
    pub name: String,

    /// URL namespace this source mounts under, e.g. `/spec/build`. May not be
    /// `/` — the root is the aggregator's own top-level `source`.
    pub path: String,

    /// Direct content directory (relative to the aggregator root). Mutually
    /// exclusive with `checkout`.
    #[facet(default)]
    pub local: Option<String>,

    /// Repo checkout directory — the stable location cloned/pulled by the
    /// service (relative to the aggregator root, e.g. `../vixen`). The content is
    /// `content` *within* this dir. Mutually exclusive with `local`.
    #[facet(default)]
    pub checkout: Option<String>,

    /// Content path *within* `checkout` (e.g. `docs/content`). Defaults to the
    /// checkout root. Only meaningful with `checkout`.
    #[facet(default)]
    pub content: Option<String>,

    /// Remote to `git clone` into `checkout` when it's absent on disk, and to
    /// `git pull` from on a webhook/poll. Only meaningful with `checkout`.
    #[facet(default)]
    pub git: Option<String>,

    /// Browsable repository URL for "view source" links, e.g.
    /// `https://github.com/facet-rs/facet`. An override: when set it wins;
    /// otherwise `repo` composes from the mounted source's own `source {}`. Lets
    /// a same-monorepo / vendored mount (which has no config of its own to
    /// compose from) still carry a view-source URL.
    #[facet(default)]
    pub repo: Option<String>,
}

/// One implementation of a source's spec: a named set of code files scanned for
/// requirement references. Mirrors tracey's `impl` block, attached to a dodeca
/// source so editing it hot-reloads coverage through the config input.
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct ImplDef {
    /// Name of this implementation (e.g. `rust`, `core`, `frontend`).
    pub name: String,

    /// Glob patterns for source files to scan, relative to the project root.
    #[facet(default)]
    pub include: Vec<String>,

    /// Glob patterns to exclude from `include`.
    #[facet(default)]
    pub exclude: Vec<String>,

    /// Glob patterns for test files. References in these files may only
    /// *verify* a rule, never *implement* it.
    #[facet(default)]
    pub test_include: Vec<String>,
}

/// Authentication / authorization config. Its mere presence turns on gating of
/// `/_dodeca/*` behind a forwarded identity (oauth2-proxy). Editing is
/// **fail-closed**: a user may edit only if listed in `editors` or a member of
/// an `editor_groups` group — no allowlist means no one edits.
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct AuthConfig {
    /// Forgejo groups whose members may edit (matched against forwarded groups).
    #[facet(default)]
    pub editor_groups: Option<Vec<String>>,
    /// Explicit user allowlist for editing (matched against the forwarded user).
    #[facet(default)]
    pub editors: Option<Vec<String>>,
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
