# picante

[![Coverage Status](https://coveralls.io/repos/github/facet-rs/picante/badge.svg?branch=main)](https://coveralls.io/github/facet-rs/facet?branch=main)
[![crates.io](https://img.shields.io/crates/v/picante.svg)](https://crates.io/crates/picante)
[![documentation](https://docs.rs/picante/badge.svg)](https://docs.rs/picante)
[![MIT/Apache-2.0 licensed](https://img.shields.io/crates/l/picante.svg)](./LICENSE)
[![Discord](https://img.shields.io/discord/1379550208551026748?logo=discord&label=discord)](https://discord.gg/JhD7CwCJ8F)

[![Crates.io](https://img.shields.io/crates/v/picante.svg)](https://crates.io/crates/picante)
[![docs.rs](https://docs.rs/picante/badge.svg)](https://docs.rs/picante)
[![License](https://img.shields.io/crates/l/picante.svg)](https://github.com/bearcove/picante#license)
[![codecov](https://codecov.io/gh/bearcove/picante/graph/badge.svg)](https://codecov.io/gh/bearcove/picante)
![Experimental](https://img.shields.io/badge/status-experimental-yellow.svg)

**picante** is an **async** incremental query runtime for Rust, inspired by Salsa but built for **Tokio-first** pipelines (like Dodeca).

**What it has today**

- Inputs (`InputIngredient<K, V>`) and derived async queries (`DerivedIngredient<DB, K, V>`)
- Dependency tracking via Tokio task-locals
- Per-task cycle detection (fast path)
- Async single-flight memoization per `(kind, key)`
- Cache persistence to disk (snapshot file) using `facet` + `facet-postcard` (**no serde**)
- Runtime notifications for live reload (`Runtime::subscribe_revisions`, `Runtime::subscribe_events`)

## Quickstart (minimal)

```rust
use picante::{DerivedIngredient, HasRuntime, InputIngredient, QueryKindId, Runtime};
use std::sync::Arc;

#[derive(Default)]
struct Db {
    runtime: Runtime,
}

impl HasRuntime for Db {
    fn runtime(&self) -> &Runtime {
        &self.runtime
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> picante::PicanteResult<()> {
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

## Notes

- Tokio-only: query execution uses Tokio task-local context.
- Global invalidation v1: updating any input bumps a single global `Revision`.

## License

Licensed under either of Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0) or MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT) at your option.

## Sponsors

Thanks to all individual sponsors:

<p> <a href="https://github.com/sponsors/fasterthanlime">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/github-dark.svg">
<img src="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/github-light.svg" height="40" alt="GitHub Sponsors">
</picture>
</a> <a href="https://patreon.com/fasterthanlime">
    <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/patreon-dark.svg">
    <img src="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/patreon-light.svg" height="40" alt="Patreon">
    </picture>
</a> </p>

...along with corporate sponsors:

<p> <a href="https://aws.amazon.com">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/aws-dark.svg">
<img src="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/aws-light.svg" height="40" alt="AWS">
</picture>
</a> <a href="https://zed.dev">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/zed-dark.svg">
<img src="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/zed-light.svg" height="40" alt="Zed">
</picture>
</a> <a href="https://depot.dev?utm_source=facet">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/depot-dark.svg">
<img src="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/depot-light.svg" height="40" alt="Depot">
</picture>
</a> </p>

...without whom this work could not exist.

## Special thanks

The facet logo was drawn by [Misiasart](https://misiasart.com/).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](https://github.com/facet-rs/facet/blob/main/LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](https://github.com/facet-rs/facet/blob/main/LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
