+++
title = "Terminal Recording"
description = "Capture terminal output with ANSI colors for documentation"
weight = 50
+++

`ddc term` records terminal sessions and converts ANSI-colored output to HTML, ready for embedding in your documentation.

## Quick Start

Record a command and paste directly into your markdown:

```bash
ddc term -- cargo build
```

The output is automatically copied to your clipboard wrapped in a `term` code fence. Paste it into any markdown file and it renders with full color support.

## How It Works

1. **Record**: `ddc term` runs your command in a pseudo-terminal, capturing all output including ANSI escape codes
2. **Convert**: The raw output is converted to semantic HTML using custom elements (`<t-b>` for bold, `<t-fred>` for red foreground, etc.)
3. **Copy**: The HTML is wrapped in a ` ```term ` fence and copied to your clipboard
4. **Render**: When dodeca builds your site, `term` code blocks pass through as raw HTML with automatic CSS injection

## Usage

### Record a Command

```bash
# Record a single command
ddc term -- ls -la --color=always

# Record a pipeline
ddc term -- cargo test 2>&1 | head -20

# Record with arguments containing spaces
ddc term -- echo "Hello, World!"
```

### Interactive Mode

Start an interactive session and manually run commands:

```bash
ddc term
```

Type `exit` when done. Everything displayed in the terminal is captured.

## Embedding in Markdown

After running `ddc term`, paste the clipboard contents into your markdown:

````markdown
```term
<t-b><t-fgrn>✓</t-fgrn></t-b> Build successful
<t-fcyn>→</t-fcyn> Running tests...
```
````

The `term` fence tells dodeca to render the content as pre-formatted HTML rather than escaping it.

## Supported ANSI Features

The terminal recorder supports:

- **Basic colors**: Black, red, green, yellow, blue, magenta, cyan, white (foreground and background)
- **Bright colors**: Bright variants of all basic colors
- **256 colors**: Extended palette colors
- **24-bit colors**: True color RGB values
- **Attributes**: Bold, dim, italic, underline, strikethrough, inverse

## Tips

**Keep recordings focused**: Record specific commands rather than long sessions. Shorter recordings are easier to maintain.

**Test your colors**: Run `ddc term -- ls -la --color=always` to verify colors are being captured correctly.

**Re-record when needed**: If command output changes, re-run `ddc term` and paste the new output. The old content is simply replaced.

**Use for error messages**: Recording compiler errors or test failures with colors makes them much more readable in documentation.
