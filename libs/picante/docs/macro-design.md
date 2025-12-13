# Picante Macro Design

## Goal

Provide `#[picante::input]` and `#[picante::tracked]` macros that generate the boilerplate for picante's ingredient system, similar to Salsa's ergonomics but async-first.

## Current status (implemented subset)

- `#[picante::tracked]`: implemented (requires `fn name<DB: ...>(db: &DB, ...) -> T | PicanteResult<T>`)
- `#[picante::input]`: implemented as `InternedIngredient<Key> + InputIngredient<InternId, Arc<Data>>`
- `#[picante::interned]`: implemented as `InternedIngredient<Data>`
- `#[picante::db]`: implemented (generates `Runtime`, `IngredientRegistry`, and all `Has*` trait impls)

---

## `#[picante::input]` - Input Structs

### Keyed Inputs

Inputs with a `#[key]` field create multiple instances indexed by that key:

```rust
#[picante::input]
pub struct SourceFile {
    #[key]
    pub path: SourcePath,

    pub content: SourceContent,
    pub last_modified: i64,
}
```

### Singleton Inputs

Inputs without a `#[key]` field are singletons — only one instance exists:

```rust
#[picante::input]
pub struct Config {
    pub debug: bool,
    pub timeout: u64,
}
```

Singletons generate a different API:
- `Config::set(&db, debug, timeout)` — set the singleton
- `Config::get(&db)` — returns `Option<Config>` (None if not set)

### Macro generates:

```rust
/// The input struct (just an ID wrapper for ergonomics)
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceFile(picante::InternId);

impl SourceFile {
    /// Get the path (key field)
    pub fn path<DB: HasSourceFileIngredient>(self, db: &DB) -> &SourcePath {
        db.source_file_ingredient().key(db, self.0)
    }

    /// Get the content
    pub fn content<DB: HasSourceFileIngredient>(self, db: &DB) -> &SourceContent {
        &db.source_file_ingredient().get_data(db, self.0).content
    }

    /// Get last_modified
    pub fn last_modified<DB: HasSourceFileIngredient>(self, db: &DB) -> i64 {
        db.source_file_ingredient().get_data(db, self.0).last_modified
    }
}

/// The actual data stored (non-key fields)
#[derive(Clone, facet::Facet)]
pub struct SourceFileData {
    pub content: SourceContent,
    pub last_modified: i64,
}

/// Trait for DBs that have this ingredient
pub trait HasSourceFileIngredient: picante::HasRuntime {
    fn source_file_ingredient(&self) -> &SourceFileIngredient;
}

/// The ingredient type
pub type SourceFileIngredient = picante::InputIngredient<SourcePath, SourceFileData>;

/// Builder for creating/updating SourceFile
impl SourceFile {
    pub fn new<DB: HasSourceFileIngredient>(
        db: &DB,
        path: SourcePath,
        content: SourceContent,
        last_modified: i64,
    ) -> Self {
        let data = SourceFileData { content, last_modified };
        let id = db.source_file_ingredient().set(db, path, data);
        Self(id)
    }

    /// Update content (invalidates dependents)
    pub fn set_content<DB: HasSourceFileIngredient>(self, db: &DB, content: SourceContent) {
        db.source_file_ingredient().update(db, self.0, |data| data.content = content);
    }

    /// Update last_modified (invalidates dependents)
    pub fn set_last_modified<DB: HasSourceFileIngredient>(self, db: &DB, last_modified: i64) {
        db.source_file_ingredient().update(db, self.0, |data| data.last_modified = last_modified);
    }
}
```

---

## `#[picante::tracked]` - Derived Queries

### User writes:

```rust
#[picante::tracked]
pub async fn parse_source<DB: Db>(db: &DB, file: SourceFile) -> picante::PicanteResult<ParsedData> {
    let path = file.path(db);
    let content = file.content(db);
    // ... parsing logic ...
    Ok(ParsedData { /* ... */ })
}
```

### Macro generates:

```rust
/// Trait for DBs that have this query
pub trait HasParseSourceQuery: HasSourceFileIngredient + picante::IngredientLookup {
    fn parse_source_query(&self) -> &ParseSourceQuery;
}

/// The query ingredient type
pub type ParseSourceQuery = picante::DerivedIngredient<dyn Db, SourceFile, ParsedData>;

/// The user-facing function (async, memoized)
pub async fn parse_source<DB: HasParseSourceQuery>(db: &DB, file: SourceFile) -> picante::PicanteResult<ParsedData> {
    db.parse_source_query().get(db, file).await
}

/// Query registration helper
pub fn register_parse_source(registry: &mut picante::IngredientRegistry<impl Db>) {
    registry.register(
        PARSE_SOURCE_QUERY_ID,
        Arc::new(picante::DerivedIngredient::new(
            PARSE_SOURCE_QUERY_ID,
            "parse_source",
            |db, file| Box::pin(parse_source_impl(db, file))
        ))
    );
}

/// The actual implementation (what the user wrote)
async fn parse_source_impl<DB: Db>(db: &DB, file: SourceFile) -> picante::PicanteResult<ParsedData> {
    let path = file.path(db);
    let content = file.content(db);
    // ... parsing logic ...
    Ok(ParsedData { /* ... */ })
}
```

---

## `#[picante::db]` - Database Definition

### User writes:

```rust
#[picante::db(
    inputs(SourceFile, TemplateFile),
    interned(CharSet),
    tracked(parse_source, render_template),
    db_trait(Db),
)]
pub struct Database {
    // Custom fields are forwarded to `Database::new(...)`
    pub config: Config,
}
```

**Options:**
- `inputs(...)` / `input(...)` — register input ingredients
- `interned(...)` — register interned ingredients
- `tracked(...)` — register tracked queries
- `db_trait(Name)` — generate a combined trait with all `Has*` bounds (defaults to `{StructName}Trait`)

### Macro generates:

```rust
pub struct Database {
    runtime: picante::Runtime,
    ingredients: picante::IngredientRegistry<Database>,

    // Generated ingredient fields
    source_file_keys: Arc<SourceFileKeysIngredient>,
    source_file_data: Arc<SourceFileDataIngredient>,
    template_file_keys: Arc<TemplateFileKeysIngredient>,
    template_file_data: Arc<TemplateFileDataIngredient>,
    char_set: Arc<CharSetIngredient>,
    parse_source: Arc<ParseSourceQuery<Database>>,
    render_template: Arc<RenderTemplateQuery<Database>>,

    // User's custom fields
    pub config: Config,
}

impl picante::HasRuntime for Database {
    fn runtime(&self) -> &picante::Runtime {
        &self.runtime
    }
}

impl picante::IngredientLookup for Database {
    fn ingredient(&self, kind: picante::QueryKindId) -> Option<&dyn picante::DynIngredient<Self>> {
        self.ingredients.ingredient(kind)
    }
}

// Generated trait impls
impl HasSourceFileIngredient for Database {
    fn source_file_ingredient(&self) -> &SourceFileIngredient {
        &self.source_file
    }
}

impl HasParseSourceQuery for Database {
    fn parse_source_query(&self) -> &ParseSourceQuery {
        &self.parse_source
    }
}

impl Database {
    pub fn new(config: Config) -> Self {
        let mut ingredients = picante::IngredientRegistry::new();

        // ... construct and register all ingredients ...

        Self {
            runtime: picante::Runtime::new(),
            ingredients,
            source_file_keys,
            source_file_data,
            template_file_keys,
            template_file_data,
            char_set,
            parse_source,
            render_template,
            config,
        }
    }
}

/// Combined trait for all ingredients (from db_trait(Db))
pub trait Db:
    HasSourceFileIngredient
    + HasTemplateFileIngredient
    + HasCharSetIngredient
    + HasParseSourceQuery
    + HasRenderTemplateQuery
    + picante::HasRuntime
    + picante::IngredientLookup
{
}

impl Db for Database {}
```

---

## `#[picante::interned]` - Interned Values

### User writes:

```rust
#[picante::interned]
pub struct CharSet {
    pub chars: Vec<char>,
}
```

### Macro generates:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CharSet(picante::InternId);

impl CharSet {
    pub fn new<DB: HasCharSetIngredient>(db: &DB, chars: Vec<char>) -> Self {
        Self(db.char_set_ingredient().intern(db, chars))
    }

    pub fn chars<DB: HasCharSetIngredient>(self, db: &DB) -> &Vec<char> {
        db.char_set_ingredient().lookup(db, self.0)
    }
}

pub trait HasCharSetIngredient: picante::HasRuntime {
    fn char_set_ingredient(&self) -> &CharSetIngredient;
}

pub type CharSetIngredient = picante::InternedIngredient<Vec<char>>;
```

---

## Key Design Decisions

1. **Trait-per-ingredient**: Each ingredient gets a `Has*` trait, enabling modular composition
2. **Combined trait**: `db_trait(Name)` generates a single trait with all bounds for ergonomic use
3. **Async by default**: All tracked queries are async (can use `.await` inside)
4. **Explicit registration**: QueryKindIds are assigned at compile time via macro
5. **Facet for persistence**: Data types derive `facet::Facet` for snapshot serialization
6. **Key field annotation**: `#[key]` marks which field is the lookup key for keyed inputs
7. **Singleton inputs**: Inputs without `#[key]` are singletons with `set`/`get` API

---

## Migration from Salsa

| Salsa | Picante |
|-------|---------|
| `#[salsa::db]` | `#[picante::db]` |
| `#[salsa::input]` | `#[picante::input]` |
| `#[salsa::tracked]` | `#[picante::tracked]` |
| `#[salsa::interned]` | `#[picante::interned]` |
| `salsa::Setter` | Generated `set_*` methods |
| Sync queries | Async queries (add `.await`) |
| `db.as_serialize()` | `picante::persist::snapshot()` |

---

## Open Questions

1. **Lifetime handling**: Should `#[picante::tracked]` support `'db` lifetimes for borrowed returns?
2. **Error handling**: Should queries return `Result<T>` or `PicanteResult<T>`?
3. **Cancellation**: How to handle query cancellation in async context?
4. **Parallel execution**: Should the macro generate `spawn` calls for independent queries?
