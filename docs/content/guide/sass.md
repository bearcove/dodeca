+++
title = "Sass/SCSS"
description = "Using Sass stylesheets in dodeca"
weight = 50
+++

Dodeca compiles Sass/SCSS stylesheets automatically.

## Directory structure

Place your Sass files in a `sass/` directory at your project root:

```
my-site/
├── .config/
│   └── dodeca.styx
├── content/
├── sass/
│   ├── main.scss        # Entry point (required)
│   ├── _variables.scss  # Partial (not compiled directly)
│   └── _components.scss # Another partial
├── static/
└── templates/
```

## Entry point

The file `sass/main.scss` is the entry point. This is the only file that gets compiled directly - all other styles should be imported from here.

If `main.scss` doesn't exist, SCSS compilation is skipped (no error).

## Partials

Files starting with `_` are partials. They're not compiled on their own but can be imported:

```scss
// _variables.scss
$primary-color: #3498db;
$font-stack: system-ui, sans-serif;

// _components.scss
.button {
  background: $primary-color;
  padding: 0.5rem 1rem;
}
```

Import them in `main.scss` using `@use`:

```scss
// main.scss
@use "variables";
@use "components";

body {
  font-family: variables.$font-stack;
  color: variables.$primary-color;
}
```

Note: Use `@use` (modern Sass) rather than `@import` (deprecated).

## Using in templates

Reference the compiled CSS in your templates:

```html
<link rel="stylesheet" href="/main.css">
```

Dodeca automatically:
1. Compiles `sass/main.scss` to CSS
2. Rewrites URLs inside the CSS (for images, fonts, etc.)
3. Adds a content hash for cache busting (e.g., `/main.a1b2c3d4.css`)
4. Serves `/main.css` with a redirect to the hashed version

## Live reload

When you modify any `.scss` file during `ddc serve`:

1. The file change is detected
2. SCSS is recompiled
3. A CSS-specific live reload message is sent
4. The browser updates styles without a full page reload

This makes style iteration quick and avoids a full page reload for pure CSS changes.

## Cache busting

The compiled CSS gets a content-based hash in its filename:

- Source: `sass/main.scss`
- Output: `main.0a3dec24.css` (hash changes when content changes)

This means browsers can cache the CSS forever, but will always fetch new versions when you deploy changes.

## URL rewriting

URLs in your CSS (for backgrounds, fonts, etc.) are automatically rewritten to point to the correct cache-busted paths:

```scss
// Input
.hero {
  background: url('/images/hero.jpg');
}

// Output (after compilation)
.hero {
  background: url('/images/hero.dec0da12.jpg');
}
```

## Troubleshooting

### SCSS not compiling

- Check that `sass/main.scss` exists (exact name required)
- Check the terminal for compilation errors

### Styles not updating

- Hard refresh the browser (Cmd+Shift+R / Ctrl+Shift+R)
- Check that live reload is connected (look for WebSocket connection in browser dev tools)
- Try `ddc clean` to clear caches, then restart `ddc serve`

### Import errors

- Use `@use "filename"` without the `_` prefix or `.scss` extension
- Partials must be in the same `sass/` directory
