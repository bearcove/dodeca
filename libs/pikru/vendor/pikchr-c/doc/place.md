# place

A *place* is a specific point on an object.
A *[position](./position.md)* is a more general concept that means
any X,Y coordinate on the drawing.  This page is about *place*.

  *  *object*
  *  *object* *dot-edgename*
  *  *edgename* **of** *object*
  *  ORDINAL **vertex of** *object*

Every object has at least 9 places.  Line objects have additional
places for each internal vertex.   Most places are on the boundary
of the object, though ".center" or ".c" is in the middle.  The
".start" and ".end" places might be in the interior of the object
for the case of lines.
Some places may overlap.
Places usually have multiple names.
There are 22 different place names to refer to the 9 potentially
distinct places.

For a block object, when the layout direction is "right", we have:

~~~ pikchr
B: box thick thick color blue

circle ".n" fit at 1.5cm heading 0 from B.n
    arrow thin from previous to B.n chop
circle ".north" fit at 3cm heading 15 from B.north
    arrow thin from previous to B.north chop
circle ".t" fit at 1.5cm heading 30 from B.t
    arrow thin from previous to B.t chop
circle ".top" fit at 3cm heading -15 from B.top
    arrow thin from previous to B.top chop
circle ".ne" fit at 1cm ne of B.ne; arrow thin from previous to B.ne chop
circle ".e" fit at 2cm heading 50 from B.e; arrow thin from previous to B.e chop
circle ".right" fit at 3cm heading 75 from B.right
    arrow thin from previous to B.right chop
circle ".end&sup1;" fit at 3cm heading 100 from B.end
    arrow thin from previous to B.end chop
circle ".se" fit at 1cm heading 110 from B.se
    arrow thin from previous to B.se chop
circle ".s" fit at 1.5cm heading 180 from B.s
    arrow thin from previous to B.s chop
circle ".south" fit at 3cm heading 195 from B.south
    arrow thin from previous to B.south chop
circle ".bot" fit at 1.8cm heading 215 from B.bot
    arrow thin from previous to B.bot chop
circle ".bottom" fit at 2.7cm heading 160 from B.bottom
    arrow thin from previous to B.bottom chop
circle ".sw" fit at 1cm sw of B.sw; arrow thin from previous to B.sw chop
circle ".w" fit at 2cm heading 270 from B.w
    arrow thin from previous to B.w chop
circle ".left" fit at 3cm heading 180+75 from B.left
    arrow thin from previous to B.left chop
circle ".start&sup1;" fit at 2.5cm heading 295 from B.start
    arrow thin from previous to B.start chop
circle ".nw" fit at 1cm nw of B.nw; arrow thin from previous to B.nw chop
circle ".c" fit at 2.5cm heading -25 from B.c
    line thin from previous to 0.5<previous,B.c> chop
    arrow thin from previous.end to B.c
circle ".center" fit at 3.6cm heading 180-44 from B.center
    line thin from previous to 0.5<previous,B.center> chop
    arrow thin from previous.end to B.center
circle "&lambda;" fit at 2.5cm heading 250 from B
    line from previous to 0.5<previous,B> chop
    arrow thin from previous.end to B
~~~

The diagram above is for a box with square corners.  The non-center
places for other block objects are always on the boundary of the
object.  Thus for an ellipse:

~~~ pikchr
B: ellipse thick thick color blue

circle ".n" fit at 1.5cm heading 0 from B.n
    arrow thin from previous to B.n chop
circle ".north" fit at 3cm heading 15 from B.north
    arrow thin from previous to B.north chop
circle ".t" fit at 1.5cm heading 30 from B.t
    arrow thin from previous to B.t chop
circle ".top" fit at 3cm heading -15 from B.top
    arrow thin from previous to B.top chop
circle ".ne" fit at 1cm ne of B.ne; arrow thin from previous to B.ne chop
circle ".e" fit at 2cm heading 50 from B.e; arrow thin from previous to B.e chop
circle ".right" fit at 3cm heading 75 from B.right
    arrow thin from previous to B.right chop
circle ".end&sup1;" fit at 3cm heading 100 from B.end
    arrow thin from previous to B.end chop
circle ".se" fit at 1cm heading 110 from B.se
    arrow thin from previous to B.se chop
circle ".s" fit at 1.5cm heading 180 from B.s
    arrow thin from previous to B.s chop
circle ".south" fit at 3cm heading 195 from B.south
    arrow thin from previous to B.south chop
circle ".bot" fit at 1.8cm heading 215 from B.bot
    arrow thin from previous to B.bot chop
circle ".bottom" fit at 2.7cm heading 160 from B.bottom
    arrow thin from previous to B.bottom chop
circle ".sw" fit at 1cm sw of B.sw; arrow thin from previous to B.sw chop
circle ".w" fit at 2cm heading 270 from B.w
    arrow thin from previous to B.w chop
circle ".left" fit at 3cm heading 180+75 from B.left
    arrow thin from previous to B.left chop
circle ".start&sup1;" fit at 2.5cm heading 295 from B.start
    arrow thin from previous to B.start chop
circle ".nw" fit at 1cm nw of B.nw; arrow thin from previous to B.nw chop
circle ".c" fit at 2.5cm heading -25 from B.c
    line thin from previous to 0.5<previous,B.c> chop
    arrow thin from previous.end to B.c
circle ".center" fit at 3.6cm heading 180-44 from B.center
    line thin from previous to 0.5<previous,B.center> chop
    arrow thin from previous.end to B.center
circle "&lambda;" fit at 2.5cm heading 250 from B
    line from previous to 0.5<previous,B> chop
    arrow thin from previous.end to B
~~~

The "&lambda;" case refers to when a bare object name is used.
A bare object name is the same as referring to the center of
the object.

In the previous two diagrams, the ".start" and ".end" objects
are marked with "&sup1;" because
the location of ".start" and ".end" varies 
according to the layout direction.  The previous diagrams assumed
a layout direction of "right".  For other layout directions, we have:

<blockquote>
<table border="1" cellpadding="10px" cellspacing="0">
<tr><th>Layout Direction<th>.start<th>.end
<tr><td>right<td>.w<td>.e
<tr><td>down<td>.n<td>.s
<tr><td>left<td>.e<td>.w
<tr><td>up<td>.s<td>.n
</table></blockquote>

For a line, the place names refer to a bounding box that
encloses the line:

~~~ pikchr
B: line thick thick color blue go 0.8 heading 350 then go 0.4 heading 120 \
    then go 0.5 heading 35 \
    then go 1.2 heading 190  then go 0.4 heading 340 "+"

   line thin dashed color gray from B.nw to B.ne to B.se to B.sw close

circle ".n" fit at 1.5cm heading 0 from B.n
    arrow thin from previous to B.n chop
circle ".north" fit at 3cm heading 15 from B.north
    arrow thin from previous to B.north chop
circle ".t" fit at 1.5cm heading 30 from B.t
    arrow thin from previous to B.t chop
circle ".top" fit at 3cm heading -15 from B.top
    arrow thin from previous to B.top chop
circle ".ne" fit at 1cm ne of B.ne; arrow thin from previous to B.ne chop
circle ".e" fit at 2cm heading 50 from B.e; arrow thin from previous to B.e chop
circle ".right" fit at 3cm heading 75 from B.right
    arrow thin from previous to B.right chop
circle ".end" fit at 2cm heading 120 from B.end
    arrow thin from previous to B.end chop
circle ".se" fit at 1cm heading 170 from B.se
    arrow thin from previous to B.se chop
circle ".s" fit at 1.5cm heading 180 from B.s
    arrow thin from previous to B.s chop
circle ".south" fit at 3cm heading 195 from B.south
    arrow thin from previous to B.south chop
circle ".bot" fit at 1.8cm heading 215 from B.bot
    arrow thin from previous to B.bot chop
circle ".bottom" fit at 2.7cm heading 160 from B.bottom
    arrow thin from previous to B.bottom chop
circle ".sw" fit at 1cm sw of B.sw; arrow thin from previous to B.sw chop
circle ".w" fit at 2cm heading 300 from B.w
    arrow thin from previous to B.w chop
circle ".left" fit at 3cm heading 280 from B.left
    arrow thin from previous to B.left chop
circle ".start" fit at 2.5cm heading 265 from B.start
    arrow thin from previous to B.start chop
circle ".nw" fit at 1cm nw of B.nw; arrow thin from previous to B.nw chop
circle ".c" fit at 2.5cm heading -15 from B.c
    line thin from previous to 0.5<previous,B.c> chop
    arrow thin from previous.end to B.c
circle ".center" fit at 3.3cm heading 110 from B.center
    line thin from previous to 0.5<previous,B.center> chop
    arrow thin from previous.end to B.center
circle "&lambda;" fit at 1.7cm heading 250 from B
    line from previous to 0.5<previous,B> chop
    arrow thin from previous.end to B
~~~

The ".start" of a line always refers to its first vertex.
The ".end" is usually the last vertex, except when the "`close`"
keyword is used, in which case the ".end" is the same as
".e", ".s", ".w", or ".n" depending on layout direction,
just like a block object.

The vertexes of a line object are also places:

~~~ pikchr
B: line -> thick color blue go 0.8 heading 350 then go 0.4 heading 120 \
    then go 0.5 heading 35 \
    then go 1.2 heading 190  then go 0.4 heading 340

oval "1st vertex" fit at 2cm heading 250 from 1st vertex of B
    arrow thin from previous to 1st vertex of B chop
oval "2nd vertex" fit at 2cm west of 2nd vertex of B
    arrow thin from previous to 2nd vertex of B chop
oval "3rd vertex" fit at 2cm north of 3rd vertex of B
    arrow thin from previous to 3rd vertex of B chop
oval "4th vertex" fit at 2cm east of 4th vertex of B
    arrow thin from previous to 4th vertex of B chop
oval "5th vertex" fit at 2cm east of 5th vertex of B
    arrow thin from previous to 5th vertex of B chop
oval "6th vertex" fit at 2cm heading 200 from 6th vertex of B
    arrow thin from previous to 6th vertex of B chop
~~~
