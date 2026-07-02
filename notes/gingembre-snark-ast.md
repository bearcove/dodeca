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

## Parse gap root-caused (commit 24a4f7c15, `--profile`)
Hypothesis (Amos): parser collects too much info instead of efficiently building the AST — and
"parse to AST should be a DIFFERENT LOWERING, falling back to rich parse on errors." Confirmed:
- Tree materialization (resolved tree) is CHEAP (~0-4us). NOT the bottleneck.
- Parse loop is the cost: ~300ns/step. `{{1+2*3}}`=106 steps/30us, if/else=180/62us, for=234/87us.
- **fork (max_live_versions) ≈ 1** for valid input (if/for peak at 2, one transient decision) —
  so the GLR multi-branch machinery is pure overhead.
- `step_runtime_weavy_branch` does `branch.clone()` (full LR stack Vec + auto_close_stack +
  reusable_nodes + tree_journal) on ~every action, UNCONDITIONALLY.
- ~0.9 trace + ~0.15 tree events collected PER STEP unconditionally (91 trace/17 tree for a
  13-char template) + VecDeque + HashMap recovery-costs + version tracking.
Fix = lean single-stack parse->AST lowering: one mutable stack (no clone), emit AST build-ops
directly on reduce (fuse parse+materialize, no sexp/trace/tree events), GLR-fork only on real
table conflicts; rich parse (parse_prepared_weavy_with_report) kept only for error/ambiguity
fallback. Lives in SNARK (needs internal table actions + lexer program) = PL core's domain;
additive new entry point, doesn't touch the hot loop. Blocker for spike prototype: ParseTable
action/goto + lexer aren't public.

## REAL copy-and-patch JIT — "let it JIT" met (commit 70373bbf9, `--specialize`)
Earlier "JIT" claims were HOSTCALL chains (emit_hostcall only, 0 emit_stencil): each op is an
indirect call into interpreted Rust; copy-patch machinery used only to stitch the chain =
unrolled threaded interpreter, NOT copy-and-patch. Amos caught the conflation. facet-json = 2
trivial stencils + hostcall (beachhead). **phon-jit = the real reference: 63 dedicated stencils,
0 hostcalls**, built on the SAME weavy::jit infra (StencilLayout/patch_branch26) + copypatch::extract.
Now done for the unboxed IntOp lane, mirroring phon:
- `stencils/intop.rs`: extern C push/add/sub/mul, each tail-calls undefined `weavy_cont` (the
  BRANCH26 hole), immediates via `Ctx.prog`; done = lone ret.
- build.rs `build_intop_stencils`: `copypatch::extract::compile_object` (rustc --emit=obj -O
  panic=abort relocation-model=static) + `extract_stencil` -> $OUT_DIR/intop_stencils.rs (bytes+cont_relocs).
- main.rs `build_intop_native`: emit_stencil per op + patch_continuation(site+rel -> next), push
  immediates to prog stream, NativeProgram, run over an i64 stack. IntCtx repr(C) matches stencil Ctx.
Measured (23-op expr, JIT run-only, 1M iters): (a) boxed HOSTCALL ~430ns, (b) unboxed HOSTCALL ~60ns,
(c) unboxed COPY-AND-PATCH **~25ns** — 2.4x over hostcall, 17x over boxed. Result oracle'd = gingembre (71).
Mislabeled "copy-and-patch" comments on BuildOp/EvalOp lanes corrected to "hostcall chain".
Next for real speed: dedicated stencils for the guarded-var path (SMI guard+deopt), and for the
parse ops (shift/reduce) — same technique. Nightly `become` (--cfg tailcall) would make the chain
one stack frame of jumps (currently stable `call`).

## become guard + type speculation/deopt (commits 8afbe7337, 3c092c264, `--speculate`)
- **become guard**: stencils tail-call via a `cont!` macro = `become` under `--cfg tailcall`,
  compiled on STABLE via `RUSTC_BOOTSTRAP=1` in build.rs env (no nightly toolchain; contained to
  the build-time stencil obj). At -O bytes identical to stable call (LLVM already TCOs) -> pure
  correctness guard (a stencil that can't tail-call fails to compile vs silently `bl`-chaining).
  RUSTC_BOOTSTRAP verified working on rustc 1.95.0; phon-jit uses +nightly instead. facet-json
  uses neither (stable call, tailcall=false).
- **speculation + deopt** (the V8 SMI story, real): `stencils/guard.rs` = a conditional-branch
  stencil (cbz on the type tag) with TWO cont holes (fast `weavy_cont` / deopt `weavy_deopt`),
  extracted via `extract_stencil_n`. Generated AST -> guarded program (Variable -> Guard(idx)
  betting i64; Number -> Push). Bet holds: push unboxed, stay on the fast IntOp chain. Bet miss:
  set deopt flag, branch to exit; caller falls back to gingembre::eval_expression. Guard deopt
  holes + last op patch to DONE; fast holes chain linearly. Measured `x*3+1`: x=10 guard HELD ->
  6ns native (=31, == gingembre); x=2.5 guard MISSED -> deopt -> gingembre 8.5. Fast ~396x the
  full evaluator. SpecCtx/SpecVarSlot repr(C) match guard.rs Ctx/VarSlot; prog/sp layout-shared
  with IntCtx so guard+push+mul mix in one chain. VarSlot{is_int,value} is a stand-in for reading
  facet-value's tag inline (mechanism real; production guard tests the Value tag itself).
  Next: inline-cache feedback to choose which type to speculate; guards for float/string.

## Polymorphic inline cache + float lane (commit b14d7009a, `--ic`)
Added f64 stencils (fadd/fsub/fmul; push reused with f64-bit immediates) + guard_f64. VarSlot ->
{tag: i64(0)/f64(1), bits}. `build_ic_native(ops, ty)` compiles a program specialized to ONE
observed type: int lane (GUARD/ADD/MUL, i64 immediates) vs float lane (GUARD_F64/FADD/FMUL, f64-bit
immediates). `InlineCache` caches one NativeProgram per type profile: HIT -> run cached native code;
MISS -> compile for the new type, cache. Demo `x*3+1` over a mixed stream: int (compile once, then
HITs), fractional float (compile float lane, HIT), back to int (HIT) -> 2 cache entries, 4 hits,
2 compiles, all oracle'd to gingembre. cached HIT ~5ns vs cold compile+run ~3.5us (amortizes ~690
calls). The guard stencil IS the IC's type check (same cbz conditional-branch, now selecting the
cached specialization). tag_of uses to_i64 as a stand-in (whole floats -> int lane; production reads
the Value's real tag). Next: real facet-value tag in the guard; megamorphic cap -> permanent deopt;
key IC on the full type profile (multiple vars).

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
