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

## Workstream status

### 1. Shortcodes — IN PROGRESS
- Layer 1 (marq): DONE. 3 grammars (fenced `+++ :name: +++`, body `> *:name*`, inline
  `*:name*`), 149 tests green. Commits 5f54a2c8 / 61ce1b09 / 6e38e3da / d071b24d / c121636b.
- Layer 2/3 (dodeca gingembre resolver + filters): delegated to a Paseo agent. Spec in
  `notes/shortcode-port.md`. Usage counts there (~4500 invocations).

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
dodeca is static and has no query-the-DB-from-markdown feature. These 2 articles either get
rewritten to plain content (the live tables become static snapshots) or left degraded.
Tiny scope — only 2 real files.

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
