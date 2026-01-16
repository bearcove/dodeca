+++
title = "Debugging Templates"
description = "How to debug template issues in dodeca"
weight = 60
+++

When templates don't render as expected, here's how to diagnose and fix the problem.

## Inspecting variables

Print a variable's type and value:

```jinja
DEBUG: {{ some_var }} (type: {{ some_var | typeof }})
```

The `typeof` filter returns: `"string"`, `"number"`, `"none"`, `"list"`, `"dict"`, `"bool"`.

Check if a variable is defined:

```jinja
{% if some_var is defined %}
  Has value: {{ some_var }}
{% else %}
  Variable is undefined
{% endif %}
```

## Available context

### In page templates (`page.html`)

```jinja
{{ page.title }}        {# Page title #}
{{ page.content }}      {# Rendered HTML body #}
{{ page.permalink }}    {# URL path like "/guide/intro" #}
{{ page.weight }}       {# Sort order #}
{{ page.toc }}          {# Table of contents (list of headings) #}
{{ page.ancestors }}    {# Breadcrumb chain (list of sections) #}
{{ page.last_updated }} {# Last modification timestamp #}
{{ page.extra }}        {# Custom frontmatter fields #}

{{ section.title }}     {# Parent section #}
{{ section.pages }}     {# Sibling pages #}
```

### In section templates (`section.html`, `index.html`)

```jinja
{{ section.title }}       {# Section title #}
{{ section.content }}     {# Rendered HTML body from _index.md #}
{{ section.permalink }}   {# URL path #}
{{ section.pages }}       {# Pages in this section (sorted by weight) #}
{{ section.subsections }} {# Child sections #}
{{ section.toc }}         {# Table of contents #}
{{ section.extra }}       {# Custom frontmatter fields #}
```

### Global variables (all templates)

```jinja
{{ config.title }}       {# Site title #}
{{ config.base_url }}    {# Base URL #}
{{ current_path }}       {# Current page's path #}
{{ root }}               {# Root section (for navigation) #}
{{ data }}               {# Custom data files (if any) #}
```

## Common gotchas

### Lists vs single values

Iterating over something that isn't a list:

```jinja
{# This fails if page.extra.tags is a string, not a list #}
{% for tag in page.extra.tags %}
  {{ tag }}
{% endfor %}

{# Check first #}
{% if page.extra.tags is defined %}
  {% for tag in page.extra.tags %}
    {{ tag }}
  {% endfor %}
{% endif %}
```

### None vs empty string vs undefined

These are all different:

```jinja
{% if value is undefined %}     {# Variable doesn't exist at all #}
{% if value is defined %}       {# Variable exists (might be none/empty) #}
{% if value %}                  {# Truthy: not none, not empty, not 0 #}
{% if value is empty %}         {# Empty string "" or empty list [] #}
```

Use `default` filter for fallbacks:

```jinja
{{ page.description | default(value="No description") }}
```

### Filter order matters

Filters apply left to right:

```jinja
{{ items | first | upper }}     {# Get first, then uppercase it #}
{{ items | sort | first }}      {# Sort first, then get first #}
```

### Accessing nested fields

Use dot notation:

```jinja
{{ page.extra.author }}
{{ section.subsections | first | attr(name="title") }}
```

## Reading error messages

Dodeca provides detailed error diagnostics. Example:

```
Error: template::unknown_field

  × Unknown field `titl` on object
   ╭─[templates/page.html:5:1]
 5 │ <h1>{{ page.titl }}</h1>
   ·              ──┬─
   ·                ╰── unknown field
   ╰────
  help: Available fields: title, content, permalink, path, weight, toc, ancestors, last_updated, extra
```

The error shows:
- **Error type**: `unknown_field` - you're accessing something that doesn't exist
- **Location**: `templates/page.html:5` - file and line number
- **Context**: The actual template code with the problem highlighted
- **Help**: Available alternatives

## Template inheritance issues

When using `{% extends "base.html" %}`:

1. The child template must define blocks that exist in the parent
2. Content outside blocks is ignored in child templates
3. Use `{{ super() }}` to include parent block content

```jinja
{% extends "base.html" %}

{% block content %}
  {{ super() }}  {# Include parent's content block first #}
  <p>Additional content</p>
{% endblock %}
```

## Useful filters for debugging

```jinja
{{ value | typeof }}              {# Get type name #}
{{ items | length }}              {# Count items #}
{{ items | first }}               {# First element #}
{{ items | last }}                {# Last element #}
{{ text | escape }}               {# HTML-escape for safe display #}
```

## Quick reference: available tests

Use with `is` in conditions:

```jinja
{% if value is defined %}
{% if value is undefined %}
{% if value is string %}
{% if value is number %}
{% if value is empty %}
{% if value is odd %}
{% if value is even %}
{% if value is truthy %}
{% if path is starting_with("/admin") %}
{% if text is containing("search") %}
```
