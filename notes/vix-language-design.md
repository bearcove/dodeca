# Vix language design — decisions + open forks (from design sessions with Amos)

Vix = the language. Vixen = the build-system technology around it. This note is the
compaction-resilient record of the design dialogue. Amos: "keep asking questions, it jogs my
memory." DO NOT read ~/vixenware/vixen (fifth restart; old notes/primitives there would anchor
on decisions since reversed — Amos's explicit instruction). Two-agent protocol: one agent on
proprietary primitives, THIS agent on the open language; all communication routed through Amos
(enforces review + separation).

## What vix is
Full statically-typed programming language, Rust-inspired. You write imperative-LOOKING code
that builds the project; the code IS the build graph (control flow lives in the graph, not a
staging phase). Selection at the CLI/API boundary back-propagates demand. Fully lazy: calling a
function + projecting `.field[0]` composes lenses/projections — no evaluation forced until the
edge demands. Must be a DROP-IN CARGO REPLACEMENT: 100% compatible incl. lockfiles (resolution
result may differ; on-disk files extremely compatible; target/ exempt).

## Decided
- **Monadic/dynamic deps**: dep sets can depend on build-output CONTENT. Scheduler discovers
  the graph while running; JIT compiles SUSPENDABLE nodes.
- **Demand reaches INTO function bodies** (fine-grained thunks) — "that's the whole thing" —
  but likely auto-AGGREGATE regions via cost heuristics (granularity = compiler decision;
  open: exact heuristics; our stencil measurements provide real cost inputs).
- **Streams**: pull-based; if something is pulling, data arrives immediately (rustc .rmeta
  mid-compile → downstream fires). Warm cache = replay in order. No downstream demand →
  backpressure to executor. (More stream design work existed; Amos forgets details — re-derive
  together.)
- **Identity**: everything keyed by content hashes of inputs; files live in Merkle trees,
  hashed recursively, max early cutoff. Remote file → send back as tree (hash first) over a
  channel. Closure's canonical-AST hash JOINS the memo key (edit code → invalidate its execs).
- **Caching invariant**: conservative. False negatives fine ("rebuild the universe one too many
  times"), false positives NEVER.
- **Ambient toolchains** (can't legally redistribute): NOT an executor property ("that's what
  bazel does and I fucking hate it"). A **vixen daemon** runs on each executor host and
  ADVERTISES CAPABILITIES; local-toolchain access is a capability. Capabilities: route jobs to
  matching executors; discoverable within your ACCESS LEVEL; and the daemon is RESPONSIBLE for
  advertised invariance — e.g. fs-watches every file of an MSVC install and POISONS in-flight
  builds using that capability if anything changes. (Amos: "other aspects not covered by my
  answer" — more to jog.)
- **Registry/package-manager as vix code**: registry layout, crate listing/search/extraction,
  manifest→constraint interpretation all written IN VIX; the SOLVER is an engine that reaches
  BACK into vix code ("give me metadata for this node" — solver doesn't know it's a crate
  manifest). Looked at PubGrub; wants parallel solving; decomposition into primitives not
  remembered — re-derive. Changing registry: had a solution — "discovered facts that are then
  locked and can be upgraded" (generalized lockfile; to be re-explored in dialogue).
- **Metadata locus resolved graph-side**: laziness state + provenance attach to GRAPH NODES +
  facet paths (columnar sidecar), NOT to values. Values are plain facet/Rust values so
  intrinsics/primitives are written as natural Rust (not node-API style). Provenance uses:
  supply-chain audit, cache explanation, errors — all graph-side. (Sub-value taint through
  opaque Rust primitives: pending — Amos asked what taint means; likely non-goal.)
- **fable/vix lowering**: fable becomes SUSPENDABLE by leaning fully into Rust-style async
  (goal for weavy anyway — file/network I/O). Previous attempt was hand-rolled coroutines +
  scheduler/net-driver message passing = too complicated; decision: real Rust async style.
  Then vix CAN lower through fable (fable = boring imperative debug-friendly middle IR).
- **What ships to executors**: canonical AST of the CLOSURE (everything needed): args, env,
  plus an OBSERVER CLOSURE holding the process handle, able to return anything incl. streams.
  Executor runs the observer (needs a runtime); wire format is part of the OPEN contract.
- **Open/proprietary split**: vix language fully open in the facet monorepo (co-founder
  approved): design, parsing, types, lowering, JIT, graph analysis. Proprietary (cloud
  product): VFS, sandboxing, the primitives, version-solving engine. Bring-your-own-runtime is
  legitimate; bearcove's runtime = the trusted/premium/subsidized-for-OSS option.

## Open forks (to resolve in dialogue)
1. Effect system: hazy; Amos went back and forth. Design discussion needed.
2. Loop/iteration semantics under demand (fan-out): being walked through now.
3. Async surface in VIX itself: implicit (demand IS await) vs explicit syntax — my strong prior
   is implicit; fable/weavy carry the async machinery.
4. Aggregation × memoization: does aggregating thunks change CACHING (memo only at aggregate
   boundaries) or only execution?
5. Language-vs-runtime concern split for identity subtleties (toolchains etc.) — partially
   answered by capabilities; more decisions to jog.
6. Sub-value provenance (taint) through opaque primitives: likely declare non-goal.
7. Purity: presumably all vix code is pure w/ effects only via primitives — confirm.

## Heritage (context)
Snark (tree-sitter fork), weavy (substrate), vox RPC (built on phon), phon (binary serde with
schemas+evolution) are v2/3/4/7 of a line of work going back ~18 years. The past two days'
stack (grammar→generated AST, stencil JIT, speculation/IC, DWARF debug, stax profiling, phon
serialization, derived diagnostics) is the intended foundation. Fifth restart of vixen; prior
failures = LLM code sprawl ignoring design constraints, architect losing track. WORKING RULES:
small slices, constraints written before code, Amos stays legible as architect, no sprawl.
