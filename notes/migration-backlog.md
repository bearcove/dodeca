# fasterthanli.me → dodeca migration backlog

Tracking the move of fasterthanli.me (currently on the custom `home` CMS in `~/cove`,
content in `~/fasterthanli.me`) onto dodeca. Companion to `notes/shortcode-port.md`.

Decisions already made (Amos):
- **Deployment format = dodeca running as a SERVER** (not static files to a CDN). This
  matters: request-time rendering is fine, and dodeca-as-server already has an identity
  concept (the `auth` config). So "identity" features ride on the running server.
- **Gated content** (early-access / reveal-date articles): OUT OF SCOPE for now.
- **Login**: stays in scope, but only for **Discord access** (sponsors → Discord role),
  NOT for hiding article bodies. Can be a **proxy on top of dodeca's server-mode identity**
  rather than a fully separate service. Not on the critical path.
- Bend dodeca to fit the content, not the content to dodeca.

## Showcase / oracle
`examples/showcase/` — a runnable example site (`ddc build examples/showcase`) that
exercises every migration feature, one section per workstream. Doubles as a dev target
and golden-output oracle: a feature that isn't implemented shows up as broken output here
before it reaches the real site. First section (shortcodes) already surfaced 4 gaps (below).

## Workstream status

### 1. Shortcodes — VERTICAL SLICE WORKING
- Layer 1 (marq): DONE. 3 grammars (fenced `+++ :name: +++`, body `> *:name*`, inline
  `*:name*`), 149 tests green. Commits 5f54a2c8 / 61ce1b09 / 6e38e3da / d071b24d / c121636b.
- Layer 2/3 (dodeca): DONE for the vertical slice (commits 8097fbf5 / 443de305). `youtube`
  renders end-to-end via gingembre (fenced + inline); args (YAML + pairs) spread as template
  vars; body threaded; `escape_for_attribute`/`basic_markdown` filters work; reviewed —
  dependency tracking (picante `TemplateRegistry` input) and template naming
  (`shortcodes/<name>.html`, matches `.jinja`-stripped convention) both correct; dodeca
  compiles, 159 gingembre tests pass.
- ~~**`get_media(src)` stub**~~ DONE (b5064e6a): returns the resolved src; new `markup`
  host fn emits a safe `<img>` that the existing image post-pass upgrades to responsive
  `<picture>` (and tracks the asset dep). Verified: tip/bearsays avatars emit.
- ~~**gingembre method calls on call results**~~ DONE (b5064e6a): `eval_call` dispatches
  `<expr>.method(args)` for any receiver, passing it as the first positional arg.
- **Remaining for figure/media**: add a showcase section with a REAL image so the
  `<img>→<picture>` post-pass + asset dependency are exercised end-to-end (also unblocks
  the `#[ignore]` get_media dependency test). figure/media templates use the same
  `get_media().markup()` pattern, now working — needs verification with a real asset +
  `basic_markdown`/`escape_for_attribute` (those filters already exist).

### 2. gingembre improvements — TODO
So the existing `.jinja` templates port unchanged. Engine lives at `libs/gingembre`.
Gaps found vs the templates fasterthanli.me uses (`~/fasterthanli.me/templates/`):
- ~~**`{%- -%}` whitespace control**~~ DONE (2a4cb19c): lexer consumes trim dashes on all
  three delimiters; trims preceding/following text. 159 gingembre tests pass.
- **Lenient (Jinja) undefined variables** — HIGH, NEXT. gingembre is strict: `{% if width %}`
  on an undefined `width` raises `UndefinedError`. ftl templates use undefined vars as
  optional args everywhere (figure.html: width/height/attr/attrlink all optional), assuming
  Jinja semantics where undefined is falsy / renders empty. Need: undefined → falsy in
  boolean/`if` contexts, undefined passable as a kwarg (→ null), and `{{ undefined }}` →
  empty (or configurable). Watch the existing intentional `UndefinedError` tests — decide
  targeted leniency vs a global lenient-undefined value. Found by the showcase (figure.html).
- **`loop.*`** — `loop.index`, `loop.first`, `loop.last`, `loop.revindex` not exposed in
  for-loop context.
- **`{% raw %}…{% endraw %}`** — token not in lexer.
- **`{% filter name %}…{% endfilter %}`** block.
- **Missing filters**: `tojson` (must use facet-json, not serde), `truncate`, `urlencode`,
  `wordcount`, `striptags`, `indent`.
- **Missing functions**: `int()`, `float()`, `string()`, `list()`, `range()`.
- **`super()`** inside blocks (unconfirmed — verify).
- **Method calls on call results** (`get_media(src).markup(...)`): `eval_call` only handles
  `obj.method()` for `obj: Var`. Needed for figure/media shortcodes. (Found during Layer 2.)
- ~~**`is defined` / `is not defined` throws on an undefined operand**~~ DONE (3e049131):
  operand is now lenient (undefined → null) for these tests. Verified: bearsays default mood.
- ~~**Namespaced macro call renders EMPTY** (`{{ macros.youtube_embed(...) }}`)~~ DONE
  (3e049131): dotted `ns.macro(...)` now routes to the macro registry in the Print path,
  like the `::` form. Verified: youtube embed renders. (Note: still Print-context only,
  same as the `::` form — not yet in `{% set %}`/filter contexts.)
- NOTE: the `default` filter was fixed during Layer 2 to match Jinja2 (was broken on
  undefined vars) — commit 443de305.
- Custom filters the shortcodes/templates need: `basic_markdown`, `escape_for_attribute`,
  and `get_media(src).markup(...)` (these overlap with the shortcode Layer 3 work).

### 3. Taxonomy / series / feeds — TODO (dodeca additions) — **HIGH priority (Amos)**
dodeca keeps everything beyond `title/weight/description/template` in `extra` and has no:
- **Tag/taxonomy pages** — `tags` are unindexed strings; no `/tags/rust/` aggregation.
- **Series prev/next navigation** — no built-in ordering within a series.
- **Pagination** — `section.pages` is always the full flat list.
- **RSS/Atom feeds** — absent (ftl has `index.xml.jinja`). 18 series + 126 articles need a feed.
- **Sitemap** — absent.
- **Redirects / aliases** — no mechanism, no `aliases` frontmatter. Changed slugs would 404.
- **First-class `date` / `draft` / `slug`** — date lives in `extra`; no draft exclusion;
  URL is always derived from file path.

### 4. Math → MathML (build-time) — TODO
`$…$` / `$$…$$` in 18 content files. Decision (Amos): render to **MathML at build time**,
NOT client-side KaTeX/MathJax JS. So: enable math parsing (pulldown-cmark `ENABLE_MATH` is
off in marq) and convert TeX → MathML server-side during render (e.g. a math handler that
emits `<math>`). No JS shipped.

### 5. Liquid + embedded SQL articles — TODO (rewrite, low effort)
What this is: two *meta* articles that demonstrate home's own CMS by embedding live
`{% capture %} … {% assign x = sql | query: revision %} … {% for page in pages %}`
templating that runs SQL against home's content DB at render time:
- `content/articles/i-won-free-load-testing/_index.md`
- `content/articles/a-new-website-for-2020/_index.md`
(The 3rd hit, `content/tests/shortcode.md`, is just a ```liquid code *sample*, not executed.)
dodeca has no query-the-DB-from-markdown feature. Decision (Amos): don't rewrite — if these
are a problem, just **redirect those 2 URLs to an archive.org snapshot**. Needs the redirect
mechanism (workstream 3). Tiny scope — only 2 real files, and they're dispensable.

### 6. Search parity — LIKELY FINE (validate when running)
home uses server-side Tantivy; dodeca has `cell-search`, which Amos says is actually very
good. Not a real concern — just confirm it covers the corpus once the site is rendering.

### 7b. Authoring LSP awareness — TODO (don't forget)
The in-editor authoring LSP (`crates/dodeca-authoring-lsp`, + `authoring_model.rs`) is
blind to everything this migration adds, so editing content gives stale/wrong assistance:
- **Shortcodes**: it doesn't know the grammars (`+++ :name: +++`, `> *:name*`, inline
  `*:name*`), the available shortcode names (from `templates/shortcodes/*.html`), or their
  args. Wants: completion of shortcode names + args, hover/validation, and NOT flagging
  shortcode syntax as errors.
- **Frontmatter**: `+++` is now overloaded — leading `+++` is TOML frontmatter, but a
  `+++ :name:` block mid/anywhere is a shortcode. The LSP must distinguish them and
  understand both `---` (YAML) and `+++` (TOML) frontmatter.
- Later: new content-model fields (date/draft/tags/series) once workstream 3 lands.
Amos flagged this from opening a file in the editor. Not blocking, but keep it on the list.

### 7. Identity → Discord — PROXY on dodeca server-mode identity
Since dodeca deploys as a server (see top) and already has an identity concept, this need
not be a separate service — it can be a proxy/extension on top of the running dodeca server,
reusing its identity: log in with GitHub/Patreon → verify sponsor tier → grant Discord role.
The site stays public; only Discord access is gated. `home.json` has the inputs
(`patreon_campaign_ids`, `admin_github_ids`, etc.). Not blocking the static migration.

## Scale (for sizing)
~285 markdown files, 126 articles, 9 series, 3 videos, content dir ~346M (mostly assets).
