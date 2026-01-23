# Handoff: QualName Facet Serialization Issue

## Completed
- Migrated from IndexMap<Stem, Stem> to Vec<(QualName, Stem)> for attributes (hotmeal/src/dom.rs, hotmeal/src/diff.rs)
- Changed Properties::Key from QualName to PropKey enum {Text, Attr(QualName)} (hotmeal/src/diff.rs:98-113)
- Added proxy types LocalNameProxy and QualNameProxy for Facet serialization (hotmeal/src/diff.rs:24-73)
- Added AttrPair struct to wrap (QualName, Stem) tuples (hotmeal/src/diff.rs:75-93)
- Added Hash derive to Namespace enum (hotmeal/src/dom.rs:767)
- Fixed hotmeal-wasm to use PropKey and convert QualName to strings for web-sys (hotmeal-wasm/src/lib.rs)
- All Rust tests passing (27 lib tests, 10 integration tests)
- Committed changes in 3 commits: 0bc6b46, 772e290, 869ac6d

## Active Work

### Origin
User's original request chain:
1. "so let's trade IndexMap<K, V> for Vec<(K, V)> please"
2. "it's not that, it's that we need to take advantage of LocalName which is an interned string - we always convert it to Stem but that's wasteful"
3. "question: why is it okay to squash QualName => LocalName... have we tried diff with inline SVG items for example?"
4. User confirmed we should preserve full QualName for attributes to handle namespaced attrs like `xlink:href`, `xml:lang`
5. "Key type in PropertyInFinalState should be an enum {Text,Attr(QualName)}"
6. "fix hotmeal-wasm next please"
7. "yeah revert that last change and write a unit test for serialization/deserialization via facet-json please. we can run it with tracing to see exactly where it fails"

User wanted to use LocalName (interned strings from html5ever) instead of Stem to avoid wasteful conversions, and preserve QualName for attributes to support SVG/XML namespaces.

### The Problem
WASM tests are failing with: **"serialize failed: unsupported value kind for serialization"**

When `hotmeal::diff_html()` generates patches and tries to serialize them via `facet_json::to_string()`, it fails. This happens in hotmeal-wasm/src/lib.rs:38-39:

```rust
let patches = hotmeal::diff_html(old_html, new_html)
    .map_err(|e| JsValue::from_str(&format!("diff failed: {e}")))?;
let json = facet_json::to_string(&patches)
    .map_err(|e| JsValue::from_str(&format!("serialize failed: {e}")))?;
```

13 WASM tests failing:
- `hotmeal WASM › roundtrip: add_class`
- `hotmeal WASM › roundtrip: change_class`
- `hotmeal WASM › roundtrip: newlines_between_elements`
- `hotmeal fuzzing › fuzz seed 0-9` (10 tests)

All Rust tests pass, only WASM serialization fails.

### Current State
- Branch: `more-arena`
- All Rust compilation clean (`cargo check`, `cargo test --lib`, `cargo test`)
- WASM compilation clean (`cargo check --target wasm32-unknown-unknown`)
- WASM tests fail at runtime during JSON serialization
- Test command: `cd hotmeal-wasm && npm test -- --grep "add_class"`

### Technical Context

**The proxy architecture:**

We have opaque types (QualName, LocalName) from html5ever that don't implement Facet. We use proxy types with `#[facet(opaque, proxy = ...)]`:

```rust
// hotmeal/src/diff.rs:24-41
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[facet(transparent)]
pub struct LocalNameProxy(pub String);

impl TryFrom<LocalNameProxy> for LocalName {
    type Error = std::convert::Infallible;
    fn try_from(proxy: LocalNameProxy) -> Result<Self, Self::Error> {
        Ok(LocalName::from(proxy.0))
    }
}

impl TryFrom<&LocalName> for LocalNameProxy {
    type Error = std::convert::Infallible;
    fn try_from(local: &LocalName) -> Result<Self, Self::Error> {
        Ok(LocalNameProxy(local.to_string()))
    }
}
```

```rust
// hotmeal/src/diff.rs:43-73
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct QualNameProxy {
    pub prefix: Option<String>,
    pub ns: String,
    pub local: String,
}

impl TryFrom<QualNameProxy> for QualName { /* ... */ }
impl TryFrom<&QualName> for QualNameProxy { /* ... */ }
```

**Where QualName/LocalName appear in serializable types:**

1. `Patch::InsertElement { tag: LocalName, attrs: Vec<AttrPair>, ... }` (diff.rs:197-204)
2. `Patch::SetAttribute { name: QualName, ... }` (diff.rs:228-232)
3. `Patch::RemoveAttribute { name: QualName, ... }` (diff.rs:235-239)
4. `PropChange { name: PropKey, ... }` where `PropKey::Attr(QualName)` (diff.rs:177-187, 98-113)
5. `InsertContent::Element { tag: LocalName, attrs: Vec<AttrPair>, ... }` (diff.rs:163-170)
6. `AttrPair { name: QualName, value: Stem }` (diff.rs:75-81)

All use `#[facet(opaque, proxy = ...)]` annotations.

**The mystery:**

I attempted to change LocalNameProxy from transparent tuple struct to regular struct:
```rust
// BAD attempt (reverted):
pub struct LocalNameProxy { pub value: String }
```

But the user said "revert that last change" because `#[facet(transparent)]` is specifically designed for single-field wrappers and should be correct.

**What we don't know:**
- Why does facet-json fail with "unsupported value kind for serialization"?
- Is the error from LocalName, QualName, or something else?
- Does the proxy mechanism work at all, or is there a configuration issue?

### Success Criteria
1. Write unit test in `hotmeal/src/diff.rs` that serializes and deserializes patches containing QualName/LocalName
2. Run test with `RUST_LOG=trace cargo test <test_name> -- --nocapture` to see exactly where facet-json fails
3. Fix the serialization issue (likely one of):
   - Missing derive macro
   - Incorrect proxy implementation
   - Facet version issue
   - Need different proxy structure
4. All WASM tests pass: `cd hotmeal-wasm && npm test`
5. Commit the fix

### Files to Touch
- `hotmeal/src/diff.rs:1545` - add test after the last test (test_arena_dom_diff_add_element)
  - Name it `test_patch_serialization`
  - Test a simple case like add_class: `<div>Content</div>` → `<div class="highlight">Content</div>`
  - Use `diff_html()` to generate patches
  - Serialize with `facet_json::to_string(&patches).expect("should serialize")`
  - Deserialize back with `facet_json::from_str::<Vec<Patch>>(&json).expect("should deserialize")`
  - Assert patches match
  - Run with tracing to see errors

### Decisions Made
- Using `#[facet(transparent)]` for LocalNameProxy is correct (user confirmed)
- QualNameProxy uses regular struct with public fields (not transparent) because it has 3 fields
- Attributes use empty namespace `ns!()` for HTML attrs, not `ns!(html)` (HTML namespace is for element tags)
- PropKey enum eliminates the "_text" hack for text content vs attributes

### What NOT to Do
- Don't change the transparent attribute on LocalNameProxy (user said revert that)
- Don't try to optimize string conversions in proxies yet (there's a TODO but it's deferred)
- Don't refactor unrelated code

### Blockers/Gotchas
- facet-json is only available in hotmeal-wasm, not hotmeal (so the test needs to be careful about imports)
- WASM tests run in browser via Playwright, can't easily add debug output
- The error message "unsupported value kind for serialization" is vague - tracing will be critical
- HTML attributes have empty namespace `ns!()`, not `ns!(html)` - this tripped up tests earlier

### Test Template
```rust
#[test]
fn test_patch_serialization() {
    use facet_json;

    let old_html = r#"<html><body><div>Content</div></body></html>"#;
    let new_html = r#"<html><body><div class="highlight">Content</div></body></html>"#;

    let patches = diff_html(old_html, new_html).expect("diff should work");
    debug!("Patches: {:#?}", patches);

    // This should fail with "unsupported value kind for serialization"
    let json = facet_json::to_string(&patches).expect("serialization should work");
    debug!("JSON: {}", json);

    let roundtrip: Vec<Patch> = facet_json::from_str(&json).expect("deserialization should work");
    assert_eq!(patches, roundtrip);
}
```

Run with:
```bash
cd hotmeal
RUST_LOG=trace cargo test test_patch_serialization -- --nocapture 2>&1 | tee /tmp/serialize.log
```

Look for facet-json internals showing which field/type fails.

## Bootstrap
```bash
cd /Users/amos/bearcove/hotmeal/hotmeal
git status  # should show changes committed
cargo test --lib  # should pass (27 tests)
cd ../hotmeal-wasm
npm test -- --grep "add_class"  # should fail with serialization error
```
