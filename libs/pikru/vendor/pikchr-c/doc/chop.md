# The "chop" Attribute

Line objects may have a single "`chop`" attribute.  When the chop
attribute is present, and if the line starts or ends at the center
of a block object, then that start or end is automatically moved to
the edge of the object.  For example:

~~~~ pikchr toggle
file "A"
cylinder "B" at 5cm heading 125 from A
arrow <-> from A to B "from A to B" aligned above color red
arrow <-> from A to B chop "from A to B chop" aligned below color blue
~~~~

In the example, both of the arrows use "`from A to B`"  The difference
is that the blue line adds the "`chop`" keyword whereas the red line
does not.

The chop feature only works if one or both ends of the line land on
the center of a block object.  If neither end of a line is on the
center of a block object, then the "`chop`" attribute is a no-op

## Different From Legacy PIC

The chop attribute in Pikchr differs from the chop attribute in legacy PIC.
In PIC, the "`chop`" keyword can be followed by a distance and can appear
twice.  The chop keyword causes the line to be shortened by the amount
specified, or by `circlerad` if no distance is given.  The legacy "chop"
works okay if you are drawing lines between circles, but it mostly pointless
for lines between all other kinds of objects.  The enhanced "chop" in
Pikchr is intended to make the feature helpful on a wider variety of
diagrams.
