# Box objects

A box is a rectangle with a specified width and height.  The default
width and height are the values of the "`boxwid`" and "`boxht`" variables.

~~~~ pikchr indent toggle
A: box thick
line thin color gray left 70% from 2mm left of A.nw
line same from 2mm left of A.sw
text "height" at (7/8<previous.start,previous.end>,1/2<1st line,2ndline>)
line thin color gray from previous text.n up until even with 1st line ->
line thin color gray from previous text.s down until even with 2nd line ->
X1: line thin color gray down 50% from 2mm below A.sw
X2: line thin color gray down 50% from 2mm below A.se
text "width" at (1/2<X1,X2>,6/8<X1.start,X1.end>)
line thin color gray from previous text.w left until even with X1 ->
line thin color gray from previous text.e right until even with X2 ->
~~~~

If a "`radius`" is specified, then the corners of the box are rounded using
arcs of the given radius.  The default radius for each new box is the value
of the "`boxrad`" variable which is initially 0.0.

~~~~ pikchr indent toggle
A: box thick rad 0.3*boxht
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
X3: line thin color gray right 70% from 2mm right of (A.e,A.s)
X4: line thin color gray right 70% from A.rad above start of X3
text "radius" at (6/8<X4.start,X4.end>,1/2<X3,X4>)
line thin color gray from (previous,X3) down 30% <-
line thin color gray from (previous text,X4) up 30% <-
~~~~

## Boundary points:

~~~~ pikchr indent toggle
A: box thin
dot ".c" above at A
dot ".n" above at A.n
dot " .ne" ljust above at A.ne
dot " .e" ljust at A.e
dot " .se" ljust below at A.se
dot ".s" below at A.s
dot ".sw " rjust below at A.sw
dot ".w " rjust at A.w
dot ".nw " rjust above at A.nw

A: box thin at 2.0*boxwid right of previous box rad 15px
dot ".c" above at A
dot ".n" above at A.n
dot " .ne" ljust above at A.ne
dot " .e" ljust at A.e
dot " .se" ljust below at A.se
dot ".s" below at A.s
dot ".sw " rjust below at A.sw
dot ".w " rjust at A.w
dot ".nw " rjust above at A.nw
~~~~
