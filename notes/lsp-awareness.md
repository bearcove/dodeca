# Authoring LSP awareness: shortcodes + frontmatter

Design note for teaching `crates/dodeca-authoring-lsp` about the new shortcode
grammars and the `+++`/`---` frontmatter ambiguity. Companion to
`notes/shortcode-port.md` and `notes/migration-backlog.md` (workstream 7b).

---

## 1. Current state

### crates/dodeca-authoring-lsp/src/authoring_lsp.rs (~9 700 lines)

The LSP implements `tower_lsp::LanguageServer` via a `Backend` / `AuthoringState`
pair. `AuthoringState` holds an `AuthoringWorkspace` (disk snapshot) and a
`world_cache: Option<CachedAuthoringWorld>` that rebuilds on file changes.
`AuthoringWorld` bundles:
- `project: AuthoringProject` — pages, routes, templates, source contents
- `template_index: TemplateAuthoringIndex` — gingembre semantic indices
- `content_graph: ContentAuthoringGraph` — route graph
- `source_document_targets` — per-page frontmatter document link targets

Features today:

| Feature | Entry point | Scope |
|---|---|---|
| Completion | `completions()` | frontmatter fields, `@/` source paths, routes, static files, template gingembre symbols |
| Hover | `hover_for_position()` | frontmatter block, markdown links/images, template blocks/vars |
| Diagnostics | `diagnostics_for_page()` / `load_authoring_diagnostics_for_world()` | frontmatter validation, broken links/images/routes, duplicate routes/titles, orphaned pages |
| Document links | `document_links()` | markdown `[text](url)` targets, template `{% include/extends/import %}` paths |
| Semantic tokens | `semantic_tokens_for_document()` | templates only (gingembre vars, macros, filters) |
| Go-to-definition / references / rename | multiple | routes in markdown and templates |
| Code actions | `code_actions()` | create frontmatter block |

### crates/dodeca/src/authoring_model.rs

`AuthoringProject` is the central data bag built by
`build_authoring_project_on_db()`. Relevant fields:

```
pages: Vec<AuthoringPage>
source_contents: HashMap<String, String>          // source path → raw markdown
template_paths: HashMap<String, Utf8PathBuf>      // logical key → physical path
template_contents: HashMap<String, String>        // logical key → content
template_semantics: HashMap<String, TemplateSemanticIndex>
```

`template_paths` is keyed by the **logical** template path as stored in
`TemplateRegistry` — so shortcode templates already appear there as e.g.
`"shortcodes/youtube.html"`, `"shortcodes/tip.html"`. No model changes are
needed to enumerate shortcode names: filter `project.template_paths.keys()` for
`starts_with("shortcodes/")` and strip the prefix/suffix.

### How the LSP parses markdown content

The LSP does **not** call marq. It drives pulldown-cmark directly:

```rust
// authoring_lsp.rs:5461
Parser::new_ext(content, Options::all()).into_offset_iter()

// authoring_lsp.rs:6556  (markdown_headings)
for (event, range) in Parser::new_ext(content, Options::all()).into_offset_iter()
```

`Options::all()` includes `ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS` and
`ENABLE_YAML_STYLE_METADATA_BLOCKS`, so pulldown-cmark already emits
`MetadataBlock(PlusesStyle)` events for `+++ … +++` blocks — they are just
silently discarded by `markdown_references()` which only pattern-matches `Link`
and `Image` events.

### Frontmatter detection

All frontmatter helpers use a hand-rolled byte scan for TOML only:

```rust
// authoring_lsp.rs:9214
pub fn frontmatter_content_byte_range(content: &str) -> Option<FrontmatterContentByteRange> {
    content.strip_prefix("+++\n")?;
    let closing_start = content[4..].find("\n+++")? + 4;
    ...
}

// authoring_lsp.rs:6676
pub fn frontmatter_lsp_range(content: &str) -> Option<Range> {
    content.strip_prefix("+++\n")?;
    ...
}
```

YAML `---` frontmatter is **completely ignored**: no completion, no validation,
no hover. Documents using `---` get no frontmatter assistance at all.

---

## 2. Shortcodes: what the LSP needs

### 2a. The three grammars (as marq sees them)

All three are detected in the pulldown-cmark event stream inside
`libs/marq/src/render.rs`. The private functions are:

| Grammar | Detection function | Event shape |
|---|---|---|
| **Fenced** `+++ :name: <yaml> +++` | `parse_fenced_shortcode_name(text)` (line 729) | `MetadataBlock(PlusesStyle)` whose buffered text starts with `:name:` |
| **Body** `> *:name(args)*` + blockquote body | `extract_body_shortcode(events)` (line 798) | `BlockQuote` whose first paragraph is a single `Emphasis` whose text starts with `:` |
| **Inline** `*:name(args)*` (no body) | `resolve_inline_shortcodes()` (line 849) using `parse_emphasis_shortcode()` (line 745) | `Emphasis` whose text starts with `:` and contains only text/code |

Disambiguation: marq disambiguates a PlusesStyle block from TOML frontmatter by
checking whether the first non-empty line of the block text matches `:name:` (a
YAML key with a leading colon). **Only PlusesStyle blocks match** — a YamlStyle
`---` block is never a shortcode.

### 2b. Completion of shortcode names

The data is already in `AuthoringProject.template_paths`. A helper:

```rust
pub fn available_shortcode_names(project: &AuthoringProject) -> Vec<&str> {
    project
        .template_paths
        .keys()
        .filter_map(|k| {
            k.strip_prefix("shortcodes/")?.strip_suffix(".html")
        })
        .collect()
}
```

**Trigger points** (completion must fire on `:` inside the right context):

- **Fenced**: user is inside a `+++ … +++` block mid-document (not at doc start) and types `:`. Detect: the cursor line, after stripping whitespace, starts with `:` and the surrounding context is a pluses block.
- **Body**: user types `> *:` — the blockquote + emphasis open has been typed.
- **Inline**: user types `*:` mid-paragraph.

The simplest approach: add `:` to the LSP's `trigger_characters` (currently
`["(", "/", "@", "#", "."]`, line 369–376), then in `completions()` check
whether the byte before the cursor is `:` and the cursor is not in the
frontmatter block. If so, return shortcode name completion items.

For fenced shortcodes the context is a `+++ … +++` block, so the check is: the
current line starts with `:` (or is empty / has whitespace), the line immediately
above is `+++`, and the block is not the leading frontmatter block. A lightweight
text-scan (no full parse needed) is enough.

For inline/body: the check is `*:` appearing before the cursor on the same line,
outside a code span. Same `markdown_target_contexts`-style byte scan.

**Completion item structure** (name completion):

```
label:    "youtube"
kind:     CompletionItemKind::FUNCTION
detail:   "shortcodes/youtube.html"
insertText: fenced → ":youtube:\n  \n" (snippet inside +++ block)
           inline  → "youtube"  (the user types the closing *... * themselves)
```

For fenced insertions a snippet that places the cursor at the YAML args body is
ideal. For inline/body, the name alone suffices.

### 2c. Arg awareness

Shortcode templates (`shortcodes/<name>.html`) are gingembre templates, so their
variable accesses are already indexed in `project.template_semantics`. Extract the
top-level variable names (those not under a known context root like `config`,
`page`, etc.) from `TemplateSemanticIndex` to infer arg names:

```rust
let semantics = project.template_semantics.get("shortcodes/youtube.html")?;
// Variables accessed in the template that are not context roots → arg names.
```

These become the completion items when the cursor is inside a known shortcode's
YAML or pairs-arg position.

This is a stretch goal — name completion already covers the acute pain. Arg
completion can follow.

### 2d. Hover on shortcode invocations

A `scan_shortcodes()` function (see §4) returns `(name, range)` pairs. If the
hover position falls inside a shortcode span:

```
**Shortcode** `youtube`
Template: `shortcodes/youtube.html`
Grammar: fenced | body | inline
```

Link the template name so go-to-definition opens the template file (reuse the
existing template document link infrastructure).

### 2e. Not flagging shortcodes as markdown errors

**Already safe.** `markdown_references()` (line 5460) only matches `Link` and
`Image` events. Shortcode spans are `MetadataBlock` (fenced) or `Emphasis` (body/
inline) events — neither matches — so no diagnostic is produced. No change needed
here for the "no false positive" goal.

The one subtlety: if a fenced shortcode happens to contain a `[text](url)` in its
YAML args block, pulldown-cmark would not emit it as a link (metadata block content
is opaque text). Safe.

---

## 3. Frontmatter overload: `+++` disambiguation

### The three cases

```
# Case A: leading TOML frontmatter
+++
title = "Hello"
+++

# Case B: leading YAML frontmatter
---
title: Hello
---

# Case C: fenced shortcode (anywhere, including after frontmatter)
+++
:youtube:
  id: dQw4w9WgXcQ
+++
```

pulldown-cmark's rule: both `+++` and `---` blocks are emitted as `MetadataBlock`
(`PlusesStyle` and `YamlStyle` respectively). At the application level, marq
distinguishes Case A from Case C by checking whether the block text's first non-
empty line is `:name:`. The LSP must apply the same test.

### Current LSP behavior

`frontmatter_content_byte_range()` does a dumb `content.strip_prefix("+++\n")`
check. This means:

- Case A: detected correctly.
- Case B: **not detected** — YAML frontmatter is invisible to all frontmatter helpers.
- Case C at position 0: would be *incorrectly* detected as frontmatter. Practically
  impossible in real content (pages always have real TOML frontmatter first), but
  fragile.

### Fix: use marq's strip_frontmatter()

`marq::strip_frontmatter()` (exported from `libs/marq/src/frontmatter.rs`) already
handles both `+++` and `---` delimiters and returns a `StrippedFrontmatter`:

```rust
pub struct StrippedFrontmatter<'a> {
    pub raw: Option<&'a str>,      // content between delimiters
    pub body: &'a str,             // markdown after frontmatter
    pub format: Option<FrontmatterFormat>,  // Toml | Yaml
}
```

Replace `frontmatter_content_byte_range()` with a wrapper over `strip_frontmatter`:

```rust
pub fn frontmatter_content_byte_range(content: &str) -> Option<FrontmatterContentByteRange> {
    let stripped = marq::strip_frontmatter(content);
    let raw = stripped.raw?;
    // Recompute byte offsets from the raw slice position in content.
    let start = content.find(raw)?; // slice address arithmetic is cleaner
    Some(FrontmatterContentByteRange { start, end: start + raw.len(), format: stripped.format })
}
```

With `format` available, `frontmatter_entries()` can route YAML frontmatter to a
YAML key-scanner instead of the current TOML `key = value` scanner.

The YAML frontmatter used in fasterthanli.me content is simple (`key: value`
pairs), so a minimal YAML key scanner is enough for completion/diagnostics; full
YAML parsing is not required.

### Distinguishing a leading shortcode from frontmatter

After `strip_frontmatter` returns, if `raw` is `Some` and `format` is `Toml`,
apply `parse_fenced_shortcode_name(raw)` to check whether it is actually a
shortcode (leading `:name:` line). If it is, treat the document as having no
frontmatter at position 0. This handles the degenerate case and is consistent with
marq's own logic.

---

## 4. Reuse strategy: exposing marq's detection

### What needs exposing

The three detection functions in `libs/marq/src/render.rs` are currently private:
- `parse_fenced_shortcode_name(&str) -> Option<&str>` (line 729)
- `parse_emphasis_shortcode(&str) -> (String, Vec<(String, String)>)` (line 745)
- `extract_body_shortcode(events) -> Option<(String, Vec<events>)>` (line 798)

The LSP needs a **synchronous, non-rendering scan** over markdown content that
returns shortcode spans and names. This is a lightweight read-only pass — no
resolver, no gingembre, no async.

### Proposed: `pub fn scan_shortcodes(markdown: &str) -> Vec<ParsedShortcode>`

Add to `libs/marq/src/render.rs` (or a new `libs/marq/src/scan.rs`) and export
from `libs/marq/src/lib.rs`:

```rust
pub struct ParsedShortcode {
    pub name: String,
    pub kind: ShortcodeKind,
    pub byte_start: usize,
    pub byte_end: usize,
    /// For fenced: the raw YAML block content. For body/inline: the pairs text.
    pub raw_args: String,
}

pub enum ShortcodeKind { Fenced, Body, Inline }

pub fn scan_shortcodes(markdown: &str) -> Vec<ParsedShortcode>
```

Implementation: run the same pulldown-cmark event loop with `MetadataBlock` and
`Emphasis` handling (a stripped-down version of the render pass, no resolver), call
the existing private functions, collect results. The private functions can be
changed from `fn` to `pub(crate)` or inlined into the new public function.

This is the pattern already used by marq for `detect_rfc2119_keywords()` and
`parse_rule_id()` in `reqs.rs` — a public scanning interface that exposes detection
results without triggering rendering.

### Can the LSP call marq directly?

Yes. `dodeca-authoring-lsp` already depends on the `dodeca` crate, which depends
on `marq = { path = "libs/marq" }`. The LSP can add `marq` as a direct dependency
or reach it transitively. Given that the LSP already uses `marq::slugify()` (line
6570), a direct import is already in effect. `scan_shortcodes()` is a pure
synchronous call — no async, no trait objects.

---

## 5. Phased plan

Priority is **unblocking authoring of the real site**, not theoretical completeness.
The real site uses `+++` TOML frontmatter (not YAML), and shortcodes are already
rendering correctly in dodeca's render pass.

### Phase 1 — Stop silently ignoring YAML frontmatter (2h)

Replace `frontmatter_content_byte_range()` with a version backed by
`marq::strip_frontmatter()`. This also correctly handles the degenerate "shortcode
at position 0" case. YAML frontmatter diagnostics and completions can follow
automatically since the entry point (`frontmatter_entries()`,
`frontmatter_completion_context()`) already branches on the byte range; they just
need the format tag to route YAML lines differently.

Files: `crates/dodeca-authoring-lsp/src/authoring_lsp.rs` (7 functions that call
`frontmatter_content_byte_range`).

### Phase 2 — Shortcode name completion (3h)

1. Add `:` to the LSP trigger characters (line 370).
2. In `completions()`, before the `markdown_target_context_at_position` call, add a
   check: if cursor is after `*:` (inline/body context) or after a `+++\n:` line
   (fenced context), return `available_shortcode_names(project)` as `CompletionItem`
   entries (`CompletionItemKind::FUNCTION`, detail = template path).
3. For the fenced case, the inserted text should be `":name:\n  "` (the name colon-
   wrapped as required by the grammar).

Files: `authoring_lsp.rs` (completions function + helpers), no marq changes needed
for this minimal version (text scan is enough).

### Phase 3 — Expose marq's scan_shortcodes and wire hover (4h)

1. Expose `pub fn scan_shortcodes()` → `Vec<ParsedShortcode>` from
   `libs/marq/src/render.rs` + `libs/marq/src/lib.rs`. Make
   `parse_fenced_shortcode_name`, `parse_emphasis_shortcode` pub(crate) or move the
   detection logic inline.
2. Cache scan results in `AuthoringWorld` or compute on demand in
   `hover_for_position()`.
3. In `hover_for_position()`, before the `reference_at_position` fallback, call
   `scan_shortcodes(&content)` and check if position falls inside a shortcode span.
   Return a hover with name, grammar, and template path.
4. Add document links for shortcode names → `shortcodes/<name>.html` template file.

Files: `libs/marq/src/render.rs`, `libs/marq/src/lib.rs`,
`crates/dodeca-authoring-lsp/src/authoring_lsp.rs`.

### Phase 4 — Arg completions (after workstream 2/3 lands)

Once the migration has more shortcodes in play and the author is actively writing
`.html.jinja` templates, arg name inference from `template_semantics` becomes
valuable. Defer until the shape of the shortcode library is more settled.

### Phase 5 — Workstream 3 fields (date/draft/tags/series)

Once workstream 3 (taxonomy/series) lands, update `marq::Frontmatter` and the
`FrontmatterFieldSpec` introspection in the LSP to recognise the new fields
automatically (the field-spec list is built from `<Frontmatter as Facet>::SHAPE`,
so new fields appear at no extra cost).

---

## Summary of file changes per phase

| Phase | marq | authoring_model | authoring_lsp |
|---|---|---|---|
| 1 (YAML fm) | none (use existing `strip_frontmatter`) | none | `frontmatter_content_byte_range`, `frontmatter_lsp_range`, and 5 callers |
| 2 (name completion) | none | none | `completions()`, new `available_shortcode_names()`, trigger chars |
| 3 (hover + scan) | expose `scan_shortcodes()` | none | `hover_for_position()`, document links |
| 4 (arg completion) | none | none | `completions()` + arg inference helper |
| 5 (wks 3 fields) | add fields to `Frontmatter` | propagate | auto via facet reflection |
