# numeric-property

There are really only four numeric properties:

  * width
  * height
  * radius
  * thickness

The width and height are the size of most objects.  The radius is used
to set the size of circles.  The thickness value is the width of lines used to
draw each object.  The other property names are just aliases for these
four:

  * wid &rarr; an abbreviation for "width"
  * ht &rarr; an abbreviation for "height"
  * rad &rarr; an abbreviation for "radius"
  * diameter &rarr;  twice the radius

## Radius Of A "box" Object

By default, boxes have a radius of 0.  But if you assign a positive
radius to a box, it causes the box to have rounded corners:

~~~~~ pikchr center
box "radius 0"
move
box "radius 5px" rad 5px
move
box "radius 20px" rad 20px
~~~~~

## Dimensions Of A "circle" Object

If you change any of the "width", "height", "radius", or "diameter" of
a circle, the other three values are set automatically.

## Radius Of A "cylinder" Object

The "radius" of a "cylinder" object is the semiminor axis of the ellipse
that forms the top of the "cylinder".

~~~~~ pikchr center
C: cylinder
line thin left from C.nw - (2mm,0)
line thin left from C.nw - (2mm,C.radius)
arrow <- from 3/4<first line.start,first line.end> up 30%
arrow <- from 3/4<2nd line.start,2nd line.end> down 30%
text "radius" above at end of 1st arrow
~~~~~

Some examples:

~~~~~ pikchr center
cylinder "radius 50%" rad 50%
move
cylinder "radius 100%" rad 100%
move
cylinder "radius 200%" "height 200%" rad 200% ht 200%
~~~~~


## Radius Of A "file"

For a "file" object, the radius is the amount by which the upper right
corner is folded over.

~~~~~ pikchr center
F: file
line thin from 2mm right of (F.e,F.n) right 75%
line thin from F.rad below start of previous right 75%
arrow <- from 3/4<first line.start,first line.end> up 30%
arrow <- from 3/4<2nd line.start,2nd line.end> down 30%
text "radius" above at end of 1st arrow
~~~~~

## Radius Of A "line"

Setting a radius on a line causes the corners to be rounded by that
amount.

~~~~~ pikchr center
line go 2cm heading 40 then 4cm heading 165 then 1cm heading 280\
   "radius" "0"
move to 3cm right of previous.start
line same "radius" "15px" rad 15px
move to 3cm right of previous.start
line same  "radius" "30px" rad 30px
~~~~~
