# Pikchr Grammar

This file describes the grammar of the input files to Pikchr.  Keywords
and operators are shown in **bold**.  Non-terminal symbols are shown
in *italic*.  Special token classes are shown in ALL-CAPS.  A grammar
symbol followed by "*" means zero-or-more.  A grammar symbol
followed by "?" means zero-or-one.  Parentheses are used for grouping.
Two grammar symbols within "(..|..)" means one or the other.
 Marks of the form
"[&#9654;info](./grammar.md)" are links to more information and are
not part of the grammar.

The following special token classes are recognized:

  *  NEWLINE  &rarr;  A un-escaped newline character, U+000A.
     A backslash followed by zero or more whitespace characters
     and then a U+000A character is interpreted as ordinary whitespace,
     not as a NEWLINE.

  *  LABEL  &rarr;  An object or place label starting with an
     upper-case ASCII letter and continuing with zero or more
     ASCII letters, digits, and/or underscores.  A LABEL always starts
     with an upper-case letter.

  *  VARIABLE  &rarr;  A variable name consisting of a lower-case
     ASCII letter or "$" or "@" and followed by zero or more
     ASCII letters, digits, and/or underscores.  VARIABLEs may
     contain upper-case letters, but they never begin with an upper-case.
     In this way, VARIABLEs are distinct from LABELs.

  *  NUMBER  &rarr;  A numeric literal.  The value can be a decimal
     integer, a floating point value, or a hexadecimal literal
     starting with "0x".  Decimal and floating point values can
     optionally be followed by a two-character unit designator that is
     one of:  "in", "cm", "px", "pt", "pc", or "mm".  There can be
     no whitespace in between the numeric portion of the constant and
     the unit.

  *  ORDINAL  &rarr;  A non-zero integer literal followed by one of the
     suffixes "st", "nd", "rd", or "th".  Examples: "1st", "2nd",
    "3rd", "4th", "5th", and so forth.   As a special case, "first"
     is accepted as an alternative spelling of "1st".

  *  STRING  &rarr;  A string literal that begins and ends with
     double-quotes (U+0022).  Within the string literal, a double-quote
     character can be escaped using backslash (U+005c).  A backslash
     can also be used to escape a backslash.  No other escape sequences
     are recognized and standalone backslashes are elided from the output.
     Newlines are permitted in strings.

  *  COLORNAME &rarr;  One of the 140 official HTML color names, in
     any mixture of upper and lower cases.  The value of a COLORNAME is
     an integer which is the 24-bit RGB value of that color.  Two
     additional color names of "None" and "Off" are also recognized and
     have a value of -1.

  *  CODEBLOCK &rarr;   All tokens contained within nested {...}.  This
     is only used as the body of a "define" statement.

There are many non-terminals in the grammar, but a few are more important.
If you are new to the Pikchr language, begin by focusing on these
six:

  *  *statement* &rarr;  A Pikchr script is just a list of statements.

  *  *attribute* &rarr;  Each graphic object is configured with zero or
     more attributes.

  *  *object* &rarr;  A reference to a prior graphic object, or `this` to
     refer to the current object.

  *  *place* &rarr;  A specific point associated with an *object*.

  *  *position* &rarr;  Any (2-D) point in space.  An (x,y) pair.

  *  *expr* &rarr;  A scalar expression.


A complete input file to Pikchr consists of a single *statement-list*.

## *statement-list*: [&#9654;info](./stmtlist.md)

  * *statement*?
  * *statement-list* NEWLINE *statement*?
  * *statement-list* **;** *statement*?

## *statement*:  [&#9654;info](./stmt.md)
  * *object-definition*
  * LABEL **:** *object-definition*
  * LABEL **:** *place*
  * *direction*
  * VARIABLE *assignment-op* *expr*
  * **define** VARIABLE CODEBLOCK     [&#9654;info](./macro.md)
  * **print** *print-argument* (**,** *print-argument*)\*
  * **assert (** *expr* **==** *expr* **)**
  * **assert (** *position* **==** *position* **)**


## *direction*:
  * **right**
  * **down**
  * **left**
  * **up**

## *assignment-op*:
  * **=**
  * **+=**
  * **-=**
  * **\*=**
  * **/=**

## *print-argument*:
  * *expr*
  * STRING

## *object-definition*:
  * *object-class* *attribute*\*
  * STRING *text-attribute*\* *attribute*\*
  * **[** *statement-list* **]** *attribute*\*

## *object-class*:
  * **arc**
  * **arrow**
  * **box**          [&#9654;info](./boxobj.md)
  * **circle**       [&#9654;info](./circleobj.md)
  * **cylinder**     [&#9654;info](./cylinderobj.md)
  * **diamond**      [&#9654;info](./diamondobj.md)
  * **dot**
  * **ellipse**      [&#9654;info](./ellipseobj.md)
  * **file**         [&#9654;info](./fileobj.md)
  * **line**
  * **move**
  * **oval**         [&#9654;info](./ovalobj.md)
  * **spline**
  * **text**

## *attribute*:
  * *path-attribute*              [&#9654;info](./pathattr.md)
  * *location-attribute*          [&#9654;info](./locattr.md)
  * STRING *text-attribute*\*     [&#9654;info](./annotate.md)
  * **same**
  * **same as** *object*
  * *numeric-property* *new-property-value*
  * **dashed** *expr*?
  * **dotted** *expr*?
  * **color** *color-expr*
  * **fill** *color-expr*
  * **behind** *object*      [&#9654;info](./behind.md)
  * **cw**
  * **ccw**
  * **&lt;-**                [&#9654;info](./arrowdir.md)
  * **-&gt;**                [&#9654;info](./arrowdir.md)
  * **&lt;-&gt;**            [&#9654;info](./arrowdir.md)
  * **invis**|**invisible**  [&#9654;info](./invis.md)
  * **thick**                [&#9654;info](./thickthin.md)
  * **thin**                 [&#9654;info](./thickthin.md)
  * **solid**                [&#9654;info](./thickthin.md)
  * **chop**                 [&#9654;info](./chop.md)
  * **fit**                  [&#9654;info](./fit.md)

## *color-expr*: [&#9654;info](./colorexpr.md)
  * *expr*

## *new-property-value*:  [&#9654;info](./newpropval.md)
  * *expr*
  * *expr* **%**

## *numeric-property*:  [&#9654;info](./numprop.md)
  * **diameter**
  * **ht**
  * **height**
  * **rad**
  * **radius**
  * **thickness**
  * **width**
  * **wid**

## *text-attribute*:  [&#9654;info](./textattr.md)
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

## *path-attribute*:   [&#9654;info](./pathattr.md)
  * **from** *position*
  * **then**? **to** *position*
  * **then**? **go**? *direction* *line-length*?
  * **then**? **go**? *direction* **until**? **even with** *position*
  * (**then**|**go**) *line-length*? **heading** *compass-angle*
  * (**then**|**go**) *line-length*? *compass-direction*
  * **close**

## *line-length*:  [&#9654;info](./linelen.md)

  * *expr*
  * *expr* **%**

## *compass-angle*:   [&#9654;info](./compassangle.md)

  * *expr*

## *compass-direction*:
  * **n**
  * **north**
  * **ne**
  * **e**
  * **east**
  * **se**
  * **s**
  * **south**
  * **sw**
  * **w**
  * **west**
  * **nw**

## *location-attribute*: [&#9654;info](./locattr.md)
  * **at** *position*
  * **with** *edgename* **at** *position*
  * **with** *dot-edgename* **at** *position*

## *position*:  [&#9654;info](./position.md)

  *  *expr* **,** *expr*
  *  *place*
  *  *place* **+** *expr* **,** *expr*
  *  *place* **-** *expr* **,** *expr*
  *  *place* **+ (** *expr* **,** *expr* **)**
  *  *place* **- (** *expr* **,** *expr* **)**
  *  **(** *position* **,** *position* **)**
  *  **(** *position* **)**
  *  *fraction* **of the way between** *position* **and** *position*
  *  *fraction* **way between** *position* **and** *position*
  *  *fraction* **between** *position* **and** *position*
  *  *fraction* **<** *position* **,** *position* **>**
  *  *distance* *which-way-from* *position*

## *fraction*:
  *  *expr*

## *distance*
  *  *expr*

## *which-way-from*:

  *  **above**
  *  **below**
  *  **right of**
  *  **left of**
  *  **n of**
  *  **north of**
  *  **ne of**
  *  **e of**
  *  **east of**
  *  **se of**
  *  **s of**
  *  **south of**
  *  **sw of**
  *  **w of**
  *  **west of**
  *  **nw of**
  *  **heading** *compass-angle* **from**

## *place*:      [&#9654;info](./place.md)

  *  *object*
  *  *object* *dot-edgename*
  *  *edgename* **of** *object*
  *  ORDINAL **vertex of** *object*

## *object*:

  *  LABEL
  *  *object* **.** LABEL
  *  *nth-object* **of**|**in** *object*

## *nth-object*:

  *  ORDINAL *object-class*
  *  ORDINAL **last** *object-class*
  *  ORDINAL **previous** *object-class*
  *  **last** *object-class*
  *  **previous** *object-class*
  *  **last**
  *  **previous**
  *  ORDINAL **[]**
  *  ORDINAL **last []**
  *  ORDINAL **previous []**
  *  **last []**
  *  **previous []**

## *dot-edgename*:
  * **.n**
  * **.north**
  * **.t**
  * **.top**
  * **.ne**
  * **.e**
  * **.east**
  * **.right**
  * **.se**
  * **.s**
  * **.south**
  * **.bot**
  * **.bottom**
  * **.sw**
  * **.w**
  * **.west**
  * **.left**
  * **.nw**
  * **.c**
  * **.center**
  * **.start**
  * **.end**

## *edgename*:
  * **n**
  * **north**
  * **ne**
  * **e**
  * **east**
  * **se**
  * **s**
  * **south**
  * **sw**
  * **w**
  * **west**
  * **nw**
  * **t**
  * **top**
  * **bot**
  * **bottom**
  * **left**
  * **right**
  * **c**
  * **center**
  * **start**
  * **end**


## *expr*:

  *  NUMBER
  *  VARIABLE
  *  COLORNAME
  *  *place* **.x**
  *  *place* **.y**
  *  *object* *dot-property*
  *  **(** *expr* **)**
  *  *expr* **+** *expr*
  *  *expr* **-** *expr*
  *  *expr* **\*** *expr*
  *  *expr* **/** *expr*
  *  **-** *expr*
  *  **+** *expr*
  *  **abs (** *expr* **)**
  *  **cos (** *expr* **)**
  *  **dist (** *position* **,** *position* **)**
  *  **int (** *expr* **)**
  *  **max (** *expr* **,** *expr* **)**
  *  **min (** *expr* **,** *expr* **)**
  *  **sin (** *expr* **)**
  *  **sqrt (** *expr* **)**

## *dot-property*:

  * **.color**
  * **.dashed**
  * **.diameter**
  * **.dotted**
  * **.fill**
  * **.ht**
  * **.height**
  * **.rad**
  * **.radius**
  * **.thickness**
  * **.wid**
  * **.width**
