# Macros

A macro is created using a "`define`" statement:

~~~ pikchr toggle
$r = 0.2in
linerad = 0.75*$r
linewid = 0.25

# Start and end blocks
#
box "define-statement" bold fit
line down 50% from last box.sw
START: dot rad 250% color black
X0: last.e
move right 3.2in
END: box wid 5% ht 25% fill black
X9: last.w

# The main rule
#
arrow from X0 right 2*linerad+arrowht
oval "\"define\"" fit
arrow
oval "MACRONAME" fit
arrow
oval "{...}" fit
line right to X9
~~~

A define statement consists of the keyword "`define`" followed by
an identifier that is the name of the macro and then the body of
the macro contained within (possibly nested) curly braces.

After a macro is defined, the body of the macro is substituted in
place of any subsequent occurrence of the identifier that is the
macro name.  The macro name can occur anywhere.  The substitution
is performed by the lexical analyzer, before tokens are identified
and sent into the parser.  Note this distinction:  The "`define`"
statement used to create a new macro is recognized by the parser,
but the expansion of the macro is subsequent text happens in the
lexical analyzer.

## Parameters

The invocation of a macro can be followed immediately by a
parenthesized list of parameters.  The open-parenthesis must immediately
follow the macro name with no intervening whitespace.  Parameters are
comma-separated.  There can be at most 9 parameters.

When parameters are present, they are substituted in the macro body
in place of "`$1`", "`$2`", ..., "`$9`" in the macro body.  If
"$N" (for N between 1 and 9) occurs in the macro body but there are
fewer than N parameters, then the "$N" is omitted.

## Nested Macros

Macros can be nested up to a maximum depth that is determined at
compile-time.  (The current limit is 10.)

Arguments to nested macros can be arbitrary text, or a single "$N"
parameter, but not both.

## Macros cannot be undefined or redefined

Once created, a macro cannot be redefined.  If you attempt to redefine
a macro by providing a second "`define`" statement with the same macro
name, the macro name will be replaced by the previous macro body definition
during lexical analysis, likely resulting in a syntax error.
