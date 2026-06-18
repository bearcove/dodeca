# compass-angle

Because of the extensive historical use of compass heading names
like "north" and "se" (short for "south-east") in PIC and Pikchr,
it makes sense that angles should be specified according to compass
degrees.   North is 0&deg; and the angle increases clockwise so that
east is 90&deg;, south is 180&deg;, west is 270&deg; and 360&deg; is
back to north again.

~~~ pikchr
C: dot
arrow up from C; text " 0&deg;"
arrow right from C; text "  90&deg;" rjust
arrow down from C; text "180&deg;" below
arrow left from C; text "270&deg;  " ljust
~~~

Even though heading angles are specified in degrees, the arguments
to the built-in "sin()" and "cos()" functions are in radians.
