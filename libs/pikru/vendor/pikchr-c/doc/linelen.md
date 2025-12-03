# line-length

A *line-length* is an expression that specifies how long to draw a
line segment.  The value can be either absolute (ex: "`1.2cm`", 
"`.5in`", "`0.5*circlerad`", and so forth) or it can be a percentage value
(ex: "`85%`").

  * *expr*
  * *expr* **%**

If the percentage value is used, the basis is usually the
value stored in the "`linewid`" variable.  However, for a case of
either:

  * **up** *expr* **%**
  * **down** *expr* **%**

…then the percentage refers to the current "`lineht`" value instead.  The
"`linewid`" value is always used for headings even if the heading
is "`0`" or "`180`" or "`north`" or "`south`".

In most cases it does not matter whether "`linewid`" or "`lineht`"
gets used for the percentage basis since both variables have the
same initial default of 0.5in.
