+++
title = "Terminal Recording"
description = "Capture terminal output with ANSI colors for documentation"
weight = 50
+++

`ddc term` records terminal sessions and converts ANSI-colored output to HTML, ready for embedding in your documentation.

## Usage

Record a command and paste directly into your markdown:

```bash
ddc term -- cargo build
```

The output is automatically copied to your clipboard wrapped in a `term` code fence. Paste it into any markdown file and it renders with full color support.

### Interactive Mode

Start an interactive session and manually run commands:

```bash
ddc term
```

Exit the shell (Ctrl-D) when done. Everything displayed in the terminal (except
alt mode) is captured.

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
