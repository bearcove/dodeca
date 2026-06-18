# Picante Macros: What They Generate

This doc is intended to be *mechanically accurate* with the current macro implementations in `crates/picante-macros`.

## Overview

picante provides:

- `#[picante::input]`: keyed and singleton inputs (stored in two ingredients: keys + data)
- `#[picante::interned]`: interning for values (one ingredient)
- `#[picante::tracked]`: async memoized derived queries (one ingredient)
- `#[picante::db]`: a db struct that owns a `Runtime`, an `IngredientRegistry`, and all generated ingredient fields + trait impls

All macro-generated kind ids use `QueryKindId::from_str(...)` to remain stable across builds as long as the path/name stays the same.

---

## `#[picante::input]`

### Keyed inputs

User writes:

```rust
#[picante::input]
pub struct SourceFile {
    #[key]
    pub path: String,
    pub content: String,
}
```

The macro generates (shape, simplified):

```rust
pub const SOURCE_FILE_KEYS_KIND: picante::QueryKindId = /* QueryKindId::from_str(...) */;
pub const SOURCE_FILE_DATA_KIND: picante::QueryKindId = /* QueryKindId::from_str(...) */;

pub type SourceFileKeysIngredient = picante::InternedIngredient<String>;
pub type SourceFileDataIngredient =
    picante::InputIngredient<picante::InternId, std::sync::Arc<SourceFileData>>;

#[derive(facet::Facet)]
pub struct SourceFileData {
    pub content: String,
}

pub trait HasSourceFileIngredient: picante::HasRuntime {
    fn source_file_keys(&self) -> &SourceFileKeysIngredient;
    fn source_file_data(&self) -> &SourceFileDataIngredient;
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, facet::Facet)]
#[repr(transparent)]
pub struct SourceFile(pub picante::InternId);

impl SourceFile {
    pub fn new<DB: HasSourceFileIngredient>(db: &DB, path: String, content: String)
        -> picante::PicanteResult<Self>
    {
        let id = db.source_file_keys().intern(path)?;
        db.source_file_data()
            .set(db, id, std::sync::Arc::new(SourceFileData { content }));
        Ok(Self(id))
    }

    pub fn path<DB: HasSourceFileIngredient>(self, db: &DB)
        -> picante::PicanteResult<std::sync::Arc<String>>
    {
        db.source_file_keys().get(db, self.0)
    }

    pub fn data<DB: HasSourceFileIngredient>(self, db: &DB)
        -> picante::PicanteResult<std::sync::Arc<SourceFileData>>
    {
        // Reads from the underlying InputIngredient; errors if missing/removed.
        /* ... */
    }

    pub fn content<DB: HasSourceFileIngredient>(self, db: &DB) -> picante::PicanteResult<String> {
        Ok(self.data(db)?.content.clone())
    }
}
```

Notes:

- `new(...)` overwrites the record for the same key (and bumps revision only if the value actually changes at the ingredient layer).
- Field getters return `PicanteResult<T>` and clone from an `Arc<...>`; they do not return references.
- Trait bounds are enforced by generated `const` assertions: keys and values must be `facet::Facet<'static> + Send + Sync + 'static` (and values must also be `Clone`).

### Singleton inputs

User writes:

```rust
#[picante::input]
pub struct Config {
    pub debug: bool,
    pub timeout: u64,
}
```

The macro generates a `struct Config;` with:

- `Config::set(&db, debug, timeout) -> PicanteResult<()>`
- `Config::get(&db) -> PicanteResult<Option<Arc<ConfigData>>>`
- Per-field convenience getters like `Config::debug(&db) -> PicanteResult<Option<bool>>`

Singletons still have *two ingredients* (keys + data); the key ingredient is `InternedIngredient<()>`.

---

## `#[picante::interned]`

User writes:

```rust
#[picante::interned]
pub struct CharSet {
    pub chars: Vec<char>,
}
```

The macro generates (shape, simplified):

```rust
pub type CharSetIngredient = picante::InternedIngredient<CharSetValue>;

#[derive(facet::Facet)]
pub struct CharSetValue {
    pub chars: Vec<char>,
}

pub trait HasCharSetIngredient: picante::HasRuntime {
    fn char_set_ingredient(&self) -> &CharSetIngredient;
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, facet::Facet)]
#[repr(transparent)]
pub struct CharSet(pub picante::InternId);

impl CharSet {
    pub fn new<DB: HasCharSetIngredient>(db: &DB, chars: Vec<char>) -> picante::PicanteResult<Self> { /* ... */ }
    pub fn value<DB: HasCharSetIngredient>(self, db: &DB) -> picante::PicanteResult<std::sync::Arc<CharSetValue>> { /* ... */ }
}
```

---

## `#[picante::tracked]`

User writes:

```rust
#[picante::tracked]
pub async fn line_count<DB: DatabaseTrait>(db: &DB, file: SourceFile) -> picante::PicanteResult<usize> {
    Ok(file.content(db)?.lines().count())
}
```

The macro generates (shape, simplified):

```rust
pub const LINE_COUNT_KIND: picante::QueryKindId = /* QueryKindId::from_str(...) */;

pub type LineCountQuery<DB> = picante::DerivedIngredient<DB, SourceFile, usize>;

pub trait HasLineCountQuery: picante::IngredientLookup + Send + Sync + 'static {
    fn line_count_query(&self) -> &LineCountQuery<Self>;
}

pub fn make_line_count_query<DB>() -> std::sync::Arc<LineCountQuery<DB>>
where
    DB: DatabaseTrait + picante::IngredientLookup + Send + Sync + 'static,
{
    std::sync::Arc::new(picante::DerivedIngredient::new(
        LINE_COUNT_KIND,
        "line_count",
        |db, key| Box::pin(async move {
            let file = key;
            __picante_impl_line_count(db, file).await
        }),
    ))
}

pub async fn line_count<DB>(db: &DB, file: SourceFile) -> picante::PicanteResult<usize>
where
    DB: HasLineCountQuery,
{
    db.line_count_query().get(db, file).await
}
```

Notes:

- If a tracked function has multiple parameters after `db`, the query key is a tuple `(A, B, ...)`.
- Return types can be either `T` or `PicanteResult<T>` (the macro wraps `T` into `Ok(T)`).

---

## `#[picante::db]`

User writes:

```rust
#[picante::db(inputs(SourceFile), tracked(line_count))]
pub struct Database {}
```

The macro generates:

- A concrete struct with reserved fields `runtime` and `ingredients`
- Generated ingredient fields (naming follows:
  - inputs: `{name}_keys_ingredient` and `{name}_data_ingredient`
  - interned: `{name}_ingredient`
  - tracked: `{fn}_query`)
- `impl HasRuntime` and `impl IngredientLookup` for the db struct
- `impl Has*` traits for each input/interned/tracked item
- A `new(...)` constructor that constructs and registers every ingredient in an `IngredientRegistry`
- A combined trait (defaults to `{StructName}Trait`, configurable via `db_trait(Name)`) that includes:
  - all generated `Has*` traits
  - `picante::HasRuntime`
  - `picante::IngredientLookup`

