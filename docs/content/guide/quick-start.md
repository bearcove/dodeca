+++
title = "Quick Start"
description = "Get up and running with dodeca"
weight = 20
+++

## Create a project

Create a new directory for your site:

```bash
mkdir my-site
cd my-site
```

## Add configuration

Create `.config/dodeca.kdl`:

```kdl
content "content"
output "public"
```

## Create content

Create `content/_index.md`:

```markdown
+++
title = "My Site"
+++

Hello, world!
```

## Create a template

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

## Build

```bash
ddc build
```

Your site is now in `public/`.

## Serve with live reload

```bash
ddc serve
```

Open http://localhost:4000 and start editing. Changes appear instantly.
