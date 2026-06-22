# fasterthanli.me → dodeca migration backlog

Tracking the move of fasterthanli.me (currently on the custom `home` CMS in `~/cove`,
content in `~/fasterthanli.me`) onto dodeca. Companion to `notes/shortcode-port.md`.

Decisions already made (Amos):
- **Gated content** (early-access / reveal-date articles): OUT OF SCOPE for now.
- **Login**: stays in scope, but only for **Discord access** (sponsors → Discord role),
  NOT for hiding article bodies. Becomes a small standalone identity→Discord service,
  independent of the static site. Not on the critical path.
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

### 3. Taxonomy / series / feeds — TODO (dodeca additions)
dodeca keeps everything beyond `title/weight/description/template` in `extra` and has no:
- **Tag/taxonomy pages** — `tags` are unindexed strings; no `/tags/rust/` aggregation.
- **Series prev/next navigation** — no built-in ordering within a series.
- **Pagination** — `section.pages` is always the full flat list.
- **RSS/Atom feeds** — absent (ftl has `index.xml.jinja`). 18 series + 126 articles need a feed.
- **Sitemap** — absent.
- **Redirects / aliases** — no mechanism, no `aliases` frontmatter. Changed slugs would 404.
- **First-class `date` / `draft` / `slug`** — date lives in `extra`; no draft exclusion;
  URL is always derived from file path.

### 4. Math (KaTeX) — TODO
`$…$` / `$$…$$` used in 18 content files. pulldown-cmark `ENABLE_MATH` not enabled in marq;
no KaTeX/MathJax pipeline. Add a math extension/handler.

### 5. Liquid + embedded SQL articles — TODO (rewrite, low effort)
Only 3 files use `{% assign %} … | query: revision` (SQL against home's content DB):
`articles/i-won-free-load-testing`, `articles/a-new-website-for-2020`, and one more.
Inherently dynamic; rewrite as plain content rather than port the SQL/Liquid layer.

### 6. Search parity — LATER
home uses server-side Tantivy; dodeca has `cell-search` (static index, pagefind-style).
Different but dodeca has a story. Validate it covers the corpus acceptably.

### 7. Identity → Discord service — SEPARATE PROJECT
Small standalone service: log in with GitHub/Patreon → verify sponsor tier → grant Discord
role. Owns identity; the static site stays public. `home.json` has the inputs
(`patreon_campaign_ids`, `admin_github_ids`, etc.). Not blocking the static migration.

## Scale (for sizing)
~285 markdown files, 126 articles, 9 series, 3 videos, content dir ~346M (mostly assets).
