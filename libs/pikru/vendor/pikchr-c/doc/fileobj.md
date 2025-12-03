# File objects

A "file" is a stylized image of a piece of paper with the upper right
corner folded over.  Similar images are commonly used to represent "files".
The shape of a file object is defined by its width, height, and radius.
The radius is the height and width of the folded corner.  The default values
for height, radius, and width are controlled by variables
"`fileht`", "`filerad`", and "`filewid`".

~~~~ pikchr indent toggle
A: file thick rad 100%
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
X3: line thin color gray right 70% from 2mm right of (A.e,A.n)
X4: line thin color gray right 70% from A.rad below start of X3
text "radius" at (6/8<X4.start,X4.end>,1/2<X3,X4>)
line thin color gray from (previous,X4) down 30% <-
line thin color gray from (previous text,X3) up 30% <-
~~~~

## Boundary points:

~~~~ pikchr indent toggle
A: file thin rad 80%
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
