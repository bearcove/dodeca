# Design: ULTIMATE Per-Value Lazy Data Loading

This document proposes **per-value granularity** for lazy data loading - every single value access is individually tracked by Salsa.

## The Vision

```jinja
{{ data.versions.dodeca.version }}
```

This should create a Salsa dependency on **exactly** `["versions", "dodeca", "version"]` - not the whole file, not even the whole `dodeca` object. Just that one value.

Change `data.versions.dodeca.version`? Only pages using that exact path re-render.
Change `data.versions.dodeca.name`? Pages using `.version` are untouched.

## Core Concept: Lazy Paths

Instead of resolving data eagerly, we track **paths** through the data tree. Resolution happens only when we need a concrete value (printing, comparison, iteration).

```
{{ data.versions.dodeca.version }}
         ↓
LazyValue { path: [] }
         ↓ .versions
LazyValue { path: ["versions"] }
         ↓ .dodeca
LazyValue { path: ["versions", "dodeca"] }
         ↓ .version
LazyValue { path: ["versions", "dodeca", "version"] }
         ↓ {{ ... }} needs to print
resolver.resolve(["versions", "dodeca", "version"])
         ↓
Salsa tracks THIS EXACT PATH as dependency
```

## Architecture

### 1. LazyValue Type (gingembre)

Replace direct `facet_value::Value` usage with a wrapper:

```rust
/// A value that may be lazily resolved.
///
/// Field access on lazy values extends the path rather than
/// immediately resolving. Resolution happens when a concrete
/// value is needed (printing, comparison, arithmetic, etc.)
#[derive(Clone)]
pub enum LazyValue {
    /// A concrete, already-resolved value
    Concrete(facet_value::Value),

    /// A lazy reference that will be resolved on demand
    Lazy {
        /// The resolver that can fetch values by path
        resolver: Arc<dyn DataResolver>,
        /// The path through the data tree (e.g., ["versions", "dodeca", "version"])
        path: DataPath,
    },
}

/// A path through the data tree
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DataPath(pub Vec<String>);

impl DataPath {
    pub fn root() -> Self {
        Self(Vec::new())
    }

    pub fn push(&self, segment: impl Into<String>) -> Self {
        let mut new_path = self.0.clone();
        new_path.push(segment.into());
        Self(new_path)
    }
}
```

### 2. DataResolver Trait (gingembre)

```rust
/// Trait for resolving data values by path.
///
/// Implementations should track dependencies - each unique path
/// that is resolved becomes a separate dependency.
pub trait DataResolver: Send + Sync {
    /// Resolve a value at the given path.
    /// Returns None if the path doesn't exist.
    fn resolve(&self, path: &DataPath) -> Option<facet_value::Value>;

    /// Get all child keys at a path (for iteration).
    /// Returns None if the path doesn't exist or isn't an object/array.
    fn keys_at(&self, path: &DataPath) -> Option<Vec<String>>;

    /// Get the length at a path (for arrays).
    fn len_at(&self, path: &DataPath) -> Option<usize>;
}
```

### 3. LazyValue Operations

```rust
impl LazyValue {
    /// Access a field - extends path for lazy values, normal access for concrete
    pub fn field(&self, name: &str) -> Result<LazyValue> {
        match self {
            LazyValue::Concrete(value) => {
                // Normal field access on concrete value
                match value.destructure_ref() {
                    DestructuredRef::Object(obj) => {
                        obj.get(name)
                            .cloned()
                            .map(LazyValue::Concrete)
                            .ok_or_else(|| /* error */)
                    }
                    _ => Err(/* type error */)
                }
            }
            LazyValue::Lazy { resolver, path } => {
                // Extend the path - don't resolve yet!
                Ok(LazyValue::Lazy {
                    resolver: resolver.clone(),
                    path: path.push(name),
                })
            }
        }
    }

    /// Index access - similar to field
    pub fn index(&self, idx: usize) -> Result<LazyValue> {
        match self {
            LazyValue::Concrete(value) => { /* normal index */ }
            LazyValue::Lazy { resolver, path } => {
                Ok(LazyValue::Lazy {
                    resolver: resolver.clone(),
                    path: path.push(idx.to_string()),
                })
            }
        }
    }

    /// Force resolution to a concrete value (for printing, comparison, etc.)
    pub fn resolve(&self) -> Result<facet_value::Value> {
        match self {
            LazyValue::Concrete(value) => Ok(value.clone()),
            LazyValue::Lazy { resolver, path } => {
                resolver.resolve(path)
                    .ok_or_else(|| /* undefined error */)
            }
        }
    }

    /// Check if truthy (forces resolution)
    pub fn is_truthy(&self) -> bool {
        self.resolve().map(|v| v.is_truthy()).unwrap_or(false)
    }

    /// Render to string (forces resolution)
    pub fn render_to_string(&self) -> String {
        self.resolve().map(|v| v.render_to_string()).unwrap_or_default()
    }

    /// Iterate over values (forces resolution of keys, lazy iteration of values)
    pub fn iter(&self) -> impl Iterator<Item = (String, LazyValue)> {
        match self {
            LazyValue::Concrete(value) => {
                // Normal iteration
                match value.destructure_ref() {
                    DestructuredRef::Object(obj) => {
                        obj.iter()
                            .map(|(k, v)| (k.to_string(), LazyValue::Concrete(v.clone())))
                            .collect::<Vec<_>>()
                            .into_iter()
                    }
                    DestructuredRef::Array(arr) => {
                        arr.iter()
                            .enumerate()
                            .map(|(i, v)| (i.to_string(), LazyValue::Concrete(v.clone())))
                            .collect::<Vec<_>>()
                            .into_iter()
                    }
                    _ => Vec::new().into_iter()
                }
            }
            LazyValue::Lazy { resolver, path } => {
                // Get keys at this path, but keep values lazy!
                let keys = resolver.keys_at(path).unwrap_or_default();
                keys.into_iter()
                    .map(|key| {
                        let child_path = path.push(&key);
                        (key, LazyValue::Lazy {
                            resolver: resolver.clone(),
                            path: child_path,
                        })
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
            }
        }
    }
}
```

### 4. Salsa Integration (dodeca)

The magic happens here. Each unique path becomes a separate Salsa query:

```rust
/// A path through the data tree, interned for efficient comparison
#[salsa::interned]
pub struct DataPathId<'db> {
    pub segments: Vec<String>,
}

/// Resolve a value at a specific path - THIS IS THE KEY QUERY
/// Each unique path is tracked separately by Salsa!
#[salsa::tracked]
pub fn resolve_data_path<'db>(
    db: &'db dyn Db,
    registry: DataRegistry,
    path: DataPathId<'db>,
) -> Option<Value> {
    let segments = path.segments(db);

    if segments.is_empty() {
        // Root path - return object with all file keys
        let mut obj = VObject::new();
        for file in registry.files(db) {
            let key = extract_filename_without_extension(file.path(db).as_str());
            // Don't load the content here - just create the structure
            obj.insert(VString::from(key), Value::NULL); // Placeholder
        }
        return Some(obj.into());
    }

    // First segment is the file name
    let file_key = &segments[0];
    let file = registry.files(db).iter()
        .find(|f| extract_filename_without_extension(f.path(db).as_str()) == *file_key)?;

    // Load and parse the file (tracked!)
    let parsed = load_data_file(db, *file)?;

    // Navigate to the specific path
    let mut current = parsed;
    for segment in segments.iter().skip(1) {
        current = match current.destructure_ref() {
            DestructuredRef::Object(obj) => obj.get(segment.as_str())?.clone(),
            DestructuredRef::Array(arr) => {
                let idx: usize = segment.parse().ok()?;
                arr.get(idx)?.clone()
            }
            _ => return None,
        };
    }

    Some(current)
}

/// Get child keys at a path (for iteration)
#[salsa::tracked]
pub fn data_keys_at<'db>(
    db: &'db dyn Db,
    registry: DataRegistry,
    path: DataPathId<'db>,
) -> Vec<String> {
    let value = resolve_data_path(db, registry, path);
    match value.as_ref().map(|v| v.destructure_ref()) {
        Some(DestructuredRef::Object(obj)) => {
            obj.keys().map(|k| k.to_string()).collect()
        }
        Some(DestructuredRef::Array(arr)) => {
            (0..arr.len()).map(|i| i.to_string()).collect()
        }
        _ => Vec::new()
    }
}
```

### 5. SalsaDataResolver

```rust
/// A data resolver backed by Salsa queries.
/// Each path resolution becomes a tracked dependency.
pub struct SalsaDataResolver<'db> {
    db: &'db dyn Db,
    registry: DataRegistry,
}

impl<'db> SalsaDataResolver<'db> {
    pub fn new(db: &'db dyn Db, registry: DataRegistry) -> Self {
        Self { db, registry }
    }

    fn intern_path(&self, path: &DataPath) -> DataPathId<'db> {
        DataPathId::new(self.db, path.0.clone())
    }
}

impl gingembre::DataResolver for SalsaDataResolver<'_> {
    fn resolve(&self, path: &DataPath) -> Option<facet_value::Value> {
        let path_id = self.intern_path(path);
        resolve_data_path(self.db, self.registry, path_id)
    }

    fn keys_at(&self, path: &DataPath) -> Option<Vec<String>> {
        let path_id = self.intern_path(path);
        let keys = data_keys_at(self.db, self.registry, path_id);
        if keys.is_empty() { None } else { Some(keys) }
    }

    fn len_at(&self, path: &DataPath) -> Option<usize> {
        self.keys_at(path).map(|k| k.len())
    }
}
```

## Context and Evaluator Changes

### Context stores the resolver

```rust
pub struct Context {
    scopes: Vec<HashMap<String, LazyValue>>,  // Changed from Value
    global_fns: Arc<HashMap<String, Arc<GlobalFn>>>,
    data_resolver: Option<Arc<dyn DataResolver>>,
}

impl Context {
    /// Set up lazy data access
    pub fn set_data_resolver(&mut self, resolver: Arc<dyn DataResolver>) {
        // Create a lazy "data" variable pointing to root
        self.set("data", LazyValue::Lazy {
            resolver: resolver.clone(),
            path: DataPath::root(),
        });
        self.data_resolver = Some(resolver);
    }
}
```

### Evaluator uses LazyValue

```rust
impl Evaluator {
    pub fn eval(&self, expr: &Expr) -> Result<LazyValue> {
        match expr {
            Expr::Field(field) => {
                let base = self.eval(&field.base)?;
                base.field(&field.field.name)  // Extends path for lazy!
            }
            Expr::Index(index) => {
                let base = self.eval(&index.base)?;
                let idx = self.eval(&index.index)?.resolve()?;  // Need concrete index
                // ... index access
            }
            Expr::Var(ident) => {
                self.ctx.get(&ident.name)
                    .cloned()
                    .ok_or_else(|| /* undefined */)
            }
            // ... other cases
        }
    }
}
```

## Dependency Graph Example

Given `data/versions.toml`:
```toml
[dodeca]
version = "0.1.0"
name = "Dodeca"

[facet]
version = "0.2.0"
```

And template:
```jinja
{{ data.versions.dodeca.version }}
```

Salsa dependency graph:
```
render_page("/docs/install")
    └── resolve_data_path(["versions", "dodeca", "version"])
            └── load_data_file(versions.toml)
```

If we change `versions.toml` to update `dodeca.name`:
- `load_data_file` result changes
- `resolve_data_path(["versions", "dodeca", "version"])` is re-evaluated
- But the **value** at that path hasn't changed!
- Salsa sees same output → **no downstream invalidation**

If we change `dodeca.version`:
- `resolve_data_path(["versions", "dodeca", "version"])` returns different value
- Pages using this path re-render

## Iteration Granularity

Even iteration is lazy:

```jinja
{% for name, info in data.versions %}
  {{ info.version }}
{% endfor %}
```

Creates dependencies:
- `data_keys_at(["versions"])` → `["dodeca", "facet"]`
- `resolve_data_path(["versions", "dodeca", "version"])`
- `resolve_data_path(["versions", "facet", "version"])`

If we add a new `[other]` section to versions.toml:
- `data_keys_at(["versions"])` changes
- Loop re-runs
- But existing path resolutions may be cached!

## Edge Cases

### Truthiness checks

```jinja
{% if data.config.feature_enabled %}
```

This needs resolution to check truthiness. Creates dependency on full path.

### Filters

```jinja
{{ data.items | length }}
```

The `length` filter needs to know the array/object size. This triggers `len_at()` which may not need full value resolution.

### Comparisons

```jinja
{% if data.version == "1.0" %}
```

Both sides resolve. Comparison creates dependency.

### Nested data access in functions

```jinja
{{ format_version(data.versions.dodeca) }}
```

The function receives a `LazyValue`. If it accesses `.version`, that creates a dependency.

## Benefits

1. **Maximum granularity**: Each unique path is individually tracked
2. **Minimal invalidation**: Only re-render when the exact values used change
3. **Lazy all the way down**: Even nested access doesn't resolve until needed
4. **Composable**: Works with existing filters, functions, etc.

## Trade-offs

1. **More Salsa queries**: Each path = one query (but queries are cheap and cached)
2. **LazyValue wrapper**: Some API changes in gingembre
3. **Resolution overhead**: Extra indirection when accessing values
4. **Interned paths**: Memory for path deduplication (negligible)

## Implementation Plan

### Phase 1: LazyValue in gingembre
1. Add `LazyValue` enum and `DataPath` type
2. Add `DataResolver` trait
3. Update `Context` to use `LazyValue`
4. Update `Evaluator` to work with `LazyValue`
5. Add resolution points (printing, comparison, iteration)
6. Tests

### Phase 2: Salsa integration in dodeca
1. Add `DataPathId` interned type
2. Add `resolve_data_path` tracked query
3. Add `data_keys_at` tracked query
4. Implement `SalsaDataResolver`
5. Update render functions
6. Tests

### Phase 3: Optimize
1. Profile query overhead
2. Consider path caching strategies
3. Batch resolution for known patterns
