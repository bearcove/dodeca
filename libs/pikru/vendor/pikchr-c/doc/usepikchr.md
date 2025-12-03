# How To Use Pikchr In Fossil Markdown and Fossil Wiki Markup

## Embedded Pikchr In Markdown

To include a Pikchr diagram in the middle of a Markdown document
in [Fossil][fossil], simple place the Pikchr script in a [fenced code block][fcb]
that has a class of "pikchr".  Like this:

~~~~~
   ~~~ pikchr
   arrow; box "Hello!"; arrow
   ~~~
~~~~~

Or like this:

~~~~~
   ``` pikchr
   arrow; box "Hello!"; arrow
   ```
~~~~~

The result appears as follows:

``` pikchr
arrow; box "Hello!"; arrow
```


[fcb]: https://spec.commonmark.org/0.29/#fenced-code-blocks

## Image Placement and Source Code Toggle Options

By default, the Pikchr-generated image appears left-justified.  If you
would prefer that the picture be centered, put the keyword "center"
after the class tag.  Thus:

~~~~~
   ``` pikchr center
   arrow; box "Hello" "again"; arrow <-
   ```
~~~~~

Results in the following:

``` pikchr center
arrow; box "Hello" "again"; arrow <-
```

Pikchrs embedded in Fossil documents can be toggled between their SVG
and raw source code forms using the `ctrl` key and left mouse button,
noting that (A) "ctrl" is actually environment-dependent and might be
"alt" or the Mac-specific "Command" key, and (B) this feature requires
that JavaScript is activated. If the source code should also be
revealed by a simple click, add the word "toggle" after the `pikchr`
class tag, as in this click-toggleable example:

``` pikchr center toggle
arrow ->; box "Click to" "toggle"; arrow <-
```

Notice that the source code is displayed left-aligned by default. It can
be configured to display in the same approximate position as the image
by adding the "source-inline" modifier:

``` pikchr center toggle source-inline
arrow ->; box "Click to" "toggle" "(centered)"; arrow <-
```

The full list of such modifiers, in alphabetical order:

- `center`: center-aligns the image. The default is left-aligned.
- `float-left` and `float-right`: "float" the image to the
  left resp. right, such that other content will flow around them.
- `indent`: left-aligns the pikchr, indented by some CSS-specified
  amount.
- `source`: defaults to source code view instead of SVG view.
- `source-inline`: places the source code view, when revealed, at
  approximately the same position as the SVG, instead of left-aligned.
  The source's size may vary wildly from the image's, so this
  placement is in the same *relative* position, rather than a precise
  fit.
- `toggle`: indicates that a single click/tap is required to toggle
  between SVG and source code views. This is primarily intended to be
  used in documents which are about learning how to write pikchr
  code. For consistency, `ctrl`-click also works on pikchrs tagged
  with this modifier.

## Embedded Pikchr In Fossil Wiki

[Fossil][fossil] supports its own document markup format called
(uncreatively) "[Fossil Wiki][fossilwiki]".  Fossil Wiki is basically
just a safe subset of HTML with a few simple hyperlink, paragraph break, and
list formatting enhancements. Fossil Wiki supports something
similar to "fenced code blocks" in a special "`<verbatim>`" tag.
All content in between "`<verbatim>`" and the next
"`</verbatim>`", is displayed exactly as written.  Hence, just as
fenced code blocks are used to add Pikchr support to Markdown, so too
the "`<verbatim>`" mechanism is used to add Pikchr support to Fossil Wiki.

In Fossil Wiki, Pikchr source text can be enclosed within
"`<verbatim type="pikchr">`" or "`<verbatim type="pikchr center"`>
through the next "`</verbatim>`" tag.  Hence, code like this:

~~~~~
   <verbatim type="pikchr center">
   arrow; ellipse "Hi, Y'all"; arrow
   </verbatim>
~~~~~

Will result in a display like this:

~~~ pikchr center
arrow; ellipse "Hi, Y'all"; arrow
~~~

Fossil's `verbatim` format supports the same range of modifiers as the
Markdown format, as described above: simply add each modifier in the
`type` attribute after the word `pikchr`.

[fossil]: https://fossil-scm.org/home
[fossilwiki]: /wiki_rules
