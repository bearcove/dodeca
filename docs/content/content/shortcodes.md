+++
title = "Shortcodes"
weight = 50
+++

Shortcodes let you embed rich content in markdown via reusable templates.

## Fenced shortcode (YAML args)

Fenced shortcodes use `+++` delimiters. The first line sets the shortcode name (`:name:`), followed by YAML key-value pairs:

```
+++
:youtube:
  url: dQw4w9WgXcQ
  title: Never gonna give you up
+++
```

This renders:

+++
:youtube:
  url: dQw4w9WgXcQ
  title: Never gonna give you up
+++

## Inline shortcode (parenthesized args)

Inline shortcodes use `*:name(key=val)*` syntax with optional args in parentheses:

```
Watch this: *:youtube(url="dQw4w9WgXcQ")*
```

Watch this: *:youtube(url="dQw4w9WgXcQ")*
