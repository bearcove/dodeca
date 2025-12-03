+++
title = "Template Engine"
weight = 20
+++

dodeca includes a Jinja-like template engine built for tight integration with Salsa's incremental computation.

```jinja
{% extends "base.html" %}

{% block content %}
  <h1>{{ page.title }}</h1>
  {{ page.content | safe }}

  {% for p in section.pages %}
    <a href="{{ p.permalink }}">{{ p.title }}</a>
  {% endfor %}
{% endblock %}
```

Templates receive `page` (title, content, permalink, path, weight, toc, ancestors, last_updated), `section` (title, content, pages, subsections, last_updated), and `config`.

The `ancestors` field is an ordered list of parent sections from root to immediate parent, useful for breadcrumbs:

```jinja
{% for ancestor in page.ancestors %}
  <a href="{{ ancestor.permalink }}">{{ ancestor.title }}</a> /
{% endfor %}
{{ page.title }}
```

The `last_updated` field is a Unix timestamp (seconds since epoch) of the source file's modification time, useful for "last updated" notices:

```jinja
Last updated: {{ page.last_updated }}
```

Filters: `safe` (no escaping), `upper`, `lower`, `trim`, `default(value)`.

All output is HTML-escaped by default. Use `| safe` for pre-rendered HTML like `page.content`.
