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
- **One representation, no owned AST** (revised — Amos): the engine and the LSP both
  evaluate directly off **typed views over the CST** (rust-analyzer's `ast` accessor
  pattern). There is NO separate owned AST — `gingembre/src/ast.rs` gets deleted along with
  the old lexer/parser. For gingembre there's no optimization need that an owned AST would
  serve; a typed-CST layer gives typed access + decoded leaf values without the duplication.
- **Correctness bar = rendered output, not AST parity.** We do NOT chase field-for-field /
  span-for-span AST equality (that's the month-long trap). The oracle is "the same HTML comes
  out": the engine's render/output tests, the showcase, and the 38 real ftl pages. AST-shape
  snapshot tests are dropped; behaviour (output) tests stay.
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

## Plan / phases (status)

1. ✅ Catalog + cstree dep + `SyntaxKind` (`c35688e5`).
2. ✅ Lossless lexer (text/code modes, trim delimiters, nested comments) — 6 tests.
3. ✅ Recursive-descent + precedence parser → cstree, error-recovering — 9 tests
   (lossless roundtrip + the field-access-in-call-arg the old parser choked on).
4. ✅ Typed views over the CST (`ast.rs`, no owned AST) — 15 tests.
5. ⏳ **Port `eval`/`render` to consume the typed views**; delete `gingembre/src/{lexer,parser,
   ast}.rs`. Bar = render output: the engine's output tests + showcase + ftl pages. This is
   the bulk — `eval.rs` (~1900 lines) + `render.rs` change from matching owned `Expr`/`Node`
   enums to matching `SyntaxKind` + typed accessors. Repoint `gingembre::parse`/`Template`.
6. ⏳ Point the LSP at the CST; delete its ~9700-line separate template parsing.
7. ⏳ Validate: parse + render all 38 ftl templates; showcase output unchanged.

### Step 6 — LSP migration (scoped; do AFTER the engine flip, BEFORE deleting old front-end)
`crates/dodeca-authoring-lsp/src/authoring_lsp.rs` couples to the OLD owned front-end:
- imports `gingembre::ast::{Expr, Ident, Node, StringLit, Literal}`,
  `gingembre::parser::Parser as TemplateParser`, `gingembre::semantic::*`.
- parses via `TemplateParser::new(file, content).parse()` (≈ lines 2034, 6096, 6112, 6249).
- walks the owned AST: `collect_template_expr_diagnostics`, `collect_literal_*`,
  `collect_expr_macro_call_occurrences`, `collect_expr_definition_targets`, etc.
- consumes `project.template_semantics` (the `gingembre::semantic` index).

So **deleting `gingembre/src/{ast,parser,lexer}.rs` breaks the LSP** → the engine flip must
LEAVE them compiling (dead) until the LSP is migrated. Migration = repoint these onto
`gingembre_syntax::{parse, ast::{Expr, Item, ...}}` (typed views are shape-compatible), and
either port the `semantic` index to walk the CST or replace it. Only after the LSP builds on
`gingembre-syntax` do we delete the old front-end (final part of step 6/7). The LSP gains
shortcode/frontmatter awareness here too (see `notes/lsp-awareness.md`, workstream 7b) since
it now shares the real parser.

### Notes for the eval/render port (step 5)
- `gingembre-syntax::ast::Expr` + `Stmt`-equivalents (template items) are the input. Need
  to add typed views for the *statement/template* nodes too (IfStmt/ForStmt/SetStmt/BlockStmt/
  MacroStmt/Interpolation/Body/etc.) — only expressions are done so far.
- Whitespace-control trimming (`{%- -%}`) is applied when reading Text runs adjacent to trim
  delimiters (the CST preserves them losslessly; trimming happens in the typed Text accessor
  or the render walk).
- `loop.*` is just field access on `loop`; the engine must expose `loop` in for-context.
- Keep the engine's `LazyValue`/resolution/host-fn machinery; only the *source of the tree*
  changes (typed CST instead of owned AST).
