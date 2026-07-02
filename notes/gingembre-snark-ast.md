# snark grammar → generated Facet AST (work note)

Goal: prove **grammar (via snark) is the single source of truth for the AST**. Annotate
nodes in `grammar.js` via a DSL helper; snark parses into concrete generated
`#[derive(Facet)]` Rust types (for compiler authors) OR generic `RuntimeResolvedNode`
(for editor). Nothing hand-rolled — the ergonomics live in grammar annotations.

## Hard rules
- NO tree-sitter-cli. Derive node structure from snark's own model (`RawRuleJson` rules
  in snark/src/grammar.rs; `public_node_kinds`/`visible_node_kinds`/`supertypes` in
  parser.rs). snark REPLACES tree-sitter.
- facet, not serde. cargo nextest. Work in facet-cc (it's already a worktree).

## DONE — full goal met (`--ast` proof, branch snark-playground-rebased)
Run: `cargo run -p gingembre-snark-spike -- --ast`. Four items, all green + oracle'd:
1. **Codegen FOR REAL** — `gingembre-snark-spike/build.rs` emits grammar.json (snark-dsl
   `emit_with_boa`) + annotations (`annotations_from_source` over `gingembre_ast.snark.js`),
   derives node STRUCTURE from the grammar rules in-snark (walks `RawRuleJson`: `_expr`
   CHOICE members → enum variants; each node's SEQ → ordered Expr/Token slots; NO
   tree-sitter-cli), and WRITES `$OUT_DIR/gingembre_ast.rs`. main.rs `include!`s it in
   `mod gen_ast`. Hand-written PExpr/PBin DELETED. Generated: `enum Expr { Binary(Box<Binary>),
   Number(i64), Variable(String) }` + `struct Binary { left: Expr, op: String, right: Expr }`.
2. **Materialize THROUGH WEAVY OPS** — `build()` records a flat `BuildOp` program; `apply_op`
   is the single op-semantics fn shared by interpreter + JIT.
3. **IT JITS** — `run_ops_via_weavy_jit`: each op = a copied `HOSTCALL` stencil in one native
   chain (weavy::jit, copy-and-patch, `NATIVE_COPY_PATCH_AVAILABLE`=true on macos-aarch64),
   patched site→site, dispatching to `jit_apply` which moves the `Partial`. Same substrate as
   facet-json `from_str_weavy_jit`. Identical AST out of interpreter + JIT.
4. **GINGEMBRE SEMANTICS on the generated AST** — `gen_expr_to_gingembre` lowers `gen_ast::Expr`
   → `gingembre::ast::Expr`, rendered through gingembre's real evaluator; oracle byte-identical
   vs gingembre native parse+render (3,7,5,5,26,true,42,43).
Commits: 64925e9a5 (codegen), 106741db0 (JIT), 39d4aaad6 (semantics). Enrichment source of
truth = `gingembre-snark-spike/gingembre_ast.snark.js` (drives BOTH build.rs codegen and the
runtime builder; field TYPES derived from grammar, only names/renames/scalar-decode in the file).
Only the expression subset (binary/number/variable/literal) is annotated; extend the `.snark.js`
+ `derive_slots` for more node kinds (paren/filter/call/if/for) when needed.

## Perf / diagnostics / two-AST (measured — commits 5723f10fe, cab686228)
- `--perf`: snark-CST parse is **~15-25x slower** than gingembre's native parser
  (24-48us vs 1-2us). Plan setup one-time ~118ms. Materialize `1+2*3`: reflection 1638ns,
  weavy-interp 1261ns, JIT compile+run 5429ns (loses — recompiles per call), **JIT
  compiled-once/run-many 889ns (fastest)** + compile 3625ns one-time. JIT IS a win but only
  when reused; ops currently bake in DATA so the native program isn't reused. Real win =
  structure compiled once per grammar-shape, data in prog-stream slots (facet-json per-TYPE
  model). Parse is the dominant cost, not materialize.
- `--diag`: snark hard-errors with the EXACT expected-terminal set + byte pos (richer data
  than gingembre, but raw regexes/byte offsets/no construct context); gingembre gives
  construct-aware human prose ("unclosed if, expected endif") but coarser and sometimes wrong.
  snark = better data, gingembre = better prose. Not reconciled.
- `--eval`: **evaluation ALSO lowers to Weavy.** Was two ASTs (gen_ast::Expr lowered into
  gingembre::ast, walked by gingembre's async tree-walker). Now: ONE generated AST -> stack
  EvalOp program -> weavy interp AND copy-and-patch JIT, oracle'd vs gingembre eval_expression
  (int arith/cmp/logical/var). int-arith intrinsic is disposable stand-in. Next: text/if/for
  as emit + control-flow ops (weavy blocks) for full render; and expose gingembre's Value ops
  as the intrinsics instead of the stand-in.

## Type specialization — V8 SMI story (commit e27ceb698, `--specialize`)
When the generated AST proves a subtree is integer (number leaves + arithmetic ops — a type
fact the grammar annotations already carry: `number`->i64), lower to UNBOXED i64 ops over an
i64 stack, not boxed `Value` ops. No Value construction, no as_number()/re-box, no tag
dispatch. Measured (JIT run-only, 23-op expr, 1M iters): boxed 437ns vs unboxed **59ns ≈ 7x**.
Result oracle'd vs gingembre (71). Variables = not-statically-known → guard+deopt via weavy
branch chain (speculate SMI, deopt to boxed on miss) + inline-cache-style feedback = next.
This is the reason eval-on-weavy matters: monomorphic specialized stencils = the fast path.

## Done (earlier)
- `fedb4794a` (branch snark-playground-rebased): snark-dsl `ast({...})` DSL helper +
  `emit_source_with_annotations_boa(src,name) -> (grammar_json, annotations_json)`.
  Annotations keyed by node kind (`as`/`decode`/`drop`/`enum`/`fields`), captured in a
  side registry, grammar.json stays 100% standard tree-sitter. Test:
  `cargo nextest run -p snark-dsl --features boa ast_annotation`.

## Build (5-node proof: interpolation, binary, literal, if_statement, call)
1. node-types derivation in-snark: walk RawRuleJson → per node: fields (FIELD rules),
   children, cardinality (Optional/Choice+Blank → Option, Repeat/Repeat1 → Vec), enum
   from supertypes.
2. codegen node-types × annotations → `#[derive(Facet)]` structs/enums/decoded leaves
   (`decode:"i64"` → i64 via named fn; string → unescaped).
3. **reflection builder** `parse_into::<T: Facet>(…, src)`: walk RuntimeResolvedNode
   (kind/field/children/text) + use T's facet Shape to route kind→struct/variant,
   children→fields (Option/Vec), leaf text→scalars. Model on facet-json deserialize.
Acceptance: annotated grammar → codegen'd Facet types → parse_into → passing #[test];
finding = was the annotation vocabulary expressive enough for all 5.

## Refs
- gingembre-snark-spike/src/main.rs — correct/current snark prepare+parse+resolved-tree
  usage (39/0 render-oracle green; 57/57 real templates parse via `--corpus`).
- facet-json source = the template for Shape-driven materialization (parse_into).
- gingembre grammar.js: playgrounds/snark/src/bundled/gingembre/grammar.js.
