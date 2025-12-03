# new-property-value

When setting the value of certain numeric properties (like
"`width`" and "`radius`") you can specify either an absolute
amount, or a percentage relative to the current setting.

So, for example, you can say:

~~~~~
    box width 2.3cm
~~~~~

To create a box with a width of 2.3 centimeters - an absolute amount.
Or, if the current "`boxwid`" variable value is 2.0cm, then you could
do the same by saying:

~~~~~
    box width 115%
~~~~~
