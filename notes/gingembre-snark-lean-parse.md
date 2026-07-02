# Lean deterministic parse→AST for snark — design + handoff to PL core

Author: AST/CST agent (facet-cc worktree). Audience: PL core (facet worktree, owns
`snark/src/lower/weavy.rs`). Goal: close the ~15–25× parse gap with a lean fast path, keeping
the current rich GLR parse as the error/ambiguity fallback. Everything below is prototyped in
`gingembre-snark-spike` (facet-cc) — run the `--*` modes cited.

## 1. Evidence (why) — `gingembre-snark-spike --profile`

For a representative gingembre template:
- **Tree materialization is cheap** (`accepted_resolved_tree` ≈ 0–4 µs). NOT the bottleneck.
- **The parse loop is the cost**: ~300 ns/step. `{{1+2*3}}` = 106 steps/30 µs; if/else = 180/62 µs; for = 234/87 µs.
- **`fork` (max_live_versions) ≈ 1** for valid input (if/for peak at 2, one transient decision).
  So the GLR multi-branch machinery is unused overhead on the common path.
- `branch.clone()` does **not** fire every step — only at genuine conflicts (`actions.len() > 1`);
  the deterministic path passes the branch by value into `run_runtime_weavy_action`.
- What IS unconditional: `RuntimeWeavyOutput` holds concrete `trace_events: &mut Vec<TraceEvent>`
  and `tree_events: &mut Vec<TreeEvent>`. The driver pushes **~91 trace + ~17 tree events** for a
  13-char template, always. `trace_events` (ParseStart/StateEnter/GlrSplit/GlrRetire/ParseFinish)
  are pure parser-debug observability — separable from `tree_events` (the actual structure). The
  `EffectResource::Sink("tree_events")` markers in the lowered IR are bypassed by the concrete Vecs.

## 2. Architecture: ONE parameterizable backend, not two lowerings

Do **not** fork into a separate lean parser. Parameterize the existing one.

Replace the concrete `trace_events`/`tree_events` Vecs in `RuntimeWeavyOutput` with a generic
`B: ParseBackend`:

```rust
trait ParseBackend {
    fn open_node(&mut self, kind: PublicNodeKindId, named: bool);
    fn token(&mut self, kind: ParserSymbol, bytes: ByteRange, named: bool);
    fn reduce(&mut self, production: ProductionId, child_count: usize);
    fn close_node(&mut self, public: Option<PublicNodeKindId>, bytes: ByteRange);
    fn trace(&mut self, _e: TraceEvent) {}   // DEFAULT NO-OP — the free win
}
```

Monomorphized, a **null `trace`** makes the ~91 debug events/parse vanish for free — and it helps
EVERY consumer (highlighting, AST, tools), not just AST. `tree_events` become backend calls:
- **null/report backend** → collect into Vecs (today's behaviour) for the playground/diagnostics.
- **AST backend** → `reduce` drives the AST materializer (§5). This is parse+materialize fused.
- **highlight backend** → record kind + span.

GLR subtlety: you can't feed a backend speculatively (a forked branch's reduces may be discarded).
Keep the internal per-branch journal; at `accept`, **replay the winning lineage**
(`tree_events_for_version_lineage` already does exactly this) INTO the backend as one clean linear
stream. The backend never sees speculation; forking stays internal. "Redo with full trace on error"
= re-run with the collecting backend swapped in — same lowering, different type param.

## 3. The lean deterministic driver (the fast path)

For valid, unambiguous input (the common case), skip GLR entirely: one mutable LR stack, emit the
AST straight from reduces, bail to the rich parse on any real conflict or error.

```
State: stack: Vec<StackEntry { state: ParseStateId, ast: NodeBuilder }>
cursor = 0
loop:
    s   = stack.last().state
    tok = LEX(input, cursor, table.states()[s].lex_mode())        // <-- only missing snark API (§4)
    match table.states()[s].entries().action_for(tok.symbol):     // entries() is PUBLIC
        None                      => BAIL_TO_RICH                  // parse error → recovery+diagnostics
        Some(actions) if len > 1  => BAIL_TO_RICH                  // real GLR conflict
        Some(Shift(s'))           => { stack.push({s', ast: Leaf(tok)}); cursor = tok.end }
        Some(Reduce(prod))        => {
            n        = prod.rhs_len
            children = stack.pop_n(n)                              // their NodeBuilders
            kind     = prod.public_node_kind                       // via public_node_kinds()
            node     = BUILD_NODE(kind, children)                  // §5 — the materializer (prototyped)
            g        = table.states()[stack.last().state].gotos().goto_for(prod.lhs)  // gotos() PUBLIC
            stack.push({ g, ast: node })
        }
        Some(Accept)              => return stack.last().ast
```

`BAIL_TO_RICH` = call the existing `parse_prepared_weavy_with_report(...)` (full GLR + recovery +
diagnostics). The lean pass is pure speculation on "deterministic + well-formed"; on a miss you
lose the lean work and pay the rich parse once — fine, it's the uncommon case.

## 4. What snark must expose — SMALLER than expected

The LR table is **already public**:
- `ParseTable::states()` → `&[ParseState]`, and `ParseState::{entries() -> &[TableEntry],
  gotos() -> &[GotoEntry], lex_mode() -> LexModeId}`.
- `ParserGrammar::public_node_kinds()` → `&[PublicNodeKind]`.
- Productions carry rhs length + the reduced rule → `public_node_by_rule` maps rule → node kind.

So the driver's table/goto/production side needs **no new API** — just thin `action_for(term)` /
`goto_for(nonterm)` lookups over `entries()`/`gotos()`.

The ONE gap: a clean **lex-one-token** entry. Today lexing lives inside
`run_runtime_weavy_state_probe` (branch-coupled). Expose:

```rust
pub fn lex_one(plan: &WeavyParsePlan, table: &ParseTable, input: &str, cursor: usize,
               mode: LexModeId) -> Result<LexToken, LexError>;   // token symbol + byte range
```

That's the whole ask. (External scanners: the lean lexer can bail to rich when a state needs the
external scanner, if you don't want to thread the scanner host through the fast path initially.)

## 5. BUILD_NODE — the reduce→AST materializer (PROTOTYPED, working code)

The AST construction is fully decoupled and proven in the spike. It needs only this interface —
which a lean driver's `NodeBuilder` implements directly (kind + text + ordered children, no
sexp/tree_store/events):

```rust
trait ParseNode: Sized {
    fn kind(&self) -> &str;
    fn named(&self) -> bool;
    fn text(&self) -> Option<&str>;
    fn children(&self) -> &[Self];
}
```

`gingembre-snark-spike --fuse` builds the generated AST over BOTH the rich `RuntimeResolvedNode`
and a lightweight `LeanNode` and asserts they're **byte-identical**. So `reduce` → build a
`LeanNode`-shaped node → feed the same `build()`.

The materializer itself (spike `fn build`) is Shape-driven: dispatch on the target facet `Shape`
(the generated type supplies structure), the node supplies data, the grammar annotations supply
the mapping:
- enum target → `select_variant_named(node.kind()→variant)` + `begin_nth_field(0)` + recurse
- struct target → per field: `select_child(node, selector)` + `begin_field` + recurse
- scalar target → `set` from `leaf_text` (i64/String)
- `Box<T>` → `begin_smart_ptr` + recurse
It records a flat `BuildOp` program = the weavy materialization IR (interp today; see §7).

The AST TYPES are generated, not hand-written: `gingembre-snark-spike/build.rs` walks the grammar
(`RawRuleJson`) in-snark — `_expr` CHOICE members → enum variants; each node's SEQ → ordered
Expr/Token slots — and writes `$OUT_DIR/gingembre_ast.rs`. Structure comes from the grammar;
`gingembre_ast.snark.js` (the `ast()` DSL) adds only names/renames/scalar-decode. One annotation
file drives BOTH codegen and the runtime builder.

## 6. Perf expectations

Lean fast path removes: per-step observability (91 trace events), sexp/tree_store construction,
GLR VecDeque/version/HashMap bookkeeping, and the resolved-CST + walk (fused into reduce). No
clone on the deterministic path. Target: close most of the 15–25× on valid input; broken/ambiguous
input falls back to rich (unchanged behaviour + diagnostics).

## 7. Stencils are SECONDARY (don't chase them first) — `--specialize`

Copy-and-patch stencils win big ONLY for tiny ops (proved: `IntOp` add = 2.4× over hostcall, 17×
over boxed; guarded-var speculation = 6 ns / ~396× over the full evaluator). The current
`SnarkIntrinsic`s (Lex/DispatchActions/Reduce) are HEAVY — dispatch is a rounding error next to the
body, so per-op stencils give ~nothing (this is why the native-hostcall lane regressed 85 vs 75 ms
on JSON). The stencil jackpot opens only AFTER this lean redesign makes ops tiny (shift = push a
state; reduce = emit a build-op) — THEN they're IntOp-sized and stencils pay. So: lean + null-trace
FIRST, stencils LATER. (Also: hostcall ≠ copy-and-patch. Real copy-and-patch = dedicated per-op
stencils via `copypatch::extract`, mirroring phon-jit's 63-stencil lane; `become` tail-calls via
`RUSTC_BOOTSTRAP=1` on stable, no nightly install.)

## 8. Sequencing for PL core
1. `RuntimeWeavyOutput` → generic `B: ParseBackend`; null-trace on the Direct lane. Biggest free
   win, helps every consumer. (Front half of your Metered/Direct split already exists.)
2. `lex_one` entry (§4) + the deterministic driver (§3), bail-to-rich on conflict/error.
3. AST backend = `ParseNode`/`build()` from the spike (§5) as the `reduce` sink.
4. LATER: tiny-op stencils once ops are lean.

## Refs
- Spike (facet-cc `gingembre-snark-spike`): `--profile` (parse cost), `--fuse` (ParseNode decoupling
  proof), `--ast` (generated AST + materialize + gingembre semantics), `--specialize`/`--speculate`
  (copy-and-patch + type speculation). Commits: 24a4f7c15, 70373bbf9, 8afbe7337, 3c092c264, ad701c1e7.
- `notes/gingembre-snark-ast.md` — the AST/codegen/JIT workstream log.
