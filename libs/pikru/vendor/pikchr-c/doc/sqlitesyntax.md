# How Pikchr Generates the SQLite Syntax Diagrams

Beginning with SQLite version 3.34.0, the graphical
[syntax diagrams][1] in the SQLite documentation are SVGs generated
using Pikchr.  In prior versions of SQLite, the diagrams were GIF images generated
using a fiddly processing chain based on Tcl/Tk, Ghostscript, and ImageMagick.

[1]: https://sqlite.org/syntaxdiagrams.html

## Advantages to the New Approach

  *  The SVG syntax diagrams are embedded in the text of the
     HTML documentation pages rather than separately-loaded
     GIF images.  That speeds page loads due to fewer HTTP round-trips to the server to fetch
     resources, and it makes saved copies of the resulting HTML documentation pages
     self-contained, simplifying later off-line viewing.

  *  SVG is resolution independent, so that diagrams look better on
     high-DPI devices such as smartphones and tablets.

  *  The Pikchr source text promises to be easier to maintain than
     the prior Tcl/Tk+Ghostscript+ImageMagick toolchain.

  *  The scripts used to convert the documentation source text into
     display-ready HTML can now be cross-platform.  The prior approach
     only seemed to work on Linux, and then only if the right utilities
     were installed.

  *  Pikchr provides additional flexibility in the formatting of
     syntax diagrams, so that the diagrams can be made easier to read
     and understand.

## How It Works

The SQLite documentation is generated using [a Tcl script][2] from
source documents. The script evaluates Tcl code embedded into the
source files between `<tcl>`...`</tcl>` tags, extends HTML with
enhanced hyperlinks and formatting features, and outputs the result
as standard HTML.

[2]: https://sqlite.org/docsrc/file/wrap.tcl

Each syntax diagram is a file in the
[art/syntax/][3] subdirectory of the [documentation source repository][4].
You can click on [any of the Pikchr source files][3] to see the corresponding
diagram.  Click on the "Text" submenu option to see the original Pikchr
source text rather than the rendered SVG.

[3]: https://www.sqlite.org/docsrc/dir/art/syntax?ci=trunk
[4]: https://www.sqlite.org/docsrc/doc/trunk/README.md

### Pikchr as a Tcl Extension

The [`pikchr.c` source file][src] can be compiled into a Tcl extension by
adding the `-DPIKCHR_TCL` compile-time option.  As a Tcl extension,
Pikchr provides a single new Tcl command named "`pikchr`" which takes
a single argument: the Pikchr source text.  The `pikchr` command returns
a Tcl list of three elements which are:

   1.  The SVG output text
   2.  The width of the output in pixels
   3.  The height of the output in pixels

As with ordinary Pikchr, a negative width is returned if the input text
contains an error, and the output text is the error message.

[src]: https://pikchr.org/home/file/pikchr.c

### Automatic Insertion of Diagrams into Documentation

Within the SQLite documentation source text, markup of the following
form causes the named Pikchr syntax diagram to be loaded, converted
into SVG, and the output SVG added to the documentation under construction:

~~~~
   <tcl>
   RecursiveBubbleDiagram expr
   </tcl>
~~~~

The RecursiveBubbleDiagram Tcl procedure uses the "`pikchr`" Tcl command
to convert the Pikchr source text into SVG and inserts that SVG.  The
command also looks for other diagrams that are referenced by that diagram
and loads them as well, together with appropriate JavaScript to cause the
sub-diagrams to be initially hidden, but to expand when the reader clicks on
the appropriate links.
