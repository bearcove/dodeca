# path-attribute

A *path-attribute* is used to provide the origin and direction of a line
object, that being an arc, arrow, line, move, or spline.  It is an error
to use a *path-attribute* on a block object, that being a box, circle,
cylinder, diamond, dot, ellipse, file, oval, or text.

There are seven forms:

  *  **from** *position*
  *  **then**? **to** *position*
  *  **then**? **go**? *direction* *distance*?
  *  **then**? **go**? *direction* **until**? **even with** *place*
  *  (**then**|**go**) *distance*? **heading** *compass-angle*
  *  (**then**|**go**) *distance*? *compass-direction*
  *  **close**

The "`from`" attribute is used to assign the starting location
of the line object (its ".start" value).  The other six forms
(collectively called "to" forms) assign
intermediate vertexes or the end point (.end).   If the "`from`"
is omitted, then "`from previous.end`" is assumed, or if there
is no previous object, "`from (0,0)`".   If no "to" forms are
provided then a single movement in the current layout direction
by either the "linewid" or "lineht" (depending on layout direction)
is used.

The "from" can occur
either before or after the various "to" subclauses.  That does not
matter.  But the order is important for the various "to" subclauses.

If there are two consecutive *direction* clauses (*direction* is
always one of "`up`", "`down`", "`left`", or "`right`") then
the two will be combined to specify a single line segment.
Hence, the following are equivalent:


  *  ... **right 4cm up 3cm** ...
  *  ... **go 5cm heading 53.13010235** ...

~~~ pikchr
leftmargin = 1cm
A1: arrow thick right 4cm up 3cm
dot at A1.start
X1: line thin color gray from (0,-3mm) down 0.4cm
X2: line same from (4cm,-3mm) down 0.4cm
arrow thin color gray from X1 to X2 "4cm" above
X3: line same from (4cm+3mm,0) right 0.4cm
X4: line same from (4cm+3mm,3cm) right .4cm
arrow thin color gray from X3 to X4 "3cm" aligned above
X5: line same from A1.start go 4mm heading 90+53.13010235
X6: line same from A1.end go 4mm heading 90+53.13010235
arrow thin color gray from X5 to X6 "5cm" below aligned
line same from (0,1cm) up 1cm
spline -> from 1.5cm heading 0 from A1.start \
   to 1.5cm heading 10 from A1.start \
   to 1.5cm heading 20 from A1.start \
   to 1.5cm heading 30 from A1.start \
   to 1.5cm heading 40 from A1.start \
   to 1.5cm heading 53.13 from A1.start \
   thin color gray "53.13&deg;" aligned center small
~~~

If two separate movements are desired, one 4cm right and another 3cm up,
then the "right" and "up" subphrases must be separated by the "`then`" keyword:

  *  ... **right 4cm then up 3cm** ...

~~~ pikchr
leftmargin = 1cm
A1: arrow thick right 4cm then up 3cm
dot at A1.start
X1: line thin color gray from (0,-3mm) down 0.4cm
X2: line same from (4cm,-3mm) down 0.4cm
arrow thin color gray from X1 to X2 "4cm" above
X3: line same from (4cm+3mm,0) right 0.4cm
X4: line same from (4cm+3mm,3cm) right .4cm
arrow thin color gray from X3 to X4 "3cm" aligned above
~~~

## The "`until even with`" subclause

The "until even with" clause is a Pikchr extension (it does not exist
in PIC) that makes it easier to specify paths that follow a
"Manhattan geometry" (lines are axis-aligned) or that negotiate around
obstacles.  The phrase:

>  go *direction* until even with *position*

Means to continue the line in the specified *direction* until the
coordinate being changed matches the corresponding coordinate in
*position*. If the line is going up or down, then it continues until
the Y coordinate matches the Y coordinate of *position*.  If the line
is going left or right, then it continues until
the X coordinate matches the X coordinate of *position*.

For example, suppose in the diagram below that we want to draw an arrow 
that begins on Origin.s and ends on Destination.s but goes around
the Obstacle oval, clearing it by at least one centimeter.

~~~ pikchr toggle
box "Origin"
Obstacle: oval ht 300% wid 30% with .n at linewid right of Origin.ne;
box "Destination" with .nw at linewid right of Obstacle.n
line invis from 1st oval.s to 1st oval.n "Obstacle" aligned
~~~

The arrow might look like this:

~~~
   arrow from Origin.s \
      down until even with 1cm below Obstacle.s \
      then right until even with Destination.s \
      then to Destination.s
~~~

And we have (annotations added):

~~~ pikchr toggle
box "Origin"
Obstacle: oval ht 300% wid 30% with .n at linewid right of Origin.ne;
box "Destination" with .nw at linewid right of Obstacle.n
line invis from 1st oval.s to 1st oval.n "Obstacle" aligned
X: \
   arrow from Origin.s \
      down until even with 1cm below Obstacle.s \
      then right until even with Destination.s \
      then to Destination.s

line invis color gray from X.start to 2nd vertex of X \
    "down until even with" aligned small \
    "1cm below Obstacle.s" aligned small
line invis color gray from 2nd vertex of X to 3rd vertex of X \
    "right until even with Destination.s" aligned small above
line invis color gray from 3rd vertex of X to 4th vertex of X \
    "to Destination.s" aligned small above

# Evidence that the alternative arrow is equivalent:
assert( 2nd vertex of X == (Origin.s, 1cm below Obstacle.s) )
assert( 3rd vertex of X == (Destination.s, 1cm below Obstacle.s) )
~~~

The "**(** *position* **,** *position* **)**" syntax can be used
in a similar way.  The "**(** *position* **,** *position* **)**"
syntax means a point whose X coordinate is taken from the first
position and whose Y coordinate is taken from the second position.
So the line around the obstacle could have been written like this:

~~~ 
   arrow from Origin.s \
     to (Origin.s, 1cm below Obstacle.s) \
     then to (Destination.s, 1cm below Obstacle.s) \
     then to Destination.s
~~~

However, we believe the "`until even with`" notation is easier.

## The "`close`" subclause

The "`close`" attribute closes a multi-segment path so that it
forms a polygon.  When "`close`" is used, the "`.end`" point of the
object is no longer the last vertex in the path but is instead
one of "`.e`", "`.s`", "`.w`", or "`.n`" according to the current
layout direction, as it would be for a block object.

The following diagram illustrates this behavior.  The "`.end`" of
each line is tagged with a red dot.  The line that uses "`close`"
has its end at the "`.e`" point of the bounding box since the
layout direction is "right".  The line without "`close`" has its
"`.end`" at the last vertex of the line.

~~~ pikchr toggle
line right 2cm then down .5cm then up 1cm right 1cm \
   then up 1cm left 1cm then down .5cm then left 2cm \
   close "with 'close'"
dot color red at last line.end

move to 2.5cm south of last line.start
line right 2cm then down .5cm then up 1cm right 1cm \
   then up 1cm left 1cm then down .5cm then left 2cm \
   then down 1cm "without 'close'"
dot color red at last line.end
~~~
