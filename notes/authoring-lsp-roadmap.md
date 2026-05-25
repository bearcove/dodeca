# Authoring LSP roadmap

This tracks editor-native authoring features for `ddc lsp`. The goal is for the
language server to expose Dodeca's site graph, route model, frontmatter metadata,
template model, and build provenance directly inside editors.

Status values:

- `todo`: not started
- `doing`: actively being implemented
- `blocked`: needs a design decision or prerequisite
- `done`: implemented, verified, and shipped

## Principles

- Frontmatter is the page identity surface.
- Markdown links, source links, headings, static assets, templates, config, and
  generated output should resolve through Dodeca's own model, not editor-local
  string guesses.
- Features should prefer production-path data structures over parallel indexes.
- Every feature should have an LSP smoke test when practical, because the unit
  path can pass while JSON-RPC wiring is missing.

## Work items

| ID | Status | Feature | User-facing behavior | Notes |
| --- | --- | --- | --- | --- |
| `lsp-001` | `done` | Missing page quick fix | Broken route diagnostics offer a quick fix that creates and opens the new page. | Creates frontmatter only; no duplicate body title. |
| `lsp-002` | `done` | Project-wide link diagnostics | Editors receive warnings for broken links across the whole project, not just the open file. | Current diagnostic kinds cover missing routes, anchors, source files, and static assets. |
| `lsp-003` | `done` | Page backlinks | Find All References on page frontmatter returns every Markdown link resolving to that page. | Covers route links, `@/source.md`, anchors, and relative links. |
| `lsp-004` | `todo` | Link completions | Complete routes, source files, static assets, and heading fragments inside Markdown links. | Should use current page context for relative links and route context for `#fragment`. |
| `lsp-005` | `todo` | Link hover | Hovering a Markdown link explains the resolved target page, route, source file, heading, or static asset. | Broken links should expose the same reason as diagnostics. |
| `lsp-006` | `todo` | Frontmatter hover | Hovering frontmatter shows canonical route, source file, title, template, output path, and backlink count. | Backlink count can share the references implementation. |
| `lsp-007` | `todo` | Page workspace symbols | Editor symbol search lists pages by title, route, source file, and headings. | Lets users jump around the site graph without knowing filenames. |
| `lsp-008` | `todo` | Heading references | Find All References on a heading returns every link to that exact fragment. | Also supports route fragment links such as `/page#heading`. |
| `lsp-009` | `todo` | Heading rename | Renaming a heading updates every fragment link that targets it. | Depends on heading references and slug generation parity. |
| `lsp-010` | `todo` | Page route rename | Renaming or moving a page updates every route, relative, and source-file link that targets it. | Needs a typed edit plan before applying workspace edits. |
| `lsp-011` | `todo` | Broken anchor quick fixes | Missing anchor diagnostics suggest existing headings or creating a heading. | Fuzzy matching can suggest nearby heading slugs. |
| `lsp-012` | `todo` | Extract page code action | Extract selected Markdown into a new page and replace the selection with a link. | New page gets frontmatter, not a duplicate heading. |
| `lsp-013` | `todo` | Frontmatter schema | Complete and validate known frontmatter fields. | Should source field semantics from Dodeca's typed page model. |
| `lsp-014` | `todo` | Frontmatter document links | Template, asset, and data-file fields in frontmatter become navigable. | Requires knowing which fields contain paths. |
| `lsp-015` | `todo` | Template navigation | Go to definition for template extends/includes, blocks, macros, filters, and tests. | Start with path-bearing constructs before semantic block matching. |
| `lsp-016` | `todo` | Template diagnostics | Warn on missing templates, missing blocks, unknown macros, and unavailable filters/tests. | Should avoid reimplementing Gingembre analysis outside the real parser. |
| `lsp-017` | `todo` | Template completions | Complete `page`, `section`, `site`, `config`, `data`, filters, tests, and functions in templates. | Typed data from Dodeca models should drive completions. |
| `lsp-018` | `todo` | Site graph diagnostics | Report orphan pages, duplicate titles, duplicate routes, and pages with no inbound links. | Needs policy for intentional orphans. |
| `lsp-019` | `todo` | Route graph view | Expose incoming and outgoing page links through editor-native views. | Could begin with references/document symbols before custom UI. |
| `lsp-020` | `todo` | Build provenance hover | Explain which template, assets, transforms, and output path a page uses. | Best powered by reusable build/provenance data, not an LSP-only model. |

## Suggested implementation order

1. Completions for links and fragments.
2. Hovers for links and frontmatter.
3. Workspace symbols for pages and headings.
4. Heading references and heading rename.
5. Page route rename.
6. Template navigation and diagnostics.
7. Frontmatter schema/document links.
8. Site graph diagnostics and build provenance.

## Tracking policy

- This file is the canonical local roadmap.
- GitHub issues can mirror individual IDs when work is ready to schedule, review,
  or discuss publicly.
- Commits should mention the roadmap ID when they complete or materially advance
  an item.
- When an item ships, update its status and record any important verification
  notes in the commit message or issue.
