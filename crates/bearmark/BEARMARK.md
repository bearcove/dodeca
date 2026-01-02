# bearmark - Markdown Rendering Library

## Overview

`bearmark` is a shared markdown rendering library in the dodeca workspace, consumed by:
- **cell-markdown** (dodeca) - for static site generation
- **tracey** - for spec rendering in the dashboard

## Design Goals

1. **Pluggable code block handlers** - DI pattern for syntax highlighting, diagrams
2. **Frontmatter support** - TOML (`+++`) and YAML (`---`)
3. **Heading extraction** - with slug generation for TOC
4. **Rule definitions** - `r[rule.id]` syntax for spec traceability
5. **No heavy dependencies** - consumers bring their own handlers

## Core API

```rust
// Main render function
pub async fn render(markdown: &str, options: &RenderOptions) -> Result<Document>;

// Render options with pluggable handlers
pub struct RenderOptions {
    pub source_path: Option<String>,
    pub code_handlers: HashMap<String, BoxedHandler>,
    pub default_handler: Option<BoxedHandler>,
    pub rule_handler: Option<BoxedRuleHandler>,  // NEW: custom rule rendering
}

// Result document
pub struct Document {
    pub frontmatter: Option<Frontmatter>,
    pub html: String,
    pub headings: Vec<Heading>,
    pub rules: Vec<RuleDefinition>,
}

// Code block handler trait
pub trait CodeBlockHandler: Send + Sync {
    fn render(&self, language: &str, code: &str)
        -> Pin<Box<dyn Future<Output = Result<String>> + Send>>;
}

// NEW: Rule handler trait for custom rule rendering
pub trait RuleHandler: Send + Sync {
    fn render(&self, rule: &RuleDefinition)
        -> Pin<Box<dyn Future<Output = Result<String>> + Send>>;
}
```

## Rule Rendering Strategy

### Problem
- bearmark extracts `r[rule.id]` markers and generates basic HTML anchors
- tracey needs custom rendering with coverage info (covered/uncovered status, impl/verify refs)
- dodeca just needs simple anchor divs

### Solution: RuleHandler Trait

Add a `RuleHandler` trait similar to `CodeBlockHandler`:

```rust
pub trait RuleHandler: Send + Sync {
    fn render(&self, rule: &RuleDefinition, following_text: Option<&str>)
        -> Pin<Box<dyn Future<Output = Result<String>> + Send>>;
}
```

**Default behavior** (no handler): Generate simple anchor div
```html
<div class="rule" id="r-rule.id">
  <a class="rule-link" href="#r-rule.id">[rule.id]</a>
</div>
```

**Tracey handler**: Inject coverage status, refs
```rust
struct TraceyRuleHandler {
    coverage: Arc<RuleCoverage>,
}

impl RuleHandler for TraceyRuleHandler {
    fn render(&self, rule: &RuleDefinition, text: Option<&str>) -> ... {
        let status = self.coverage.get(&rule.id);
        // Render with covered/uncovered class, impl/verify links, etc.
    }
}
```

## Code Block Handlers

### Registered by consumers:

**dodeca/tracey common:**
- `aa` → `AasvgHandler` (ASCII art → SVG via aasvg crate)
- `pik` → `PikruHandler` (Pikchr → SVG via pikru crate)
- default → `ArboriumHandler` (syntax highlighting)

**Example setup:**
```rust
let opts = RenderOptions::new()
    .with_handler("aa", AasvgHandler)
    .with_handler("pik", PikruHandler::new(true)) // css_variables=true
    .with_default_handler(ArboriumHandler::new());

let doc = bearmark::render(markdown, &opts).await?;
```

## Dependencies

```toml
[dependencies]
pulldown-cmark = "0.13"
facet = { git = "..." }
facet-toml = { git = "..." }
facet-yaml = { git = "..." }
facet-value = { git = "..." }
thiserror = "2.0"
futures = "0.3"
```

**Not included** (brought by consumers):
- arborium (syntax highlighting)
- aasvg (ASCII diagrams)
- pikru (Pikchr diagrams)

## Implementation Status

### Done
- [x] Crate structure created
- [x] `CodeBlockHandler` trait (async, DI pattern)
- [x] `RuleHandler` trait for custom rule rendering (async)
- [x] Frontmatter parsing via pulldown-cmark (TOML/YAML)
- [x] Raw metadata exposed in Document for custom parsing
- [x] Heading extraction with slugify
- [x] Rule extraction (async, with custom handler support)
- [x] Link resolution (`@/path` and relative `.md` links)
- [x] Main render function
- [x] Handler implementations with feature flags:
  - [x] `ArboriumHandler` - syntax highlighting (`highlight` feature)
  - [x] `AasvgHandler` - ASCII art to SVG (`aasvg` feature)
  - [x] `PikruHandler` - Pikchr diagrams (`pikru` feature)
- [x] 39 tests passing

### TODO
- [ ] Update cell-markdown to use bearmark
- [ ] Update tracey to use bearmark

## File Structure

```
dodeca/crates/bearmark/
├── Cargo.toml
├── BEARMARK.md (this file)
└── src/
    ├── lib.rs          # Re-exports, Error type
    ├── frontmatter.rs  # Frontmatter parsing
    ├── headings.rs     # Heading extraction, slugify
    ├── rules.rs        # Rule definition extraction
    ├── handler.rs      # CodeBlockHandler trait
    └── render.rs       # Main render function

# Handler implementations (separate crates or in consumers)
# These wrap the underlying libraries for the handler trait

tracey/crates/tracey/src/
└── bearmark_handlers.rs  # TraceyRuleHandler, ArboriumHandler, etc.

dodeca/cells/cell-markdown/src/
└── ... uses bearmark internally
```

## Migration Plan

### Phase 1: bearmark core
1. Finish bearmark crate with basic functionality
2. Add RuleHandler trait
3. Write tests

### Phase 2: Handler crates (optional)
Could create shared handler crates:
- `bearmark-arborium` - ArboriumHandler
- `bearmark-aasvg` - AasvgHandler
- `bearmark-pikru` - PikruHandler

Or keep handlers in each consumer.

### Phase 3: cell-markdown migration
1. Replace pulldown-cmark usage with bearmark
2. Keep picante integration in host
3. Remove code block placeholder logic (bearmark handles it)

### Phase 4: tracey migration
1. Add bearmark dependency
2. Create TraceyRuleHandler with coverage injection
3. Move from client-side markdown to server-side
4. Remove marked.js from frontend

## Open Questions

1. **Async vs Sync**: Current design is async. Consider sync option?
   - Arborium is sync, but could be wrapped
   - Allows async handlers for network-based processing

2. **Link resolution**: How much of `@/path` handling goes in bearmark vs consumer?
   - bearmark could normalize paths
   - Consumer handles route resolution

3. **Heading IDs with parent context**: Issue #36 wants `websocket-transport.frame-encoding`
   - Add option for hierarchical IDs?
   - Or handle in consumer?
