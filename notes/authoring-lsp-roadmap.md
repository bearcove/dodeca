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
| `lsp-000` | `done` | Shared authoring model | LSP page/link semantics come from the same database/build-tree path as Dodeca, with editor and watched-file notifications applied to authoring inputs. | The live authoring workspace updates content, template, Sass, static/dist, and data registries; do not grow LSP-only route/frontmatter/heading semantics. |
| `lsp-001` | `done` | Missing page quick fix | Broken route diagnostics offer a quick fix that creates and opens the new page. | Creates frontmatter only; no duplicate body title. |
| `lsp-002` | `done` | Project-wide link diagnostics | Editors receive warnings for broken links across the whole project, not just the open file. | Current diagnostic kinds cover missing routes, anchors, source files, and static assets. |
| `lsp-003` | `done` | Page backlinks | Find All References on page frontmatter returns every Markdown link resolving to that page. | Covers route links, `@/source.md`, anchors, and relative links. |
| `lsp-004` | `done` | Link completions | Complete routes, source files, static assets, and heading fragments inside Markdown links. | Uses the shared authoring model for candidates and LSP edit spans only for replacement ranges. |
| `lsp-005` | `done` | Link hover | Hovering a Markdown link explains the resolved target page, route, source file, heading, or static asset. | Broken links expose the same reason as diagnostics. |
| `lsp-006` | `done` | Frontmatter hover | Hovering frontmatter shows canonical route, source file, title, template, output path, and backlink count. | Backlink count shares the references implementation. |
| `lsp-007` | `done` | Page workspace symbols | Editor symbol search lists pages by title, route, source file, and headings. | Also exposes per-document page/heading outline symbols. |
| `lsp-008` | `done` | Heading references | Find All References on a heading returns every link to that exact fragment. | Supports route, source-file, and relative fragment links. |
| `lsp-009` | `done` | Heading rename | Renaming a heading updates every fragment link that targets it. | Uses the shared authoring model for target identity and exact LSP edit spans for heading text and fragments. |
| `lsp-010` | `done` | Page route rename | Renaming or moving a page updates every route, relative, and source-file link that targets it. | Frontmatter rename returns a typed file-rename plus link-edit plan resolved through the shared authoring model. |
| `lsp-011` | `done` | Broken anchor quick fixes | Missing anchor diagnostics suggest existing headings or creating a heading. | Suggestions and heading creation are resolved from the shared authoring page/heading model. |
| `lsp-012` | `done` | Extract page code action | Extract selected Markdown into a new page and replace the selection with a link. | New page gets frontmatter, strips a selected leading heading into metadata, and uses a document-change create/edit plan. |
| `lsp-013` | `done` | Frontmatter schema | Complete and validate known frontmatter fields. | Field names and scalar kinds come from the Facet shape of Dodeca's typed `Frontmatter`; LSP scanning only supplies live-buffer ranges. |
| `lsp-014` | `done` | Frontmatter document links | Template, asset, and data-file fields in frontmatter become navigable. | Template, `asset`, and `data` fields are typed frontmatter fields and resolve through authoring template/static/data registries; arbitrary `[extra]` keys are intentionally not treated as paths. |
| `lsp-015` | `done` | Template navigation | Go to definition for template extends/includes, blocks, macros, filters, and tests. | Path-bearing `extends`, `include`, and `import` targets resolve through Gingembre parsing and the authoring template model; block, macro, filter, and test identifiers use Gingembre AST spans and registries. |
| `lsp-016` | `done` | Template diagnostics | Warn on missing templates, missing blocks, unknown macros, and unavailable filters/tests. | Uses Gingembre AST spans, template inheritance/import resolution, and exported Gingembre built-in filter/test registries. |
| `lsp-017` | `done` | Template completions | Complete `page`, `section`, `config`, `data`, filters, tests, functions, and imported macros in templates. | Data keys come from the authoring model, functions from the template host, and filters/tests from Gingembre registries. |
| `lsp-018` | `done` | Site graph diagnostics | Report orphan pages, duplicate titles, duplicate routes, and pages with no inbound links. | Root is exempt; section landing pages with children are exempt; leaf pages/sections without inbound Markdown links warn. |
| `lsp-019` | `done` | Route graph view | Expose incoming and outgoing page links through editor-native views. | `dodeca.routeGraph` returns per-page incoming/outgoing Markdown link edges with source spans for editor extensions. |
| `lsp-020` | `done` | Build provenance hover | Explain which template, assets, transforms, and output path a page uses. | Frontmatter hover includes transform chain, template dependency names, linked static assets, data keys, and output path from the authoring model. |

## Suggested implementation order

1. Keep expanding the shared authoring model when a feature needs more Dodeca semantics.
2. Completions for links and fragments.
3. Hovers for links and frontmatter.
4. Workspace symbols for pages and headings.
5. Heading references and heading rename.
6. Page route rename.
7. Template navigation and diagnostics.
8. Frontmatter schema/document links.
9. Site graph diagnostics and build provenance.

## Tracking policy

- This file is the canonical local roadmap.
- GitHub issues can mirror individual IDs when work is ready to schedule, review,
  or discuss publicly.
- Commits should mention the roadmap ID when they complete or materially advance
  an item.
- When an item ships, update its status and record any important verification
  notes in the commit message or issue.
