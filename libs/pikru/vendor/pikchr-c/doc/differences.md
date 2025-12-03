# Differences Between Pikchr And Legacy-PIC

Pikchr is mostly compatible with legacy PIC in the sense that it will
run most of the example scripts contained in the
[original technical report on PIC by BWK][bwk] with little to no change.
Nevertheless, some features of legacy PIC have been omitted, and new
features have been added.  This article attempts to highlight the
important differences.

[bwk]: /uv/pic.pdf

Pikchr is implemented from scratch, without reference to the original
PIC code, and without even access to a working version of legacy PIC
with which to perform experiments.  The syntax implemented by Pikchr
is based solely on the descriptions in the [BWK tech report][bwk] which was
intended as a user manual, not a precise description of the language.
Consequently, some details of Pikchr may differ from PIC without our
even being aware of it.  This document tries to list the differences
that we know of.  But there are likely omissions.

## Designed for the Web

Pikchr is designed to be embedded in Markdown, generating SVG
output embedded into the resulting HTML.  It is
intended for use in software development and software project
management systems for the 2020s and beyond. Examples of this focus are
Pikchr’s understanding of [CSS color names][color] and its allowance for
Unicode arrows (→, ←, ↔) as [directions in `arrow` commands][adir] to
avoid problems with angle bracket interpretation in SVG+HTML.

[adir]:  ./arrowdir.md
[color]: ./colorexpr.md

PIC was designed to be embedded in [troff][troff] - an historically
significant but now obsolete markup language developed at Bell Labs
in the late 1970s and early 1980s.
PIC could include troff markup in the middle of
a drawing, a capability omitted from Pikchr (obviously).

[troff]: https://en.wikipedia.org/wiki/Troff

## New Object Types

Pikchr supports several new object types that were unavailable
in PIC.

~~~ pikchr indent
oval "oval"
move
diamond "diamond"
move
cylinder "cylinder"
move
file "file"
move
dot "  dot" ljust
~~~

Additional object types may be added in subsequent versions of Pikchr.

## Units Other Than Inches

PIC operated purely in inches.  Pikchr allows you to attach a
units designator on numeric literals so that distances can be easily
expressed in other units.  For example, you can write "`2.3cm`" to
mean 2.3 centimeters.  This is easier and more intuitive than writing
something like
"`2.3/2.54`".  Pikchr still does all of its calculations in inches,
internally.  The "cm" suffix is actually part of the numeric literal
so that "`2.3cm`" is really just an alternative spelling for "`0.905`".

Units supported by Pikchr include:

  *  `cm` &rarr; centimeters
  *  `in` &rarr; inches (the default)
  *  `mm` &rarr; millimeters
  *  `pc` &rarr; picas
  *  `pt` &rarr; points
  *  `px` &rarr; pixels

Because the units are part of the numeric literal,
the unit designator cannot be separated from the number by whitespace.
Units only apply to numeric literals, not to expressions.


## New Uses For "`radius`":

A positive "`radius`" attribute on "`box`" items causes the box
to be displayed with rounded corners:

~~~ pikchr indent
box rad 15px "box" "radius 15px"
~~~

Similarly a "`radius`" value on a "`line`" or "`arrow`" with
multiple segments rounds the corners:

~~~ pikchr indent
arrow rad 10px go heading 30 then go 200% heading 175 \
  then go 150% west "arrow" below "radius 10px" below
~~~

## The "`color`" and "`fill`" attributes

Any object can have a "`color`" attribute to set its foreground
color and a "`fill`" attribute to set its background color.  The
default "`color`" is black and the default "`fill`" is "None".

~~~ pikchr indent
boxrad = 12px
box color blue "color blue"
move
box fill lightgray "fill lightgray"
move
box color white fill blue "color white" "fill blue"
~~~

## The "`thickness`" attribute

The new "`thickness`" attribute specifies the stroke-width.  You can
also use attributes "`thick`" and "`thin`" to increase or decrease the
stroke-width in increments.

~~~ pikchr indent
boxrad = 12px
box thin "thin"
move
box "(default)" italic
move
box thick "thick"
move
box thick thick "thick" "thick"
~~~

## The "`behind`" attribute

The new ["`behind`" attribute](./behind.md) can be used to control
object stacking order.

## Enhanced ability to control text alignment and display

There are new modifiers for text labels:

~~~ pikchr indent
box "bold" bold "italic" italic "big" big "small" small "monospace" mono fit
line from 1cm right of previous.se to 3cm right of previous.ne \
   "aligned" above aligned
~~~

## Adjust the size of objects to fit their text annotations

The ["`fit`" attribute](./fit.md) adjusts the width and height of
box-like objects to snugly surround their text labels.

Also, if the width or height of an object is zero after all attributes
have been parsed, then the zero dimensions are increased to enclose
the text annotations.

## Change numeric property values by a percentage

You can change the value of a numeric attribute by a percentage,
rather than having to specify a particular value:

~~~ pikchr indent
box "default" italic "box" italic
move
box "width 150%" width 150%
move
box "wid 75%" wid 75%
~~~

## The "`chop`" attribute works differently

The "`chop`" attribute is completely redesigned.  It takes no
argument and can only appear once.  If "`chop`" is specified on
a line (or arrow or spline) then end-points of the line that
would have landed on the center of a box-like object (box,
circle, cylinder, diamond, ellipse, file, or oval) are shortened to
land exactly on the border of that object.  

~~~ pikchr indent
file "A"
cylinder "B" at 5cm heading 125 from A
arrow <-> from A to B chop "from A to B chop" aligned above
~~~

## The "`same as` *object*" construct

An ordinary "`same`" attribute works as in PIC - it copies the
configuration of the previous object of the same class.  Pikchr
is extended with the "`same as` *object*" clause, that copies the
configuration from any other prior object, including objects of
different types.

~~~ pikchr indent
box thick thick fill lightgray "box" "thick" "fill lightgray"
move
file same as last box "file" "same as" "last box" rad filerad
~~~

## New ways to describe line paths

  *  **go** *distance* **heading** *compass-angle*
  *  **go** *distance* *compass-point*
  *  **go** *direction* **until even with** *place*
  *  **close**

## New syntax to describe positions

  *  *distance* **above**|**below** *position*
  *  *distance* **left**|**right** **of** *position*
  *  *distance* **heading** *compass-angle* **from** *position*
  *  *nth* **vertex of** *line-object*


## New ways to identify prior objects

Pikchr allows the keywords "`last`" or "`previous`" to refer to
the immediately previous object without having to specify the
type of that object.

Objects that contain text that looks like a label (starts with
an upper-case letter and contains only letters, digits, and underscores)
can be used as a label for that object.  Thus if you say:

~~~
  N1: circle "Node1"
~~~

Subsequent code can refer to that circle as either "`N1`" or as "`Node1`".

## Support for C and C++ style comments

Pikchr continues to support Bourne shell style “`#`” comments:
a `#` character and all following
characters until end-of-line.

As an extension to PIC, Pikchr also recognizes
C and C++ style comments:  “`//`” to end of line and block comments
beginning with “`/*`”, extending through “`*/`”, irrespective of
any intervening newlines.

*Example:*

        box "Hello,"            # say “hi”
        box "world!"            // complete the thought
        box "Hello," "world!!"  /* You may also break the
                                   lines, like this. */

## Variable names can start with "`$`" or "`@`" characters

There are many built-in variable names and keywords in the PIC and
Pikchr languages, all of which currently begin with lowercase letters.  To
reduce the chance of a collision between an application-defined
variable and a built-in variable name or keyword, Pikchr allows
application-defined variable names to begin with "`$`" or "`@`".
Pikchr does not now — nor will it ever — pre-define variables that
begin with "`$`" or "`@`", other than the use of positional macro
parameters `$1`, `$2`, etc.

We recommend that you begin your own variable names with either
"`$`" or "`@`" to ensure that they will never collide with variables
that might be added to future version of Pikchr.

## New assignment operators for variables

Both Pikchr and PIC allow statements that assign values to
built-in or user-defined variables, like this:

>  *variable* **=** *expr*

Pikchr adds several new assignment operators:

  *  +=
  *  -=
  *  *=
  *  /=

The new operators are handy for scaling the value of an existing
variable.  For example, to make the default radius of circles
25% smaller:

~~~~
   circlerad *= 0.75
~~~~

## New keyword aliases

Pikchr allows certain aliases for keywords that are not
recognized by PIC:

  *  "`invisible`" &lrarr; "`invis`"
  *  "`first`" &lrarr; "`1st`"
  *  "`previous`" &lrarr; "`last`"

## The "`text`" Object

With PIC, you create new text items by placing a string
literal as the first token in a statement.  Pikchr works the
same way, and further allows you to use the class name "`text`"
as the first token of the statement.

## New variables

  *  bottommargin
  *  charht
  *  charwid
  *  color
  *  fill
  *  fontscale
  *  leftmargin
  *  margin
  *  rightmargin
  *  thickness
  *  topmargin

If the "fontscale" variable exists and is not 1.0, then the point-size
of fonts is increased or decreased by multiplying by the fontscale.
This variable can be used to increase or decrease the fonts in a
diagram relative to all the other elements.

The "charht" and "charwid" variables should contain an estimate for
the average height and width of a character.  This information is used
when trying to estimate the size of text.  Because Pikchr has no access
to the rendering engine, it cannot precisely determine the bounding box
for text strings.  It tries to make a guess, and takes into account that
some letters (like "w") are wider than others (like "i").  But Pikchr
can only guess at the actual size of text strings.  Usually this guess
is close enough.  Some scripts might need to compensate, however, by
adding leading or trailing spaces to the text strings, or by adjusting
the values for "charht" and "charwid".

Setting the "`margin`" variable to a distance adds that amount of
extra whitespace around all four sides of the diagram.  The other
four margin variables ("rightmargin", "bottommargin", "leftmargin",
and "topmargin") add extra whitespace to that one side.  The two
methods are additive.  For example, to add one centimeter of extra
space on all sides except the left, you could write:

~~~
     margin = 1cm;
     leftmargin = -1cm;
~~~

The "thickness", "color", and "fill" variables determine the default
value for the "thickness", "color", and "fill" attributes on all objects.
Because the attribute name and the variable name are the same, the
variable name can only be accessed from inside of parentheses, to avoid
parsing ambiguities.  For example, to set the thickness of a box to
be twice the default thickness:

~~~~
     box thickness 2*(thickness)
     ###             ^^^^^^^^^^^---- must be inside (...)
~~~~

The extra parentheses around variables "thickness", "color", and "fill"
are only required when the values are being read, not when the variable
name appears on the left-hand size of an assignment.  You still do:

~~~~
     thickness *= 1.5
~~~~

## Scales work differently in Pikchr

In pikchr, scale-related variables like `scale` and `fontscale` work
differently: they are multipliers to apply to values. This is in
contrast to PIC, where dimensions are divided by the `scale`.


## The "`arc`" object does not actually draw an arc.

The behavior of the "`arc`" object is underspecified in the original
[BWK paper on PIC][bwk].  Nobody is sure exactly what "arc" is supposed
to do. Furthermore, arcs seem to be seldom used.
Splines and lines with a radius at corners are better mechanisms
for drawing curvy lines in a diagram.  For these reasons, and to
keep the implementation simple, Pikchr does not actually draw an
arc for the "`arc`" object.  Instead it draws a quadratic Bézier
curve across *approximately* the same path that a true arc would have
taken.

The 30&deg; dimensional "arc" in the drawing below 
(taken from [a tutorial analysis of a Pikchr script](./teardown01.md))
is really a spline.  It is close enough to a true
arc for the purposes of Pikchr.  Can you tell the difference?

``` pikchr
scale = 0.8
linewid *= 0.5
circle "C0" fit
circlerad = previous.radius
arrow
circle "C1"
arrow
circle "C2"
arrow
circle "C4"
arrow
circle "C6"
circle "C3" at dist(C2,C4) heading 30 from C2

d1 = dist(C2,C3.ne)+2mm
line thin color gray from d1 heading 30 from C2 \
   to d1+1cm heading 30 from C2
line thin color gray from d1 heading 0 from C2 \
   to d1+1cm heading 0 from C2
spline thin color gray <-> \
   from d1+8mm heading 0 from C2 \
   to d1+8mm heading 10 from C2 \
   to d1+8mm heading 20 from C2 \
   to d1+8mm heading 30 from C2 \
   "30&deg;" aligned below small

X1: line thin color gray from circlerad+1mm heading 300 from C3 \
        to circlerad+6mm heading 300 from C3
X2: line thin color gray from circlerad+1mm heading 300 from C2 \
        to circlerad+6mm heading 300 from C2
line thin color gray <-> from X2 to X1 "distance" aligned above small \
    "C2 to C4" aligned below small
```

## Limit on the number of input tokens

Pikchr is designed to operate safely in a hostile environment on the
open internet.  For that reason, it deliberately limits
the number of tokens that will be processed in a single script.
If more tokens than the limit are seen, the script aborts with an error.

The input token limit was added to prevent a denial-of-service (DoS)
attack based on deeply nested macros.  Each time a macro is invoked, it
is rescanned and all of the tokens within the macro are added to the
running total.  Without the token limit, an attacker could devise a
script that contained nested macros that generates billions and billions
of glyphs in the final image, consuming large amounts of memory and
CPU time in the process.

The token limit is determined by the `PIKCHR_TOKEN_LIMIT` preprocessor
macro in the source code.  The default token limit is 100000, which
should be more than enough for any reasonable script.  The limit
can be increased (or decreased) at compile-time by redefining that
macro.

## Pikchr does not restore variables when leaving `[]` blocks

When a `[...]` block is exited, any variables set in that block
retain the values set in that block, rather than being reset to
their pre-block values.

## Pikchr does not have the "`Here`" keyword

PIC's `Here` keyword, to refer to the current position, is not
available in pikchr.

## Pikchr adds the `this` keyword

In pikchr, `this` may be used to refer to the current object. For example:

~~~~
     box "Some" "Text" fit height max(this.height, OtherElement.height)
~~~~

## Discontinued Features

Pikchr deliberately omits some features of legacy PIC for security
reasons.  Other features are omitted for lack of utility.

### Pikchr omits the "`sh`" and "`copy`" statements.

The "`sh`" command provided the script the ability to run arbitrary
shell commands on the host computer.  Hence "`sh`" was just a built-in
[RCE vulnerability][rce].  Having the ability to run arbitrary shell
commands was a great innovation in a phototypesetting control
system for Version-III Unix running on a PDP/11 in 1982, in a
controlled-access facility.
But such a feature is undesirable in modern web-facing applications
accessible to random passers-by on the Internet.

[rce]: https://en.wikipedia.org/wiki/Arbitrary_code_execution

The "`copy`" command is similar.  It inserts the text of arbitrary
files on the host computer into the middle of the PIC-script.

### Pikchr omits "`for`" and "`if`" statements

Pikchr omits all support for branching and looping.  Each Pikchr
statement maps directly into (at most) one graphic object in the
output.  This is a choice made to enhance the security and safety
of Pikchr (without branching or looping, there is less opportunity
for mischief) and to keep the language simple and accessible.

To be clear, we *could* in theory implement loops and branches and
subroutines in Pikchr in a safe way.  But doing so would be extra
complication, both in the implementation and in the mental model that
is maintained by the user.  Hence, in order to keep thing simple
we choose to omit those features.
If you need machine-generated code, employ a separate script
language like Python or TCL to generate the Pikchr script for
you. 

### Pikchr omits the built-in "`sprintf()`" function

The `sprintf()` function has well-known security concerns, and we
do not want to make potential exploits accessible to attackers.
Furthermore, the `sprintf()` is of little to no utility in a Pikchr
script that lacks loops.  A secure version of `sprintf()` could be
added to Pikchr, but doing that would basically require recoding
a secure `sprintf()` from from scratch.  It is safer and easier
to simply omit it.

### Pikchr omits "`{...}`" subblocks

The "`[...]`" style subblocks are supported and they work just as well.

### Pikchr omits the "`arrowhead`" variable

Pikchr does not support the `arrowhead` variable. Instead, use
`arrowht` and `arrowwid` to set the height and width, respectively,
of arrowheads.

