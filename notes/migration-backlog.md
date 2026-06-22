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
- **Follow-ups to finish shortcodes (block the highest-count ones, figure 191 / media 152):**
  - **`get_media(src)` is stubbed (returns NULL)** in the template host — implement it so
    figure/media render images. (Should resolve assets through the picante-tracked path.)
  - **gingembre: method calls on call results** (`get_media(src).markup(...)`) — `eval_call`
    only dispatches `obj.method()` when `obj` is a `Var`, not a function result. Extend
    gingembre to allow method calls on arbitrary expression results. (Also a workstream-2 item.)

### 2. gingembre improvements — TODO
So the existing `.jinja` templates port unchanged. Engine lives at `libs/gingembre`.
Gaps found vs the templates fasterthanli.me uses (`~/fasterthanli.me/templates/`):
- **`{%- -%}` whitespace control** — dashes not recognized by the lexer. Used heavily
  (e.g. `figure.html.jinja`). HIGH priority — without it, whitespace is wrong everywhere.
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
- **`is defined` / `is not defined` test throws on an undefined operand** instead of
  evaluating to true/false. `{% if mood is not defined %}` raises `UndefinedError(mood)`.
  The defined-test must short-circuit (that's its whole purpose). Found by the showcase
  (`bearsays.html`). Every speech-bubble template that defaults an optional arg hits this.
- **Namespaced macro call renders EMPTY** (`{% import "macros.html" as macros %}` then
  `{{ macros.youtube_embed(...) }}`): produces nothing, no error. Inlining the macro body
  works, so it's specifically the import-namespace call path. CRITICAL — every ftl shortcode
  template opens with this import. Found by the showcase (`youtube.html`).
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

### 7. Identity → Discord — PROXY on dodeca server-mode identity
Since dodeca deploys as a server (see top) and already has an identity concept, this need
not be a separate service — it can be a proxy/extension on top of the running dodeca server,
reusing its identity: log in with GitHub/Patreon → verify sponsor tier → grant Discord role.
The site stays public; only Discord access is gated. `home.json` has the inputs
(`patreon_campaign_ids`, `admin_github_ids`, etc.). Not blocking the static migration.

## Scale (for sizing)
~285 markdown files, 126 articles, 9 series, 3 videos, content dir ~346M (mostly assets).
