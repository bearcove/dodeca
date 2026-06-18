# Tutorial Analysis Of A Pikchr Diagram

Let's look closely at an example Pikchr diagram to better understand
how they work.  For this analysis, we will use a diagram that depicts
a rebase operation in a version control system.  The original diagram
is found in the
[Rebase Considered Harmful][rch] document of the Fossil documentation.
The version shown here is modified slightly from the original, for example by
adding line number comments.
Click on the diagram to see the Pikchr code.

[rch]: https://fossil-scm.org/fossil/doc/trunk/www/rebaseharm.md

~~~ pikchr toggle
/* 01 */ scale = 0.8
/* 02 */ fill = white
/* 03 */ linewid *= 0.5
/* 04 */ circle "C0" fit
/* 05 */ circlerad = previous.radius
/* 06 */ arrow
/* 07 */ circle "C1"
/* 08 */ arrow
/* 09 */ circle "C2"
/* 10 */ arrow
/* 11 */ circle "C4"
/* 12 */ arrow
/* 13 */ circle "C6"
/* 14 */ circle "C3" at dist(C2,C4) heading 30 from C2
/* 15 */ arrow
/* 16 */ circle "C5"
/* 17 */ arrow from C2 to C3 chop
/* 18 */ C3P: circle "C3'" at dist(C4,C6) heading 30 from C6
/* 19 */ arrow right from C3P.e
/* 20 */ C5P: circle "C5'"
/* 21 */ arrow from C6 to C3P chop
/* 22 */ box height C3.y-C2.y \
/* 23 */     width (C5P.e.x-C0.w.x)+linewid \
/* 24 */     with .w at 0.5*linewid west of C0.w \
/* 25 */     behind C0 \
/* 26 */     fill 0xc6e2ff thin color gray
/* 27 */ box same width previous.e.x - C2.w.x \
/* 28 */     with .se at previous.ne \
/* 29 */     fill 0x9accfc
/* 30 */ "trunk" below at 2nd last box.s
/* 31 */ "feature branch" above at last box.n
~~~

Hint:  Copy the Pikchr source text and paste it into the
[](/pikchrshow) page in a separate browser window or tab so that
you can make minor changes and see the effect of those changes
as we work through the text.

## Lines 01 through 03 - modifying object size defaults

The script begins by setting some global property variables.  The
"`scale = 0.8`" line simply makes the whole diagram a little smaller
so that it fits better within its host document.  Try commenting out
that line by adding a "`#`" or "`//`" at the start to see the difference.

The "`fill = white`" line causes all objects on the graph to have a
default background fill color of white.  Without this line, the objects are
not filled at all, and so the background colors (to be inserted on lines 22
through 29) show through.  The result is still legible, but less pleasing.
Try commenting out line 02 to see what happens.  We could have
added a "`fill white`" attribute on every circle in the diagram instead,
but it's easier to set the default fill color once.

The "`linewid *= 0.5`" on line 03 shortens the default length of lines
and arrows by 50%.  Try commenting out that line.  You will see that the
arrows become twice as long, which makes the graph more spread out and
harder to read.  Shortening the arrows is an aesthetic improvement.

Even though they appear first in the script, directives like these
are typically inserted after creating the initial version of the diagram in order to clean up a diagram once
the basic structure is established.  Do not feel like you need
to start out by setting a bunch of variables.  Write the object
definitions first, and then perhaps go back and tweak the appearance
by adjusting some variable settings.

## Lines 04 and 05 - establishing the prototype node circle

Line 04 creates a circle sized to fit its label "C0".  We want
all the circles in this diagram to be the same size, so after sizing
the first one to fit the text, line 05 sets the new default circle radius
for all subsequent circles to be the same as the first circle.  This
not only saves us from having to add a "fit" on every "circle" call, it means
all circles will be of a uniform size despite containing
varying amounts of text.

## Lines 06 through 13 - the bottom row of nodes

After establishing the initial diagram node, lines 06 through 13 create
a sequence of nodes, C1, C2, C4, and C6, connected by arrows and
moving to the right.  The default Pikchr layout direction is
"right," so everything is placed automatically.

## Line 14 - drawing the first node of the first branch

We want the C3-C5 branch to be above and slightly to the right of
the C2 node.  For a pleasing appearance, it seems best to make the
distance from C2 to C3 be the same as the distance from C2 to C4.
This is accomplished by setting the location of C3 using
a clause of the form:

  *  *distance* **heading** *angle* **from** *basis*

The *basis* is C2.  The *distance* is the same as the distance from C2
to C4, and so we use the expression "`dist(C2,C4)`".  Notice here that
we are able to refer to the nodes using their text annotations because
the text annotations have the form of a valid object label: they begin
with a capital letter and consist of alphanumerics and underscores.
The *angle* is a compass heading: 0 to 360 degrees clockwise from
north.  A heading of 30 degrees means that there is a 60-degree
angle between C2-C4 and C2-C3, thus establishing C2, C3, and C4 as
the vertexes of an equilateral triangle.

~~~ pikchr toggle
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
~~~

## Lines 15 through 17 - completing the first branch

Lines 15 and 16 add the arrow and C5 node.

The arrow from C2 to C3 is drawn by line 17.  The "`chop`" attribute
causes the arrow to begin and end on node boundaries.  If you remove
the "`chop`" (try it!) the arrow will go from the center of the first
node to the center of the second, which isn't what we want.

## Lines 18 through 21 - nodes of the second branch

Lines 18 through 21 are mostly a repeat of lines 14 through 17.
The differences are (1) the branch is connected to C6 instead of C2
and (2) the nodes have different labels.
Because these node labels include "prime" marks (`'`), you cannot use
them as object labels as we could for the corresponding C3 and C5
nodes. Therefore, the nodes of this second branch are given
explicit labels "C3P" and "C5P".  Do not be bashful about adding
labels to objects.  The use of labels often makes the script much
easier to read and maintain.

## Lines 22 through 26 - background color for trunk

Lines 22 through 26 implement a single box object that provides background
color for the trunk.  Note the use of backslash ("`\`") to continue the
definition of this object across multiple lines.  It is not required to
break up the definition of the box across multiple lines; it
merely aids human
understanding.  Pikchr does not care how long your source lines are.

Some tricky calculations are involved here.  We need to figure out
an appropriate width and height for the box so that it encloses the
sequence of circles and arrows that represent the trunk, with a
comfortable margin, and we have to position the box so that the circles
and arrows are approximately centered.  The height is "`C3.y-C2.y`".
That is the equivalent of the distance between the bottom and second
row.  In this way, the division between the bottom and top row can
occur right in the middle of the two, and the margins above and below
the bottom row will be the same.  The width is sufficient to span the
entire row, plus one extra "`linewid`" for margin, to be evenly divided
between both ends.

Line 24 positions the background color box.  That line says that the
extreme western end of the background color box should be half a linewid
to the west of the extreme western end of the first node of the graph.
Recall that we allowed for one linewid of margin to be split between
both ends, so the western side is half that margin to the left of the
leftmost end of the graph.

Normally, Pikchr stacks elements on the SVG canvas in the order that they appear in the
source text, so this new background color box would normally paint
on top of the objects that come before, which would obscure the graph nodes.
To prevent this, the "`behind C0`" on line 25 tells Pikchr to paint 
this box before it paints the C0 circle, so that the background color
box occurs in the background rather than on top of the graph.
Try commenting out the "`behind C0`" to see what happens!

Finally, line 26 changes the fill color for the box to a light shade
of blue and the border to be thin and gray.

## Lines 27 through 29 - background color for the branches

Lines 27 through 29 create a second box to provide background color
to the upper branches.  The second box definition begins with the
keyword "`same`".  The "`same`" means that all of the settings to
the new box are initialized to values from the previous box.  That
means we don't have to set the height, or set "`behind C0`" or
"`thin`" or "`color gray`".  All those attributes are inherited.
The second box only has to change the width to accommodate the shorter
length of the branch it encloses in the diagram, to
adjust the background color, and to set the position.

We want the right edge of both background boxes to align, and we
want the branch background to begin a little to the left of C3.
The left edge of C2 seems like a reasonable starting point, so we
set the width to "`previous.e.x - C2.w.x`" on line 27.  The "`previous`"
refers to the previous background color box, of course.

If you had to insert a new object in between the two boxes,
it would change the referent of the "`previous`" qualifier, but only
if one or more of those new objects is itself a box. Inserting a
circle wouldn't affect it, since "`previous`" always means the previous
object of the same type.

If you had to interpose another box, you could solve the broken
reference problem by adding a label and using that label instead
of "`previous`":

~~~~
         ...
/* 22 */ BG1: box height C3.y-C2.y \
/* 23 */     width (C5P.e.x-C0.w.x)+linewid \
/* 24 */     with .w at 0.5*linewid west of C0.w \
/* 25 */     behind C0 \
/* 26 */     fill 0xc6e2ff thin color gray
/* 27 */ box same as BG1 width BG1.e.x - C2.w.x \
/* 28 */     with .se at BG1.ne \
/* 29 */     fill 0x9accfc
         ...
~~~~


Line 28 positions the second box.  The southeast (.se) corner of
the second box is set to align with the northeast (.ne) corner of
the previous box.  This causes the two boxes to be flush right
and stacked directly on top of each other.

Line 29 adjusts the background color to a darker shade of blue.

## Lines 30 and 31 - labeling the branches

Lines 30 and 31 create a pair of text objects to identify the two
branches depicted in the diagram.  

# Summary

A 31-line Pikchr script might look intimidating at first glance, but
as we see here, it is really quite simple.  No coordinates are involved,
nor any hard-coded distances.  Everything is laid out and sized relative
to other elements and to the system defaults.  This makes the diagram
portable and adjustments easy.

# Exercises

Practice your Pikchr-script writing skills by modifying the
example script as follows:

  1.  Add a new "C7" node to the right of "C6".

  2.  Now add "C8" to the right of "C7".  This one is harder because it
      will involve expanding the background color boxes.

  3.  Put the feature branch below the trunk rather than above it.

  4.  Move the "feature branch" and "trunk" labels to the left ends
      of their respective boxes, rather than centering them.

  5.  Add another branch above the "feature branch" that adds
      nodes "C9", "C10", and "C11" that fork off from "C5'".  You
      will probably need to find a new place to put the "feature branch"
      label to get it out of the way.

  6.  Add a new node and dashed line from "C5'" that illustrates "C5'"
      being merged back into trunk.

  7.  Rotate the graph so that it goes bottom-up rather than left-to-right.
