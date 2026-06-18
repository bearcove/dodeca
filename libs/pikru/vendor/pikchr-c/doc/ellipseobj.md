# Ellipse Objects

The shape of an ellipse is determined solely by its height and width.

~~~~ pikchr indent toggle
A: ellipse thick
line thin color gray left 70% from 2mm left of (A.w,A.n)
line same from 2mm left of (A.w,A.s)
text "height" at (7/8<previous.start,previous.end>,1/2<1st line,2ndline>)
line thin color gray from previous text.n up until even with 1st line ->
line thin color gray from previous text.s down until even with 2nd line ->
X1: line thin color gray down 50% from 2mm below (A.w,A.s)
X2: line thin color gray down 50% from 2mm below (A.e,A.s)
text "width" at (1/2<X1,X2>,6/8<X1.start,X1.end>)
line thin color gray from previous text.w left until even with X1 ->
line thin color gray from previous text.e right until even with X2 ->
~~~~

Unlike a circle, ellipses have no radius, but if the
width and height are equal, it is visually identical to a circle.


## Boundary Points

~~~~ pikchr indent toggle
A: ellipse thin
dot ".c" below at A
dot ".n" above at A.n
dot " .ne" ljust above at A.ne
dot " .e" ljust at A.e
dot " .se" ljust below at A.se
dot ".s" below at A.s
dot ".sw " rjust below at A.sw
dot ".w " rjust at A.w
dot ".nw " rjust above at A.nw
~~~~
