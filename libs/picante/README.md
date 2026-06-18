**picante** is an **async** incremental query runtime for Rust, inspired by Salsa but built for **Tokio-first** pipelines (like Dodeca).

**What it has today**

- Inputs (`InputIngredient<K, V>`), interning (`InternedIngredient<K>`), and derived async queries (`DerivedIngredient<DB, K, V>`)
- Dependency tracking via Tokio task-locals
- Per-task cycle detection (fast path)
- Async single-flight memoization per `(kind, key)`
- In-flight query deduplication across database snapshots (see below)
- Cache persistence to disk (snapshot file) using `facet` + `facet-postcard` (**no serde**)
- Runtime notifications for live reload (`Runtime::subscribe_revisions`, `Runtime::subscribe_events`)
- Debugging and observability tools (`picante::debug`): dependency graph visualization, query execution tracing, cache statistics, enhanced cycle diagnostics

## Quickstart (minimal)

```rust
use picante::{DerivedIngredient, DynIngredient, HasRuntime, IngredientLookup, IngredientRegistry, InputIngredient, QueryKindId, Runtime};
use std::sync::Arc;

#[derive(Default)]
struct Db {
    runtime: Runtime,
    ingredients: IngredientRegistry<Db>,
}

impl HasRuntime for Db {
    fn runtime(&self) -> &Runtime {
        &self.runtime
    }
}

impl IngredientLookup for Db {
    fn ingredient(&self, kind: QueryKindId) -> Option<&dyn DynIngredient<Self>> {
        self.ingredients.ingredient(kind)
    }
}

fn main() -> picante::PicanteResult<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let db = Db::default();

    let text: Arc<InputIngredient<String, String>> =
        Arc::new(InputIngredient::new(QueryKindId(1), "Text"));

    let len: Arc<DerivedIngredient<Db, String, u64>> = {
        let text = text.clone();
        Arc::new(DerivedIngredient::new(QueryKindId(2), "Len", move |db, key| {
            let text = text.clone();
            Box::pin(async move {
                let s = text.get(db, &key)?.unwrap_or_default();
                Ok(s.len() as u64)
            })
        }))
    };

        text.set(&db, "a".into(), "hello".into());
        assert_eq!(len.get(&db, "a".into()).await?, 5);

        Ok(())
    })
}
```

## Persistence (snapshot cache)

picante can save/load inputs and memoized derived values (including dependency lists):

```rust
use picante::persist::{load_cache, save_cache};

// save_cache(path, db.runtime(), &[&*text, &*len]).await?;
// load_cache(path, db.runtime(), &[&*text, &*len]).await?;
```

If you need cache size limits or custom corruption handling, use:
- `save_cache_with_options`
- `load_cache_with_options`

## In-flight query deduplication

When multiple concurrent async tasks request the same tracked query with identical parameters, picante automatically coalesces these into a single computation:

- **First caller becomes the leader** and starts computing the value
- **Concurrent callers become followers** and await the leader's result
- **All callers receive the same result** once computation completes
- **Each caller still caches the result in its own memo table**, keeping snapshot caches independent

This deduplication works across database snapshots from the same parent database. Coalescing is scoped to:
- The same **database identity** (parent database and all its snapshots share an identity)
- The same **revision** (different revisions compute independently)
- The same **query kind and key**

This is particularly useful for request-per-snapshot patterns where many concurrent requests may query the same data.

## Debugging and Observability

picante provides comprehensive debugging tools in the `picante::debug` module:

### Dependency Graph Visualization

Export the dependency graph in Graphviz DOT format for visualization:

```rust,no_run
# use picante::{Runtime, HasRuntime};
# struct Db { runtime: Runtime }
# impl HasRuntime for Db { fn runtime(&self) -> &Runtime { &self.runtime } }
# fn main() -> std::io::Result<()> {
# let db = Db { runtime: Runtime::new() };
use picante::debug::DependencyGraph;

let graph = DependencyGraph::from_runtime(db.runtime());
graph.write_dot("deps.dot")?;

// Visualize with: dot -Tpng deps.dot -o deps.png
# Ok(())
# }
```

### Query Execution Tracing

Record and analyze query execution events:

```rust,no_run
# use picante::{Runtime, HasRuntime};
# struct Db { runtime: Runtime }
# impl HasRuntime for Db { fn runtime(&self) -> &Runtime { &self.runtime } }
# fn main() {
# let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
# rt.block_on(async {
# let db = Db { runtime: Runtime::new() };
use picante::debug::{TraceCollector, TraceAnalysis};

let collector = TraceCollector::start(db.runtime());
// ... perform queries ...
let trace = collector.stop().await;

let analysis = TraceAnalysis::from_trace(&trace);
println!("{}", analysis.format());
// Shows: total events, input changes, invalidations, recomputations, duration
# });
# }
```

### Cache Statistics

Get insights into cache usage and dependency structure:

```rust,no_run
# use picante::{Runtime, HasRuntime};
# struct Db { runtime: Runtime }
# impl HasRuntime for Db { fn runtime(&self) -> &Runtime { &self.runtime } }
# fn main() {
# let db = Db { runtime: Runtime::new() };
use picante::debug::CacheStats;

let stats = CacheStats::collect(db.runtime());
println!("{}", stats.format());
// Shows: forward deps, reverse deps, total edges, root queries, distribution
# }
```

### Enhanced Cycle Diagnostics

When a dependency cycle is detected, picante shows the full path:

```text
dependency cycle detected
  → kind_1, key_0000000000000001  (initial)
  → kind_2, key_0000000000000002
  → kind_3, key_0000000000000003
  → kind_1, key_0000000000000001  ← cycle (already in stack)
```

## Notes

- Tokio-only: query execution uses Tokio task-local context.
- Global invalidation v1: changing any input bumps a single global `Revision`.
