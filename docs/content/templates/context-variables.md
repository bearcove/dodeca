+++
title = "Context Variables"
weight = 20
+++

Different templates receive different context variables.

## Page templates (`page.html`)

| Variable | Type | Description |
|----------|------|-------------|
| `page.title` | string | From frontmatter |
| `page.content` | string | Rendered HTML (use with `\| safe`) |
| `page.permalink` | string | Full URL path |
| `page.path` | string | URL route (e.g. `/blog/my-post/`) |
| `page.weight` | integer | Sort weight |
| `page.toc` | string | Table of contents HTML |
| `page.ancestors` | array | Parent section paths |
| `page.last_updated` | string | File modification time |
| `page.description` | string | From `extra.description` (if set) |
| `page.extra` | object | Custom frontmatter fields |

## Section templates (`section.html`, `index.html`)

| Variable | Type | Description |
|----------|------|-------------|
| `section.title` | string | From frontmatter |
| `section.content` | string | Rendered HTML (use with `\| safe`) |
| `section.permalink` | string | Full URL path |
| `section.path` | string | URL route |
| `section.weight` | integer | Sort weight |
| `section.last_updated` | string | File modification time |
| `section.ancestors` | array | Parent section paths |
| `section.pages` | array | Pages in this section (sorted by weight) |
| `section.subsections` | array | Child sections (sorted by weight) |
| `section.toc` | string | Table of contents HTML |
| `section.extra` | object | Custom frontmatter fields |

Each item in `section.pages` has: `title`, `permalink`, `path`, `weight`, `toc`, `description`, `extra`.

Each item in `section.subsections` has: `title`, `permalink`, `path`, `weight`, `extra`, `pages`.

## Global variables

Available in all templates:

| Variable | Type | Description |
|----------|------|-------------|
| `config.title` | string | From root `_index.md` frontmatter `title` |
| `config.description` | string | From root `_index.md` frontmatter `description` |
| `root.subsections` | array | Top-level sections (for navigation) |

## Navigation example

A sidebar that lists all sections and their pages:

```html
{% for sub in root.subsections %}
<div class="nav-section">
    <h3><a href="{{ sub.permalink }}">{{ sub.title }}</a></h3>
    <ul>
        {% for page in sub.pages %}
        <li><a href="{{ page.permalink }}">{{ page.title }}</a></li>
        {% endfor %}
    </ul>
</div>
{% endfor %}
```
