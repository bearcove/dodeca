# statement

## Rules

  * *object-definition*
  * LABEL **:** *object-definition*
  * LABEL **:** *place*
  * *direction*
  * VARIABLE *assignment-op* *expr*
  * **define** VARIABLE CODEBLOCK
  * **print** *print-argument* (**,** *print-argument*)\*
  * **assert (** *expr* **==** *expr* **)**
  * **assert (** *position* **==** *position* **)**


## Labels

A label can be attached to either an *object* or a *place* so that
the object or place can be more easily referenced by subsequent statements.
Labels always begin with an upper-case ASCII character.

Labels do not have to be unique.  When there are two or more
labels with the same name, the later one takes precedence.
This allows a label to be effectively redefined.  New labels do not
come into existence until after the object or place to which they are
attached has been completely parsed and analyzed.  This allows labels
to be redefined in terms of themselves.  Consider an example:

~~~~~
/* 01 */        down
/* 02 */  Root: dot "First \"Root\"" above color red
/* 03 */        circle wid 50% at Root + (1.5cm, -1.5cm)
/* 04 */        arrow dashed from previous to Root chop
/* 05 */  Root: 3cm right of Root   // Move the location of Root 3cm right
/* 06 */        arrow from last circle to Root chop
/* 07 */        dot "Second \"Root\"" above color blue at Root
~~~~~

Line 05 redefines Root in terms of itself.
In the rendering below, you can see that the dashed arrow drawn before
Root was redefined goes to the original Root, but the solid arrow drawn
afterwards goes to the revised location for Root.

~~~~~ pikchr center toggle
/* 01 */        down
/* 02 */  Root: dot "First \"Root\"" above color red
/* 03 */        circle wid 50% at Root + (1.5cm, -1.5cm)
/* 04 */        arrow dashed from previous to Root chop
/* 05 */  Root: 3cm right of Root   // Move the location of Root 3cm right
/* 06 */        arrow from last circle to Root chop
/* 07 */        dot "Second \"Root\"" above color blue at Root
~~~~~

## Variables

Variable names begin with a lower-case ASCII letter or with "`$`"
or with "`@`".  The $- and @- variable names are a Pikchr extension
designed to help prevent collisions between variable names and the
(numerous) keywords in the Pikchr language.

Pikchr has built-in variables as follows:

>
| Variable Name | &nbsp;&nbsp; Initial Value &nbsp;&nbsp; |: Purpose         |
------------------------------------------------------------------------------
| arcrad        |: 0.250 :| Default arc radius                               |
| arrowhead     |: 2.000 :| *Not used by Pikchr*                             |
| arrowht       |: 0.080 :| Length of arrowheads                             |
| arrowwid      |: 0.060 :| Width of arrowheads                              |
| boxht         |: 0.500 :| Default height of "box" objects                  |
| boxrad        |: 0.000 :| Default corner radius for "box" objects          |
| boxwid        |: 0.750 :| Default width for "box" objects                  |
| charht        |: 0.140 :| Average height of a character                    |
| charwid       |: 0.080 :| Average width of a character                     |
| circlerad     |: 0.250 :| Default radius for "circle" objects              |
| color         |: 0.000 :| Default foreground color                         |
| cylht         |: 0.500 :| Default height for "cylinder" objects            |
| cylrad        |: 0.075 :| Default minor axis for ellipses in a cylinder    |
| cylwid        |: 0.750 :| Default width of a "cylinder" object             |
| dashwid       |: 0.050 :| Default width of dashes in dashed lines          |
| dotrad        |: 0.015 :| Default radius for a "dot" object                |
| ellipseht     |: 0.500 :| Default height for "ellipse" objects             |
| ellipsewid    |: 0.750 :| Default width for "ellipse" objects              |
| fileht        |: 0.750 :| Default height for "file" objects                |
| filerad       |: 0.150 :| Default corner fold length for "file" objects    |
| filewid       |: 0.500 :| Default width for "file" objects                 |
| fill          |: -1.00 :| Default fill color.  Negative means "none"       |
| lineht        |: 0.500 :| Default length for lines drawn up or down        |
| linewid       |: 0.500 :| Default length for lines drawn left or right     |
| movewid       |: 0.500 :| Default distance traversed by a "move"           |
| ovalht        |: 0.500 :| Default height of an "oval" object               |
| ovalwid       |: 1.000 :| Default width of an "oval" object                |
| scale         |: 1.000 :| Scale factor for drawing.  Larger is bigger.     |
| textht        |: 0.500 :| *Not used by Pikchr*                             |
| textwid       |: 0.750 :| *Not used by Pikchr*                             |
| thickness     |: 0.015 :| Default line thickness for all objects           |

In addition to the above, Pikchr recognizes the following variables
which are not initially defined, but if they are defined by the script
have special properties:

>
| Variable Name&nbsp;&nbsp;&nbsp;&nbsp; |: Purpose                           |
------------------------------------------------------------------------------
| bottommargin  | Extra border space added to the bottom of the diagram      |
| fgcolor       | Use this foreground color in place of black                |
| fontscale     | Scale factor applied to the font size of text              |
| layer         | The default layer for all subsequent objects               |
| leftmargin    | Extra border space added to the left of the diagram        |
| margin        | Extra border space added to all four sides of the diagram  |
| rightmargin   | Extra border space added to the right side of the diagram  |
| topmargin     | Extra border space added to the top side of the diagram    |


The "VARIABLE *assignment-op* *expr*" syntax is able to modify the value
of built-in variables, or create new variables.  In legacy-PIC, the only
*assignment-op* was "`=`".  Pikchr adds "`+=`", "`-=`", "`*=`", and
"`/=`" to make it easier to scale existing variables up or down.

### Conflicts between variable names and keywords

Some of the built-in variables have names that conflict with keywords:

  *  color
  *  fill
  *  thickness

To access such variables as part of an expression, simply put them inside
of parentheses.  For example, to set the thickness of a box to be twice
the default thickness:

~~~ pikchr center toggle source
   box "Normal"
   move
   box "Double" "Thick" thickness 2*(thickness)
~~~

## Define

The "`define`" statement creates a [macro](./macro.md)
that can then be called in subsequent text.

## Print

The "`print`" statement prints the strings and the values of the expressions
in its argument into the generated output in front of the 
"`<svg>`" element for the diagram.  This facility is intended for testing
and debugging purposes.  There is no known practical use for "`print`" in
a production Pikchr script.

The following Pikchr script demonstrates the effect of "print".
Click to toggle between the script and its rendering.

~~~ pikchr toggle source indent
   oval "Hello, World!" fit
   print "Oval at: ",previous.x, ",", previous.y
   line
   oval "2nd oval" fit
   print "2nd oval at: ",previous.x, ",", previous.y
~~~

## Assert

The "`assert`" statement is intended for testing and debugging of Pikchr
scripts.  An assert() is a no-op if the equality comparison in its
argument is true.  But it raises an error if the condition is false.

Consider this script:

~~~
   oval "Hello, World!" fit
   assert( last oval.w == last oval.e ); # <-- should fail
~~~

And its rendering:

~~~ pikchr
   oval "Hello, World!" fit
   assert( last oval.w == last oval.e ); # <-- should fail
~~~
