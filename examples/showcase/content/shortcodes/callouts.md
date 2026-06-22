---
title: Callouts (body + inline)
---

## Tip — body grammar

A blockquote whose first paragraph is `*:tip*`; the rest is the rendered-markdown body.

> *:tip*
>
> This is a hot tip, with **bold text**, `inline code`, and a [link](/).
> The body is real markdown, rendered before it reaches the template.

## Cool bear — body grammar with args

> *:bearsays*
>
> I am not sure about that — let me double-check.

The `mood` argument is spread into the template context:

> *:bearsays(mood=surprised)*
>
> Wait, that actually works?!

## Inline grammar

The bear can also interject inline — *:bearsays* — though without a body it is
mostly here to prove the inline grammar resolves mid-paragraph.

> Note: `tip` and `bearsays` show their avatar via `get_media(...).markup(...)`,
> which is still stubbed — so the speech bubble renders but the bear image is
> missing until `get_media` and gingembre method-calls-on-call-results land.
