# statement-list

A complete Pikchr source document consists of a list of zero or more
statements. Individual statements within the list are separated from
each other by semicolons ("`;`") and/or newlines.  Surplus semicolons
and newlines are ignored.  A zero-length string, or a string consisting
of only semicolons and newlines, is a valid Pikchr document.

The *statement-list* is also a subpart of the syntax for 
the `[]`-collection object.

## Rules

  * *statement* 
  * *statement-list* NEWLINE *statement*
  * *statement-list* **;** *statement*

## Bubble Chart

~~~~~ pikchr indent
$r = 0.2in
linerad = 0.75*$r
linewid = 0.25

# Start and end blocks
#
box "statement-list" bold fit
line down 75% from last box.sw
dot rad 250% color black
X0: last.e + (0.3,0)
arrow from last dot to X0
move right 2in
box wid 5% ht 25% fill black
X9: last.w - (0.3,0)
arrow from X9 to last box.w


# The main rule that goes straight through from start to finish
#
box "statement" italic fit at 0.5<X0,X9>
arrow to X9
arrow from X0 to last box.w

# The by-pass line
#
arrow right $r from X0 then up $r \
  then right until even with 1/2 way between X0 and X9
line right until even with X9 - ($r,0) \
  then down until even with X9 then right $r

# The Loop-back rule
#
oval "\"&#92;n\"" fit at $r*1.2 below 1/2 way between X0 and X9
line right $r from X9-($r/2,0) then down until even with last oval \
   then to last oval.e ->
line from last oval.w left until even with X0-($r,0) \
   then up until even with X0 then right $r
oval "\";\"" fit at $r*1.2 below last oval
line from 2*$r right of 2nd last oval.e left $r \
   then down until even with last oval \
   then to last oval.e ->
line from last oval.w left $r then up until even with 2nd last oval \
   then left 2*$r ->
~~~~~

## Whitespace

Whitespace other than a newline is ignored.  If a backslash is followed
by one or more whitespace characters ending in a newline, then the
backslash and all of the spaces that follow, including the newline,
are considered whitespace.  Thus, a backslash at the end of a line
causes a statement to continue onto the next line.

## Comments

Three comment formats are supported:

   *  The "`#`" character and all characters that follow up to but not
      including the next newline character.  (Bourne-shell style comments.)

   *  Two forward slashes ("`//`") and all characters that follow up to
      but not including the next newline character.  (C++ style comments.)

   *  The sequence "`/*`" and all characters that follow up to and including
      the next "`*/`".  (C style comments.)

The first form (#-comments) is the only form supported by legacy-PIC.
The C++ and C style commenting is new to Pikchr.

For #-comments and //-comments, the newline that follows is not part of the
comment.  Hence that newline will terminate the current statement.  There
is no way to escape the newline at the end of a #- or //-comment.  If you
need a comment at the end of a line but want to continue the statement on
the next line, you must use `/*..*/` style comments.
