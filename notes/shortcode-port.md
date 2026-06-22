# Shortcode port: home → dodeca

Design note for bringing fasterthanli.me's shortcodes to dodeca. Part of the larger
migration off the custom `home` CMS (`~/cove`) onto dodeca. Gated content and
identity/Discord are **out of scope** here (separate effort). This note covers
shortcodes only — the agreed first workstream.

## What we're replicating

`home`'s `libmarkdown` (`~/cove/crates/libmarkdown/src/impls.rs`) supports two shortcode
grammars, both built on the **same pulldown-cmark 0.13** marq uses — detection lives in
the *event handler*, not the lexer, so no grammar/lexer changes are needed.

Usage in `~/fasterthanli.me/content` (285 md files, ~4500 invocations):

| Grammar | Shortcodes (count) |
|---|---|
| **Body** `*:name(args)*` + blockquote body | bearsays 1966, amossays 732, tip 686, recap 68, disclosure 12 |
| **Fenced** `+++ \n :name: \n <yaml args> \n +++` (no body) | figure 191, media 152, youtube 40, wasmres 10, wasm 2, others |

Templates live at `~/fasterthanli.me/templates/shortcodes/<name>.html.jinja`. They use
custom filters/functions: `get_media(src).markup(...)`, `basic_markdown`,
`escape_for_attribute`, `import "macros.html"`.

### Grammar 1 — fenced `:name:` (body: None)
`impls.rs:631-687`. A `MetadataBlock(PlusesStyle)` whose YAML is a single mapping with a
key starting with `:`. Key (minus colon) = shortcode name; value = args object.
Render template with args, **then re-parse the rendered output as nested markdown**.

### Grammar 2 — body `*:name(...)*` (body: Some)
`impls.rs:739-799` + `argparse.rs::parse_emphasis_shortcode`. An `Emphasis` span whose
text starts with `:`. Parse `name(key=value, ...)`. Render template passing
`body: Some("___BODY_MARKER___")`; split rendered output on the marker into
before/after; emit `before`, render the blockquote body markdown in between, emit `after`.
This is the speech-bubble pattern (`tip.html` wraps `{{ body }}` deep inside its HTML).

## dodeca-side design — PASSTHROUGH, not bake-in (this is the picante-correct path)

**Critical constraint (Amos): dependency tracking must not break.** The wrong design is
"marq RPCs gingembre and bakes shortcode HTML inline" — that hides the template/asset
reads from picante. The right design follows dodeca's existing pattern for `@/` and wiki
links: **marq stays dependency-agnostic and emits opaque passthrough tokens; dodeca
resolves them inside its picante-tracked render pass.**

Evidence this is the house pattern:
- `TemplateFile` / `TemplateRegistry` are `#[picante::input]` (`db.rs:42-80`), keyed by
  path; shortcode templates under `templates/shortcodes/` are already in this input set.
- Page render reads templates via picante queries (`render.rs:664 try_render_template`),
  so editing a template invalidates exactly the pages that read it.
- marq's `PassthroughLinkResolver` (cell-markdown `main.rs`) keeps `@/`/wiki links as
  tokens: *"dodeca will resolve them with site tree access … track dependencies via
  picante."* `template_host.rs::resolve_data` / `get_url` / `get_section` already register
  picante deps when called.
- `home` did the manual version: threaded `render_res.shortcode_input_path` +
  `assets_looked_up` into `result.deps`. We get the same guarantee for free via picante
  input reads.

**Layer 1 — `marq`** (`~/oss/marq`, branch off `release/marq-5.0.0-rc.0`)
- Detect both grammars in `render.rs` (no lexer changes):
  - Pluses metadata block NOT at document start with a `:name` key → fenced shortcode
    (hook at `render.rs:1168`, currently TOML frontmatter).
  - `Emphasis` whose first text starts with `:` → body shortcode.
- Emit a **passthrough token** per invocation: name + args (parsed) + already-rendered
  **body HTML** (marq renders the nested markdown body — that part has no external deps).
  Pluggable via a `ShortcodeResolver` trait returning `None` by default (like
  `LinkResolver`), so marq itself bakes nothing.
- Args: YAML for fenced, `key=value` parse for emphasis → serde-free data object (facet).

> Transport note: cells are **in-process** now (`cells.rs` = "in-process processing
> facade for the former dodeca cells"); `cell-markdown` is a library called directly
> (`MarkdownProcessorImpl::parse_and_render`), no roam/RPC/SHM. vox survives only at the
> server↔browser websocket boundary (devtools/livereload/`cell-http`), not between cells.
> So marq↔dodeca shortcode resolution is a direct call — passthrough is for picante
> correctness, not for any boundary.

**Layer 2 — dodeca render pass** (`~/oss/dodeca/crates/dodeca/src`, NOT inside marq)
- Resolve shortcode tokens in the **same picante-tracked pass** that resolves `@/`/wiki
  links. For each token: load `templates/shortcodes/<name>.html` (a `TemplateFile` input
  read → tracked) and render via gingembre with `{ args, body }` context. Any
  `get_media()`/`get_url()` the template calls goes through the already-tracked
  template-host functions → asset deps tracked automatically.

**Layer 3 — gingembre/template-host**
- Port custom filters/functions the shortcodes need: `get_media().markup()`,
  `basic_markdown`, `escape_for_attribute`. (Also feeds the gingembre-improvements
  workstream.) These resolve assets through the picante-tracked host, preserving tracking.

### Dependency-tracking guarantees (by construction)
- Edit `tip.html.jinja` → only pages using `tip` re-render (TemplateFile input read).
- Replace an image referenced via `get_media()` in a shortcode → using pages rebuild.
- marq adds no bespoke dep bookkeeping; it stays dep-agnostic exactly like links.

## Oracle / tests

home's snapshot tests are the oracle:
`~/cove/crates/libmarkdown/src/snapshots/*shortblocks*`, `*markdown_with_figure*`.
Port representative fixtures into marq's snapshot suite and match output.

## Related workstreams (not this note)
- **gingembre improvements**: `{%- -%}` whitespace control, `loop.index/first/last`,
  `{% raw %}`, `{% filter %}`, missing filters (`tojson`, `truncate`, `urlencode`).
- **dodeca additions**: taxonomies/tags pages, series prev/next, RSS/atom, sitemap,
  redirects/aliases, first-class `date`/`draft`.
- **identity → Discord**: separate small service. Out of scope.
