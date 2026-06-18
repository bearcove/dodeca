# Arrowheads

Line objects ("line", "arrow", "spline", and "arc") can have one
of the following attributes to specify which ends of the line contain
arrowheads:

  *  **-&gt;**
  *  **&lt;-**
  *  **&lt;-&gt;**

The first form (**-&gt;**) means that there is an arrowhead at the end.
This is the default for "arrow".  The second form (**&lt;-**) means that
there is an arrowhead at the beginning only.  The third form means that
there are arrowheads at both ends.

Note that "`arrow`" and "`line ->`" look identical to one another.

If there are multiple occurrences of these attributes on a single object,
then the last one is the one that matters.

## Enhancement 2021-06-11

To make it easier to embed pikchr scripts inside of larger HTML documents,
the arrow direction tokens now have alternative spellings.

| Legacy ASCII | HTML Entity           | Unicode Character |
------------------------------------------------------------
| &lt;-        | &amp;larr;            | &larr;            |
| &lt;-        | &amp;leftarrow;       | &leftarrow;       |
| -&gt;        | &amp;rarr;            | &rarr;            |
| -&gt;        | &amp;rightarrow;      | &rightarrow;      |
| &lt;-&gt;    | &amp;leftrightarrow;  | &leftrightarrow;  |

All the tokens in any row of the table above mean the same thing
to Pikchr and can be freely interchanged.  So, in other words,
each of the following Pikchr statements means the same thing:

  *  `line ->`
  *  `line &rarr;`
  *  `line &rightarrow;`
  *  `line →`
