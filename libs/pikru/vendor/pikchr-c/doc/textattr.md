# text-attribute

Any string literal that is intended to be displayed on the
diagram can be followed by zero or more of the following
keywords, in any order:

  * **above**
  * **aligned**
  * **below**
  * **big**
  * **bold**
  * **mono**
  * **monospace**
  * **center**
  * **italic**
  * **ljust**
  * **rjust**
  * **small**

## Attributes "above" and "below"

The "`above`" and "`below`" keywords control the location of the
text above or below the center point of the object with which
the text is associated.  If there is just one text on the object
and the "`above`" and "`below`" keywords are omitted, the text is
placed directly over the center of the object.  This causes
the text to appear in the middle of lines:

~~~~ pikchr indent
  line "on the line" wid 150%
~~~~

So if there is just a single text label on a line, you probably
want to include either the "`above`" or "`below`" keyword.

~~~~ pikchr indent
  line "above" above; move; line "below" below
~~~~

If there are two texts on the object, they straddle the center point
above and below, even without the use of the "`above`" and "`below`"
keywords:

~~~~ pikchr indent
  line wid 300% "text without \"above\"" "text without \"below\""
~~~~

The "`above`" and "`below`" attributes do not stack or accumulate.
Each "`above`" or "`below`" overrides any previous "`above`" or "`below`"
for the same text.

If there are multiple texts and all are marked "`above`" or "`below`", then
all are placed above or below the center point, in order of appearance.

~~~~ pikchr indent
  line width 200% "first above" above "second above" above
  move
  line same "first below" below "second below" below
~~~~

## Attributes "ljust" and "rjust"

As the "`above`" and "`below`" keywords control up and down positioning of
the text, so the "`ljust`" and "`rjust`" keywords control left and right
positioning.

For a line, the "`ljust`" means that the left side of the text is flush
against the center point of the line.  And "`rjust`" means that the right
side of the text is flush against the center point of the line.
(In the following diagram, the red dot is at the center of the line.)

~~~~ pikchr indent
   line wid 200% "ljust" ljust above "rjust" rjust below
   dot color red at previous.c
~~~~

For a block object, "`ljust`" shifts the text to be left justified
against the left edge of the block (with a small margin) and
"`rjust`" puts the text against the right side of the object (with
the same margin).

~~~~ pikchr indent
   box "ljust" ljust "longer line" ljust "even longer line" ljust fit
   move
   box "rjust" rjust "longer line" rjust "even longer line" rjust fit
~~~~

The behavior of "`ljust`" and "`rjust`" for block objects in Pikchr differs
from legacy PIC.
In PIC, text is always justified around the center point, as in lines.
But this means there is no easy way to left justify multiple lines of
text within a "box" or "file", and so the behavior was changed for
Pikchr.

Pikchr allows two texts to fill the same vertical slot if one is
"`ljust`" and the other is "`rjust`".

~~~~ pikchr indent
  box wid 300% \
     "above-ljust" above ljust \
     "above-rjust" above rjust \
     "centered" center \
     "below-ljust" below ljust \
     "below-rjust" below rjust
~~~~

## Attribute "center"

The "`center`" attribute cancels all prior "`above`", "`below`",
"`ljust`", and "`rjust`" attributes for the current text.

## Attributes "bold" and "italic"

The "`bold`" and "`italic`" attributes cause the text to use a bold or
an italic font.  Fonts can be both bold and italic at the same time.

~~~~ pikchr indent
  box "bold" bold "italic" italic "bold-italic" bold italic fit
~~~~

### Monospace Font Family <a id="font-family"></a>

The "`mono`" or "`monospace`" attributes cause the text object to use a
monospace font.

~~~~ pikchr indent toggle
  box "monospace" monospace fit
~~~~

## Attribute "aligned"

The "`aligned`" attribute causes text associated with a straight line
to be rotated to align with that line.

~~~~ pikchr indent
  arrow go 150% heading 30 "aligned" aligned above
  move to 1cm east of previous.end
  arrow go 150% heading 170 "aligned" aligned above
  move to 1cm east of previous.end
  arrow go 150% north "aligned" aligned above
~~~~

To display rotated text not associated with a line attach the
text to a line that is marked "`invisible`"

~~~~ pikchr indent
  box ht 200% wid 50%
  line invis from previous.s to previous.n "rotated text" aligned
~~~~

## Attributes "big" and "small"

The "`big`" and "`small`" attributes cause the text to be a little larger
or a little smaller, respectively.  Two "`big`" attributes cause the
text to be larger still, as do two "`small`" attributes.  But the text
size does not increase or decrease beyond two "`big`" or "`small`" keywords.

~~~~ pikchr indent
  box "small small" small small "small" small \
    "(normal)" italic \
    "big" big "big big" big big ht 200%
~~~~

A "`big`" keyword cancels any prior "`small`" keywords on the same text,
and a "`small`" keyword cancels any prior "`big`" keywords.
