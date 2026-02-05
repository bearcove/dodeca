+++
title = "Quick Start"
description = "Get up and running with dodeca"
weight = 20
+++

## Create a project

The fastest way to start is with `ddc init`:

```bash
ddc init my-site
cd my-site
```

This creates a new project with the basic structure. You can also specify a template:

```bash
ddc init my-site --template blog
```

### Manual setup

Alternatively, create the structure manually:

```bash
mkdir my-site
cd my-site
```

Create `.config/dodeca.styx`:

```styx
content: content
output: public
```

Create `content/_index.md`:

```markdown
+++
title = "My Site"
+++

Hello, world!
```

## Create a template (root section)

Create `templates/index.html`:

```html
<!DOCTYPE html>
<html>
<head>
    <title>{{ section.title }}</title>
</head>
<body>
    {{ section.content | safe }}
</body>
</html>
```

At minimum, the root section (`/`) uses `templates/index.html`. If you add more content later, you'll typically also want:

- `templates/section.html` (non-root sections)
- `templates/page.html` (individual pages)

## Build

```bash
ddc build
```

Your site is now in `public/`.

## Serve with live reload

```bash
ddc serve
```

`ddc serve` will print the URL it's listening on. By default it tries ports `4000–4019` and falls back to an OS-assigned port if those are taken.

## Code samples

If the code execution helper is available (`ddc-cell-code-execution`), Rust code blocks marked with `,test` are compiled and executed:

- In `ddc build`: failing code stops the build.
- In `ddc serve`: failing code is reported as a warning.

Example:

````markdown
```rust,test
let message = "Hello from dodeca!";
println!("{}", message);
```
````

Plain `rust` blocks (without `,test`) are syntax-highlighted but not executed.
