# location-attribute

A *location-attribute* is an attribute used to assign a location to
a block object (box, circle, cylinder, diamond, dot, ellipse, file, oval, or text).
If a *location-attribute* appears on a line object (arc, arrow, line, move,
or spline) an error is issued and processing stops.

There are three forms:

  *  **at** *position*
  *  **with** *edgename* **at** *position*
  *  **with** *dot-edgename* **at** *position*

The second and third forms are equivalent and only differ in
the "." that comes before the edge name.  PIC does not recognize
the second form, only the first and third.

If the "`with`" clause is omitted, then "`with center`" or
(equivalently) "`with .c`" is assumed.

This attribute causes the block object to be positioned so that
its *edgename* corner is at *position*.

If a block object omits this attribute, then a default location-attribute
is used as follows:

  *  **with .begin at previous.end**
  *  **with .c at (0,0)**

The first default form is what is normally used.  The second default
form is only used if there is no "previous" object.
