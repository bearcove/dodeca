+++
title = "Template Reference"
weight = 30
+++

Quick-reference tables for everything available in gingembre templates.

## Filters

| Filter | Args | Description |
|--------|------|-------------|
| `upper` | — | Uppercase string |
| `lower` | — | Lowercase string |
| `capitalize` | — | Capitalize first letter |
| `title` | — | Title Case |
| `trim` | — | Strip leading/trailing whitespace |
| `escape` | — | HTML-escape |
| `safe` | — | Output as raw HTML |
| `length` | — | Number of items/characters |
| `first` | — | First item of sequence |
| `last` | — | Last item of sequence |
| `reverse` | — | Reverse a sequence |
| `sort` | — | Sort ascending |
| `join` | `sep` | Join sequence into string |
| `split` | `pat` | Split string into array |
| `default` | `value` | Fallback if undefined/null |
| `typeof` | — | Type name as string |
| `slice` | `start`, `end` | Subsequence |
| `map` | `attribute` | Extract attribute from each item |
| `selectattr` | `attribute` | Keep items where attribute is truthy |
| `rejectattr` | `attribute` | Remove items where attribute is truthy |
| `groupby` | `attribute` | Group items by attribute value |
| `path_segments` | — | Split URL path into segments |
| `path_first` | — | First segment of URL path |
| `path_parent` | — | Parent URL path |
| `path_basename` | — | Last segment of URL path |

## Functions

| Function | Args | Returns |
|----------|------|---------|
| `get_section` | `path` | Section object with `title`, `permalink`, `pages`, `subsections`, etc. |
| `get_url` | `path` | URL string for a content path |
| `now` | `format` (default: `%Y-%m-%d`) | Formatted current date/time |
| `build` | positional: step name | Output of the configured build step |
| `read` | `file` | Contents of a file as a string |
| `throw` | positional: message | Aborts rendering with an error |

## Tests

Use with `{% if value is test_name %}`.

| Test | Args | Description |
|------|------|-------------|
| `defined` | — | Variable exists |
| `undefined` | — | Variable doesn't exist |
| `none` | — | Value is null |
| `string` | — | Value is a string |
| `number` | — | Value is a number |
| `mapping` | — | Value is an object/dict |
| `iterable` | — | Value is a sequence/array |
| `odd` | — | Number is odd |
| `even` | — | Number is even |
| `divisibleby` | N | Divisible by N |
| `containing` | substring | Contains substring or item |
| `starting_with` | prefix | Starts with prefix |
| `ending_with` | suffix | Ends with suffix |
| `matching` | regex | Matches regular expression |
| `eq` | value | Equal to |
| `ne` | value | Not equal to |
| `lt` | value | Less than |
| `gt` | value | Greater than |
| `le` | value | Less than or equal |
| `ge` | value | Greater than or equal |

## Template tags

| Tag | Description |
|-----|-------------|
| `{% extends "file.html" %}` | Inherit from base template |
| `{% block name %}...{% endblock name %}` | Define/override a block |
| `{% include "file.html" %}` | Include another template |
| `{% macro name(args) %}...{% endmacro name %}` | Define a macro |
| `{% for item in list %}...{% endfor %}` | Loop |
| `{% if cond %}...{% elif %}...{% else %}...{% endif %}` | Conditional |
| `{% set var = value %}` | Set a variable |
| `{# comment #}` | Comment (not in output) |
