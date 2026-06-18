# Oval objects

An oval is a box in which the narrow ends are formed by semicircles.
If the height is less than the width (the default) then the semicircles
are on the left and right.  If the width is less than the height, then the
semicircles are on the top and bottom:

~~~~ pikchr indent toggle
A: oval thick
X0: line thin color gray left 70% from 2mm left of (A.w,A.n)
X1: line same from 2mm left of (A.w,A.s)
text "height" at (7/8<previous.start,previous.end>,1/2<X0,X1>)
line thin color gray from previous text.n up until even with X0 ->
line thin color gray from previous text.s down until even with X1 ->
X2: line thin color gray down 50% from 2mm below (A.w,A.s)
X3: line thin color gray down 50% from 2mm below (A.e,A.s)
text "width" at (1/2<X2,X3>,6/8<X2.start,X2.end>)
line thin color gray from previous text.w left until even with X2 ->
line thin color gray from previous text.e right until even with X3 ->

A: oval thick wid A.ht ht A.wid at 2.0*A.wid right of A
X0: line thin color gray left 70% from 2mm left of (A.w,A.n)
X1: line same from 2mm left of (A.w,A.s)
text "height" at (7/8<previous.start,previous.end>,1/2<X0,X1>)
line thin color gray from previous text.n up until even with X0 ->
line thin color gray from previous text.s down until even with X1 ->
X2: line thin color gray down 50% from 2mm below (A.w,A.s)
X3: line thin color gray down 50% from 2mm below (A.e,A.s)
text "width" at (1/2<X2,X3>,6/8<X2.start,X2.end>)
line thin color gray from previous text.w left until even with X2 ->
line thin color gray from previous text.e right until even with X3 ->
~~~~

An oval works like a [box](./boxobj.md) in which the radius is
set to half the minimum of the height and width.  An oval where the
width and height are the same is a [circle](./circleobj.md)


## Boundary points:

~~~~ pikchr indent toggle
A: oval thin
dot ".c" below at A
dot ".n" above at A.n
dot " .ne" ljust above at A.ne
dot " .e" ljust at A.e
dot " .se" ljust below at A.se
dot ".s" below at A.s
dot ".sw " rjust below at A.sw
dot ".w " rjust at A.w
dot ".nw " rjust above at A.nw

A: oval thin  wid A.ht ht A.wid at 2.0*A.wid right of A
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
