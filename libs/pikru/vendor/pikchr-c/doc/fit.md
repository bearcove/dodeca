# The "fit" attribute

The "`fit`" attribute causes an object to automatically adjust its
"`width`", "`height`", and/or "`radius`" so that it will enclose its
text annotations with a reasonable margin.

~~~ pikchr toggle
box "with" "\"fit\"" fit
move
box "without" "\"fit\""
~~~

The "`fit`" attribute only works with text annotations that occur
earlier in the object definition.  In other words, the "`fit`" keyword
should come after all text annotations have been defined.

## Pikchr guesses at the size of text

Pikchr does not have access to the SVG rendering engine.  Therefore,
it cannot know the precise dimensions of text annotations.  It has to
guess.  Usually Pikchr does a reasonable job, but sometimes it can be
a little off, especially with unusual characters.  If "`fit`" causes the
object to be too narrow, you can try adding spaces at the beginning and
end of the longest text annotation.  You can also adjust the width
and height by a percentage after running "`fit`":

   *  `width 110%`
   *  `height 90%`
   *  `radius 120%`

And so forth.  Substitute percentage increases and decreases, as
appropriate, to make the text fit like you want.

## Auto-fit

If at the end of an objection definition the requested width or height of the
object is less then or equal to zero, then that dimension is adjusted
upwards to enclose the text annotations.  Thus, by setting variables
like:

~~~
    boxwid = 0
    boxht = 0
~~~

You can cause all boxes to scale to enclose their text annotations.
(Caution:  boxes without any text annotations go to zero height and width
and thus disappear when auto-fit is enabled.)
