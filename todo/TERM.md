# Terminal Recording Feature for Dodeca

## Overview

Add `ddc term` command to record terminal sessions with ANSI colors, producing output that can be pasted directly into dodeca markdown documents. The output uses custom HTML tags with a modern naming convention.

## Current State (libterm in cove)

The existing `libterm` crate in cove provides:

- PTY-based terminal recording using `portable_pty`
- ANSI escape code parsing via `anstyle_parse`
- Conversion to HTML with `<i class="b fg-cyn">` style classes
- CSS generation for the 256-color ANSI palette with `light-dark()` support
- Clipboard integration (pbcopy, xclip, clip.exe)

**Current output format:**
```term
<i class="b fg-cyn">text</i>
<i class="u fg-grn">underlined green</i>
```

Classes: `b` (bold), `u` (underline), `i` (italic), `st` (strikethrough), `l` (light/faint)
Colors: `fg-blk`, `fg-red`, `fg-grn`, `fg-ylw`, `fg-blu`, `fg-mag`, `fg-cyn`, `fg-wht`
256-color: `fg-ansi0` through `fg-ansi255`
24-bit RGB: inline `style="color:#rrggbb"`

## Proposed New Format

Custom tags with `t-` prefix for terminal styling:

```term
<t-b><t-fcyn>bold cyan text</t-fcyn></t-b>
<t-u><t-fgrn>underlined green</t-fgrn></t-u>
```

### Tag Naming Convention

**Attributes:**
- `<t-b>` - bold
- `<t-l>` - light/faint
- `<t-i>` - italic
- `<t-u>` - underline
- `<t-st>` - strikethrough

**Foreground colors (16 classic):**
- `<t-fblk>` - black
- `<t-fred>` - red
- `<t-fgrn>` - green
- `<t-fylw>` - yellow
- `<t-fblu>` - blue
- `<t-fmag>` - magenta
- `<t-fcyn>` - cyan
- `<t-fwht>` - white
- Light variants: `<t-flblk>`, `<t-flred>`, etc.

**Background colors (16 classic):**
- `<t-bblk>` - black background
- `<t-bred>` - red background
- etc.

**256-color palette:**
- `<t-f42>` - foreground ANSI color 42
- `<t-b42>` - background ANSI color 42

**24-bit RGB:**
- Use inline styles: `<span style="color:#ff5500">` or `<t-f style="--c:#ff5500">`
- Alternative: CSS custom properties with fallback

### Benefits of Custom Tags

1. **Semantic clarity** - `<t-fcyn>` clearly means "terminal foreground cyan"
2. **No class collisions** - Won't conflict with existing CSS
3. **Easier parsing** - marq can handle these tags specially
4. **Compact** - `<t-b>` vs `<i class="b">`
5. **Nestable** - Multiple attributes compose naturally

## Implementation Plan

### Phase 1: Cell Architecture

The term functionality lives in a **cell** to avoid pulling heavy dependencies (`portable-pty`, `anstyle-parse`) into the main `ddc` binary.

**Structure:**
```
cells/
  cell-term-proto/       # Protocol definition (minimal deps)
    Cargo.toml
    src/lib.rs           # TermRecorder trait, TermResult enum, data types
  cell-term/             # Implementation (heavy deps here)
    Cargo.toml
    src/
      main.rs            # Cell entry point
      recorder.rs        # PTY recording logic
      parser.rs          # ANSI escape code parsing  
      renderer.rs        # Convert to HTML with <t-*> tags
```

**Proto crate dependencies:**
- `facet` - Serialization
- `roam` - RPC service macro

**Cell crate dependencies:**
- `portable-pty` - Cross-platform PTY
- `anstyle-parse` - ANSI escape sequence parser
- `dodeca-cell-runtime` - Cell runtime macros

**Core types:**
```rust
pub struct TermRecording {
    lines: Vec<Line>,
}

pub struct Line {
    spans: Vec<Span>,
}

pub struct Span {
    text: String,
    style: Style,
}

pub struct Style {
    bold: bool,
    faint: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    fg: Option<Color>,
    bg: Option<Color>,
}

pub enum Color {
    Classic(ClassicColor),  // 16 colors
    Ansi256(u8),           // 256 palette
    Rgb(u8, u8, u8),       // 24-bit
}

pub enum ClassicColor {
    Black, Red, Green, Yellow, Blue, Magenta, Cyan, White,
    LightBlack, LightRed, LightGreen, LightYellow,
    LightBlue, LightMagenta, LightCyan, LightWhite,
}
```

**Proto service trait:**
```rust
#[roam::service]
pub trait TermRecorder {
    /// Record a terminal session interactively
    async fn record_interactive(&self, config: RecordConfig) -> TermResult;
    
    /// Record with auto-executed commands
    async fn record_commands(&self, commands: Vec<String>, config: RecordConfig) -> TermResult;
}

#[derive(Facet)]
#[repr(u8)]
pub enum TermResult {
    Success { html: String },
    Error { message: String },
}
```

### Phase 2: CLI Command (`ddc term`)

Location: `crates/dodeca/src/term.rs`

**Arguments:**
```rust
#[derive(Facet, Debug)]
struct TermArgs {
    /// Command to auto-execute (everything after --)
    #[facet(args::positional, default)]
    command: Option<String>,

    /// Shell to use (default: $SHELL or /bin/sh)
    #[facet(args::named, default)]
    shell: Option<String>,
}
// Default behavior: write to /tmp/ddc-term AND copy to clipboard
```

**Usage:**
```bash
# Interactive recording
ddc term

# Auto-execute a command
ddc term -- cargo build
```

Output is written to `/tmp/ddc-term` and copied to clipboard.

### Phase 3: marq Integration

Location: `../marq/` (separate repo)

marq has a `CodeBlockHandler` trait that receives `(language, code)` and returns HTML.
The `code` parameter is the **raw content** of the fenced block - not escaped.

For `term` blocks, the content already contains `<t-*>` HTML tags from `ddc term` output.
The handler just needs to wrap it:

```rust
pub struct TermHandler;

impl CodeBlockHandler for TermHandler {
    fn render<'a>(
        &'a self,
        _language: &'a str,
        code: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            // code already contains <t-b><t-fcyn>...</t-fcyn></t-b> etc.
            // Just wrap it in a pre block with the term class
            Ok(format!("<pre class=\"term\">{code}</pre>"))
        })
    }
}
```

**Registration in dodeca:**
```rust
let options = RenderOptions::new()
    .with_handler(&["term"], TermHandler);
```

That's it. The `<t-*>` tags pass through as-is and get styled by CSS.

### Phase 4: CSS and Theming

Location: `crates/dodeca-term/src/css.rs`

Generate CSS that:
1. Defines all `t-*` custom elements as `display: inline`
2. Uses `light-dark()` for color adaptation
3. Uses CSS custom properties for flexibility
4. Includes the 256-color palette

**Example CSS:**
```css
/* Attributes */
t-b { font-weight: bold; }
t-l { opacity: 0.7; }
t-i { font-style: italic; }
t-u { text-decoration: underline; }
t-st { text-decoration: line-through; }

/* Foreground colors (classic 16) */
t-fblk { color: light-dark(#292929, #d4d4d4); }
t-fred { color: light-dark(#c41a16, #ff6b6b); }
t-fgrn { color: light-dark(#007400, #5af78e); }
t-fylw { color: light-dark(#826b00, #f3f99d); }
t-fblu { color: light-dark(#0000d6, #57c7ff); }
t-fmag { color: light-dark(#a90d91, #ff6ac1); }
t-fcyn { color: light-dark(#007482, #9aedfe); }
t-fwht { color: light-dark(#6e6e6e, #f1f1f0); }
/* Light variants: t-flblk, t-flred, etc. */

/* 256-color palette */
t-f0 { color: #000000; }
t-f1 { color: light-dark(#800000, #ff5555); }
/* ... generated for all 256 ... */
```

### Phase 5: Integration with dodeca Build

Ensure the term CSS is:
1. Generated as part of the build
2. Included in the main stylesheet

Embed in the main CSS bundle.

**24-bit color syntax**: `<t-f style="--c:#ff5500">` with CSS `t-f { color: var(--c); }`

## Task Breakdown

1. [ ] Create `cells/cell-term-proto/` with service trait and data types
2. [ ] Create `cells/cell-term/` with cell implementation
3. [ ] Port ANSI parsing from libterm
4. [ ] Implement HTML renderer with `<t-*>` tags
5. [ ] Generate CSS for all colors (16 classic + 256 palette)
6. [ ] Add PTY recording functionality
7. [ ] Add clipboard support
8. [ ] Register cell in `crates/dodeca/src/cells.rs`
9. [ ] Implement `ddc term` CLI command
10. [ ] Add `TermHandler` to marq (just wraps in `<pre class="term">`)
11. [ ] Add term CSS to dodeca's default stylesheet
12. [ ] Test on macOS, Linux, Windows

## References

- Existing libterm: `/Users/amos/bearcove/cove/crates/libterm/`
- Existing ansi.css: `/Users/amos/bearcove/cove/wat/scss/ansi.css`
- marq codebase: `/Users/amos/bearcove/marq/`
- ANSI escape codes: https://en.wikipedia.org/wiki/ANSI_escape_code
