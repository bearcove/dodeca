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

Templates receive `page` (title, content, permalink, toc), `section` (title, content, pages, subsections), and `config`.

Filters: `safe` (no escaping), `upper`, `lower`, `trim`, `default(value)`.

All output is HTML-escaped by default. Use `| safe` for pre-rendered HTML like `page.content`.
