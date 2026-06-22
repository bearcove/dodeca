# gingembre parser rewrite: cstree, one parser for engine + LSP

Branch `gingembre-cstree` (worktree `~/oss/dodeca-gingembre-cstree`, off `shortcodes`).

## Why

gingembre's hand-written parser has structural gaps that surface one at a time as the
ftl migration trips over them: `is defined` raised on undefined, dotted namespaced macro
calls returned null, **field access in a positional call arg failed to parse**
(`render_reading_time(latest_article.reading_time)` → "Unexpected Dot"), strict-undefined,
etc. Whack-a-mole. Replace the front-end with a real, grammar-driven, **error-recovering**
parser that is **shared with the authoring LSP** (workstream 7b) instead of the LSP
re-implementing template parsing.

## Decisions (Amos)

- **cstree** (not rowan, not PEG). cstree is the actively-maintained rowan fork (rowan
  unmaintained since 2024); it interns token text (great for a template language) and is
  the direction styx/vixen will migrate to as well → fewer deps long-term.
- **Events architecture** (mirror `~/oss/facet/styx-parse`): the parser emits a flat
  `Start/Token/Finish/Error` event stream; a separate builder turns events into the cstree
  green tree. This decouples the parser from the tree lib (rowan↔cstree swap is localized)
  and is the rust-analyzer playbook.
- **One parser, two consumers**: the engine **lowers the CST → the existing gingembre AST**
  (so `eval`/`render` and their 159 tests are unchanged — we swap the front-end behind the
  AST boundary and keep it green throughout). The LSP consumes the **CST** directly (lossless,
  error-recovered, positions for every byte).
- Keep the `?` operator, `{%- -%}` whitespace control, and lenient `defined` semantics —
  they're requirements of the new grammar (branch is off `shortcodes`).

## Target grammar (catalogued from the REAL ftl templates, `~/fasterthanli.me/templates/`)

### Template structure (outer)
- Text runs.
- `{{ expr }}` interpolation, with whitespace control `{{- -}}`.
- `{% stmt %}` statements, with `{%- -%}`.
- `{# comment #}`, with `{#- -#}`. Nested comments.
- Whitespace-control dash trims adjacent text run.

### Statements (all used by ftl)
`if` / `elif` / `else` / `endif`; `for x in expr` / `else` / `endfor`; `set name = expr`
(and `set name %}…{% endset` block form); `block name` / `endblock`; `extends "path"`;
`include "path"`; `import "path" as alias`; `macro name(params) ` / `endmacro`.

### Expressions
- Literals: string, int, float, bool, none, list `[…]`, dict `{…}`.
- `var`; field `a.b`; index `a[i]`; **slice `a[:n]` / `a[a:b]`** (used:
  `randomized_sponsors[:visible_sponsor_count]`).
- Call `f(pos, kw=val)` — **positional args may be ANY expression** (field access, etc.);
  kwargs are `ident = expr`. (This is the bug that motivated the rewrite.)
- Filter `expr | name` and `expr | name(args)`.
- **No `::` macro syntax** — ftl uses `macros.x(...)` (dotted) exclusively (40×, 0× `::`).
  Parse all `a.b(...)` uniformly as Call(Field…); resolve macro-vs-method at eval time.
  (Can drop the `MacroCall`/`::` grammar; keep a render-time namespace check.)
- Test `expr is name` / `expr is not name` (used: `defined`, `not defined`; others like
  `is required`/`is a` in the survey are prose, not tests — verify).
- Ternary `a if cond else b`.
- Unary `not`, `-`. Binary: `and or`, `== != <= >= < >`, `+ - * / // %`, `~` (concat),
  `in` / `not in`, `**`.
- Postfix `?` (lenient access; `expr?` → null instead of raising on undefined).
- `loop.index0` / `loop.last` (parser: field access on `loop`; engine must expose `loop`).

### Filters seen (engine registry, not parser): asset_url, format_day_month_year,
format_rfc3339, format_time_ago, truncate_html, shuffle, downcase, is_past, is_future,
to_json, urlencode, length, int, safe, escape_for_attribute, basic_markdown. (Many are
home-specific → separate from the parser work; some become dodeca builtins per Amos:
reading-time, date formatting.)

## Architecture / crate layout

- `SyntaxKind` enum: tokens (delimiters, dashes, idents, literals, operators, `?`, …) +
  nodes (Template, Text, Interpolation, IfStmt, ForStmt, …, Expr nodes). One flat enum,
  `#[repr(u16)]`, with a `Language` impl for cstree.
- `lexer`: adapt the existing gingembre lexer's mode logic (text vs code, whitespace-control
  dashes already handled there) to emit tokens for the parser. Reuse what works.
- `parser` (events): recursive-descent + Pratt for expressions, emits events with recovery
  (error nodes, never bails on the LSP's behalf).
- `builder`: events → cstree `GreenNode`.
- `lower`: typed CST → existing `ast::Expr`/`Node` for the engine.
- Typed AST layer over the CST for the LSP (accessor methods per node).

## Plan / phases

1. Catalog ✅ (this note) + cstree dep + `SyntaxKind` + `Language` impl.
2. Fixture suite FIRST: a corpus of template snippets covering every construct above;
   parse-level (CST shape) snapshots + (later) render goldens. Drive the parser with it.
3. Lexer adaptation → tokens.
4. Event parser (structure → statements → expressions/Pratt) with recovery.
5. cstree builder + typed AST.
6. CST → gingembre AST lowering; flip the engine to it; keep all 159 engine tests green.
7. Point the LSP at the CST; delete the LSP's separate template parsing.
8. Validate against the real ftl templates (the build should parse all 38).
