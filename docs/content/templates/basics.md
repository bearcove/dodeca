+++
title = "Basics"
weight = 10
+++

dodeca uses gingembre, a Jinja-like template engine. If you've used Jinja2, Tera, or Nunjucks, the syntax will be familiar.

## Template files

Templates live in the `templates/` directory. dodeca looks for these templates:

| Template | Used for |
|----------|----------|
| `index.html` | Root section (`content/_index.md`) |
| `section.html` | All other sections |
| `page.html` | Individual pages |
| `base.html` | Common layout (inherited by others) |

## Inheritance

A base template defines blocks that child templates can override:

```html
{# base.html #}
<!DOCTYPE html>
<html>
<head>
    <title>{% block title %}My Site{% endblock title %}</title>
</head>
<body>
    {% block body %}{% endblock body %}
</body>
</html>
```

```html
{# page.html #}
{% extends "base.html" %}

{% block title %}{{ page.title }} - My Site{% endblock title %}

{% block body %}
<article>
    <h1>{{ page.title }}</h1>
    {{ page.content | safe }}
</article>
{% endblock body %}
```

## Variables

Output a variable with `{{ }}`:

```html
{{ page.title }}
{{ section.pages | length }}
```

Use the `safe` filter to output raw HTML (like rendered markdown):

```html
{{ page.content | safe }}
```

## Control flow

### If / else

```html
{% if page.extra.show_toc %}
    <nav>{{ page.toc | safe }}</nav>
{% endif %}

{% if page.weight > 10 %}
    ...
{% elif page.weight > 5 %}
    ...
{% else %}
    ...
{% endif %}
```

### For loops

```html
{% for page in section.pages %}
    <a href="{{ page.permalink }}">{{ page.title }}</a>
{% endfor %}
```

Loop variables:

| Variable | Description |
|----------|-------------|
| `loop.index` | Current iteration (1-based) |
| `loop.index0` | Current iteration (0-based) |
| `loop.first` | `true` on first iteration |
| `loop.last` | `true` on last iteration |
| `loop.length` | Total items |

### Set

```html
{% set name = "dodeca" %}
{% set full_title = page.title ~ " - " ~ name %}
```

## Include

Include another template file:

```html
{% include "partials/header.html" %}
```

## Macros

Define reusable template snippets:

```html
{% macro card(title, url) %}
<div class="card">
    <h3><a href="{{ url }}">{{ title }}</a></h3>
</div>
{% endmacro card %}

{{ card(title="Hello", url="/hello/") }}
```

## Comments

```html
{# This is a comment and won't appear in output #}
```
