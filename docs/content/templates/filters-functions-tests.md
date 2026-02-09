+++
title = "Filters, Functions & Tests"
weight = 30
+++

## Filters

Filters transform values. Use them with the pipe operator: `{{ value | filter_name }}`.

### String filters

| Filter | Description | Example |
|--------|-------------|---------|
| `upper` | Uppercase | `{{ "hello" \| upper }}` → `HELLO` |
| `lower` | Lowercase | `{{ "HELLO" \| lower }}` → `hello` |
| `capitalize` | Capitalize first letter | `{{ "hello world" \| capitalize }}` → `Hello world` |
| `title` | Title Case | `{{ "hello world" \| title }}` → `Hello World` |
| `trim` | Strip whitespace | `{{ " hi " \| trim }}` → `hi` |
| `escape` | HTML-escape | `{{ "<b>" \| escape }}` → `&lt;b&gt;` |
| `safe` | Mark as safe HTML (no escaping) | `{{ page.content \| safe }}` |
| `split` | Split into array | `{{ "a,b,c" \| split(pat=",") }}` |

### Collection filters

| Filter | Description | Example |
|--------|-------------|---------|
| `length` | Number of items | `{{ items \| length }}` |
| `first` | First item | `{{ items \| first }}` |
| `last` | Last item | `{{ items \| last }}` |
| `reverse` | Reverse order | `{{ items \| reverse }}` |
| `sort` | Sort ascending | `{{ items \| sort }}` |
| `join` | Join into string | `{{ items \| join(sep=", ") }}` |
| `slice` | Subsequence | `{{ items \| slice(start=1, end=3) }}` |
| `map` | Extract attribute | `{{ pages \| map(attribute="title") }}` |
| `selectattr` | Filter by attribute | `{{ pages \| selectattr(attribute="extra.featured") }}` |
| `rejectattr` | Exclude by attribute | `{{ pages \| rejectattr(attribute="extra.draft") }}` |
| `groupby` | Group by attribute | `{{ pages \| groupby(attribute="extra.category") }}` |

### Other filters

| Filter | Description | Example |
|--------|-------------|---------|
| `default` | Fallback value | `{{ x \| default(value="none") }}` |
| `typeof` | Type name | `{{ x \| typeof }}` → `string` |
| `path_segments` | Split URL path | `{{ "/a/b/c" \| path_segments }}` |
| `path_first` | First path segment | `{{ "/a/b/c" \| path_first }}` → `a` |
| `path_parent` | Parent path | `{{ "/a/b/c" \| path_parent }}` → `/a/b/` |
| `path_basename` | Last path segment | `{{ "/a/b/c" \| path_basename }}` → `c` |

## Functions

Call functions with `{{ function_name(arg=value) }}`.

| Function | Description | Example |
|----------|-------------|---------|
| `get_section(path=...)` | Get a section by content path | `{% set blog = get_section(path="blog/_index.md") %}` |
| `get_url(path=...)` | Get URL for a content path | `{{ get_url(path="blog/post.md") }}` |
| `now(format=...)` | Current date/time | `{{ now(format="%Y-%m-%d") }}` |
| `build(step_name)` | Run a build step | `{{ build("git_hash") }}` |
| `read(file=...)` | Read a file's contents | `{{ read(file="VERSION") }}` |
| `throw(message)` | Abort with error | `{{ throw("missing required field") }}` |

### `get_section` example

```html
{% set blog = get_section(path="blog/_index.md") %}
<h2>Latest posts</h2>
{% for post in blog.pages %}
    <a href="{{ post.permalink }}">{{ post.title }}</a>
{% endfor %}
```

### Build steps

Build steps are configured in `dodeca.styx` and executed via the `build()` function:

```styx
build_steps {
    git_hash {
        command (git rev-parse --short HEAD)
    }
}
```

```html
<footer>Built from {{ build("git_hash") }}</footer>
```

## Tests

Tests check conditions in `{% if %}` blocks. Use with the `is` keyword.

| Test | Description | Example |
|------|-------------|---------|
| `defined` | Variable exists | `{% if x is defined %}` |
| `undefined` | Variable doesn't exist | `{% if x is undefined %}` |
| `none` | Value is null | `{% if x is none %}` |
| `string` | Value is a string | `{% if x is string %}` |
| `number` | Value is a number | `{% if x is number %}` |
| `mapping` | Value is an object | `{% if x is mapping %}` |
| `iterable` | Value is a sequence | `{% if x is iterable %}` |
| `odd` | Number is odd | `{% if loop.index is odd %}` |
| `even` | Number is even | `{% if loop.index is even %}` |
| `divisibleby` | Divisible by N | `{% if x is divisibleby(3) %}` |
| `containing` | Contains substring/item | `{% if path is containing("/blog/") %}` |
| `starting_with` | Starts with prefix | `{% if path is starting_with("/docs") %}` |
| `ending_with` | Ends with suffix | `{% if path is ending_with(".html") %}` |
| `matching` | Matches regex | `{% if name is matching("^[a-z]+$") %}` |
| `eq` | Equal | `{% if x is eq(5) %}` |
| `ne` | Not equal | `{% if x is ne(0) %}` |
| `lt` / `gt` | Less / greater than | `{% if x is gt(10) %}` |
| `le` / `ge` | Less/greater or equal | `{% if x is le(100) %}` |
