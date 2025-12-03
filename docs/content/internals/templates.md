+++
title = "Template Engine"
weight = 20
+++

dodeca includes a Jinja-like template engine built for tight integration with Salsa's incremental computation.

## Template Files

dodeca uses three template files in your `templates/` directory:

- `index.html` - renders the root section (`/`)
- `section.html` - renders non-root sections
- `page.html` - renders individual pages

Templates can extend other templates using `{% extends "base.html" %}` and define blocks with `{% block name %}...{% endblock %}`.

## Global Context

These variables are available in all templates:

### `config`

Site configuration:

```jinja
{{ config.title }}       {# "dodeca" #}
{{ config.description }} {# Site description #}
{{ config.base_url }}    {# "/" #}
```

### `current_path`

The URL path of the current page, useful for navigation highlighting:

```jinja
<a href="/guide/" {% if current_path is starting_with("/guide") %}class="active"{% endif %}>
  Guide
</a>
```

### `root`

The root section, useful for building sidebars:

```jinja
{% for sub in root.subsections %}
  <h3><a href="{{ sub.permalink }}">{{ sub.title }}</a></h3>
  <ul>
    {% for page in sub.pages %}
      <li><a href="{{ page.permalink }}">{{ page.title }}</a></li>
    {% endfor %}
  </ul>
{% endfor %}
```

### `data`

Data files loaded from `data/` directory (JSON, YAML, TOML, KDL):

```jinja
{% for item in data.navigation %}
  <a href="{{ item.url }}">{{ item.label }}</a>
{% endfor %}
```

## Page Context

Available in `page.html`:

### `page`

| Field | Type | Description |
|-------|------|-------------|
| `title` | string | Page title from frontmatter |
| `content` | string | Rendered HTML content (use `\| safe`) |
| `permalink` | string | URL path like `/guide/getting-started/` |
| `path` | string | Source file path like `guide/getting-started.md` |
| `weight` | int | Sort order from frontmatter |
| `toc` | list | Table of contents (see below) |
| `ancestors` | list | Parent sections from root to immediate parent |
| `last_updated` | int | Unix timestamp of file modification |

### `section`

The parent section of the current page (same structure as section context below, but without recursive subsection content).

## Section Context

Available in `section.html` and `index.html`:

### `section`

| Field | Type | Description |
|-------|------|-------------|
| `title` | string | Section title from `_index.md` frontmatter |
| `content` | string | Rendered HTML content (use `\| safe`) |
| `permalink` | string | URL path like `/guide/` |
| `path` | string | Source file path like `guide/_index.md` |
| `weight` | int | Sort order from frontmatter |
| `toc` | list | Table of contents |
| `last_updated` | int | Unix timestamp of file modification |
| `pages` | list | Pages in this section (sorted by weight) |
| `subsections` | list | Child sections (sorted by weight) |

Each item in `pages`:

| Field | Type | Description |
|-------|------|-------------|
| `title` | string | Page title |
| `permalink` | string | URL path |
| `path` | string | Source file path |
| `weight` | int | Sort order |
| `toc` | list | Table of contents |

Each item in `subsections`:

| Field | Type | Description |
|-------|------|-------------|
| `title` | string | Section title |
| `permalink` | string | URL path |
| `weight` | int | Sort order |
| `pages` | list | Pages in subsection |

## Table of Contents

The `toc` field is a hierarchical list of headings:

```jinja
{% for h in page.toc %}
  <li>
    <a href="{{ h.permalink }}">{{ h.title }}</a>
    {% if h.children %}
      <ul>
        {% for child in h.children %}
          <li><a href="{{ child.permalink }}">{{ child.title }}</a></li>
        {% endfor %}
      </ul>
    {% endif %}
  </li>
{% endfor %}
```

Each heading has:

| Field | Type | Description |
|-------|------|-------------|
| `title` | string | Heading text |
| `id` | string | Anchor ID |
| `level` | int | Heading level (1-6) |
| `permalink` | string | Anchor link like `#introduction` |
| `children` | list | Nested subheadings |

## Ancestors (Breadcrumbs)

The `ancestors` field is an ordered list of parent sections from root to immediate parent:

```jinja
<nav class="breadcrumbs">
  <a href="/">Home</a>
  {% for ancestor in page.ancestors %}
    / <a href="{{ ancestor.permalink }}">{{ ancestor.title }}</a>
  {% endfor %}
  / {{ page.title }}
</nav>
```

Each ancestor has: `title`, `permalink`, `path`, `weight`.

## Functions

### `get_section(path=...)`

Retrieve a section by its source path:

```jinja
{% set guide = get_section(path="guide/_index.md") %}
<h2>{{ guide.title }}</h2>
{% for page in guide.pages %}
  <a href="{{ page.permalink }}">{{ page.title }}</a>
{% endfor %}
```

Returns a dict with: `title`, `permalink`, `path`, `content`, `toc`, `pages`, `subsections`.

Note: `subsections` from `get_section` returns a list of path strings (like `"guide/advanced/_index.md"`), which you can pass to another `get_section` call:

```jinja
{% for sub_path in section.subsections %}
  {% set sub = get_section(path=sub_path) %}
  <h3>{{ sub.title }}</h3>
{% endfor %}
```

### `get_url(path=...)`

Convert a path to a URL:

```jinja
<a href="{{ get_url(path='guide/getting-started') }}">Get Started</a>
```

## Filters

| Filter | Description |
|--------|-------------|
| `safe` | Output without HTML escaping |
| `escape` | HTML escape (default behavior) |
| `upper` | Convert to uppercase |
| `lower` | Convert to lowercase |
| `capitalize` | Capitalize first character |
| `title` | Title Case Each Word |
| `trim` | Remove leading/trailing whitespace |
| `length` | Get length of string, list, or dict |
| `first` | Get first element/character |
| `last` | Get last element/character |
| `reverse` | Reverse string or list |
| `sort` | Sort list (use `sort(attribute="field")` for dicts) |
| `join(sep)` | Join list with separator |
| `default(value)` | Fallback if value is empty/none |

All output is HTML-escaped by default. Use `| safe` for pre-rendered HTML:

```jinja
{{ page.content | safe }}
{{ page.title | default("Untitled") }}
{{ tags | sort | join(", ") }}
{{ section.pages | sort(attribute="weight") }}
```

## Tests

Use tests in conditionals with `is`:

**String tests:**

| Test | Description |
|------|-------------|
| `starting_with(prefix)` | String starts with prefix |
| `ending_with(suffix)` | String ends with suffix |
| `containing(substring)` | String contains substring (also works on lists) |

**Type tests:**

| Test | Description |
|------|-------------|
| `defined` | Value is not none |
| `undefined` | Value is none |
| `none` | Value is none |
| `string` | Value is a string |
| `number` | Value is int or float |
| `integer` | Value is an int |
| `float` | Value is a float |
| `mapping` / `dict` | Value is a dict |
| `iterable` / `sequence` | Value is list, string, or dict |
| `empty` | String, list, or dict is empty |

**Value tests:**

| Test | Description |
|------|-------------|
| `odd` | Integer is odd |
| `even` | Integer is even |
| `truthy` | Value is truthy |
| `falsy` | Value is falsy |

**Comparison tests:**

| Test | Description |
|------|-------------|
| `eq(value)` / `equalto` / `sameas` | Values are equal |
| `ne(value)` | Values are not equal |
| `lt(value)` / `lessthan` | Less than |
| `gt(value)` / `greaterthan` | Greater than |

```jinja
{% if current_path is starting_with("/guide") %}
  {# In guide section #}
{% endif %}

{% if page.path is containing("advanced") %}
  {# Advanced page #}
{% endif %}

{% if page.toc is empty %}
  {# No headings #}
{% endif %}

{% if loop.index is odd %}
  {# Odd row #}
{% endif %}
```

## Control Flow

```jinja
{% if condition %}
  ...
{% elif other %}
  ...
{% else %}
  ...
{% endif %}

{% for item in list %}
  {{ item }}
{% endfor %}

{% set variable = value %}
```

### Loop Variables

Inside `{% for %}` loops, a `loop` object is available:

| Variable | Description |
|----------|-------------|
| `loop.index` | Current iteration (1-indexed) |
| `loop.index0` | Current iteration (0-indexed) |
| `loop.first` | True if first iteration |
| `loop.last` | True if last iteration |
| `loop.length` | Total number of items |

```jinja
{% for item in items %}
  <li class="{% if loop.first %}first{% endif %}{% if loop.last %}last{% endif %}">
    {{ loop.index }}. {{ item }}
  </li>
{% endfor %}
```

## Macros

Define reusable template fragments:

```jinja
{% macro button(label, href, class="btn") %}
  <a href="{{ href }}" class="{{ class }}">{{ label }}</a>
{% endmacro %}

{{ self::button(label="Click me", href="/action") }}
```

Import macros from other files:

```jinja
{% import "macros.html" as macros %}

{{ macros::button(label="Submit", href="/submit") }}
```

## Complete Example

```jinja
{% extends "base.html" %}

{% block title %}{{ page.title }} - {{ config.title }}{% endblock %}

{% block content %}
<article>
  {# Breadcrumbs #}
  <nav class="breadcrumbs">
    <a href="/">Home</a>
    {% for ancestor in page.ancestors %}
      / <a href="{{ ancestor.permalink }}">{{ ancestor.title }}</a>
    {% endfor %}
    / {{ page.title }}
  </nav>

  <h1>{{ page.title }}</h1>
  {{ page.content | safe }}

  {# Table of contents #}
  {% if page.toc and page.toc | length > 0 %}
  <aside class="toc">
    <h2>On this page</h2>
    <ul>
      {% for h in page.toc %}
        <li><a href="{{ h.permalink }}">{{ h.title }}</a></li>
      {% endfor %}
    </ul>
  </aside>
  {% endif %}
</article>
{% endblock %}
```
