# How To Integrate Pikchr Into New Systems

Pikchr is (currently) implemented in C.

  *  It uses no external libraries other than the standard C library and
     the standard math library (for sin(), cos(), and some others).
  *  It is completely contained in a single source code file:
     [`pikchr.c`](/file/pikchr.c).  There is also a header file
     [`pikchr.h`](/file/pikchr.h) available if you want it, but it is not
     required.
  *  It uses a single C-language interface routine: `pikchr()`

Any existing Markdown or other wiki rendering engine that can invoke
a C-language library should be able to integrate Pikchr quickly and
easily.  The code has been audited and fuzzed and is
believed to be impervious to hostile inputs.

## C-language interface.

There is a single interface function:

~~~~
  char *pikchr(
    const char *zText,     /* Input PIKCHR source text.  zero-terminated */
    const char *zClass,    /* Add class="%s" to <svg> markup */
    unsigned int mFlags,   /* Flags used to influence rendering behavior */
    int *pnWidth,          /* Write width of <svg> here, if not NULL */
    int *pnHeight          /* Write height here, if not NULL */
  );
~~~~

To convert Pikchr into SVG text ready to be inserted into the HTML output
stream, simply invoke the pikchr() function, passing the source text
as the first argument.  The SVG output text is returned, and the desired
width and height of that text is written into the *pnWidth and *pnHeight
variables.

If the input Pikchr text contains errors, a negative number is
written into *pnWidth and the returned text is an error message ready
to be dropped into "`<pre>...</pre>`".  Any "`<`" or "`>`" or
"`&`" characters in the error message text have already been escaped,
so the error message can be inserted directly into an HTML output stream
without further processing.

The returned string is held in memory obtained from `malloc()`.  The caller
is responsible for freeing this memory to prevent a memory leak.  It is
possible (though unlikely) for pikchr() to return a NULL pointer, for
example if it hits a `malloc()` failure.

If the zClass parameter is not NULL, then it is an extra class name
(or names) that is inserted into the "`<svg>`" element of the returned
string.

## Flags passed to pikchr()

The [`pikchr.h`](/file/pikchr.h) header file currently defines two flags
that can be passed into the pikchr() function as the 3rd argument, "mFlags".
(Additional flags might get added in future releases.)

   *  `PIKCHR_PLAINTEXT_ERRORS` &rarr;
      Normally, the text returned by pikchr() in the event of an error
      is formatted as HTML.  Setting this flag causes the error message
      to be plain text.

   *  `PIKCHR_DARK_MODE` &rarr;
      When this flag is used,  Pikchr inverts the colors in the diagram
      to make them suitable for "dark mode" pages.  The main Pikchr
      website can be switched between
      [dark-mode](./integrate.md?skin=darkmode) and
      [light-mode](./integrate.md?skin=) so
      that you can see the effects of this flag on Pikchr diagrams.

## Example use of pikchr()

The "`pikchr.c`" source file itself contains an example use of the
pikchr() function.  If "`pikchr.c`" is compiled with the `-DPIKCHR_SHELL`
compile-time option, it will include a `main()` that reads all the
files named as arguments, runs each through pikchr() and outputs
the result embedded in HTML.  So if you want an example, look at the
"main()" function at the bottom of the "`pikchr.c`" source file.

## Performance considerations

Pikchr seems to use about 650 CPU cycles per byte of input.  So even
a slow core can handle on the order of 3 or 4 megabytes of Pikchr input
per second.  As most Pikchr scripts are less than 1000 bytes, the processing
overhead of running Pikchr is likely to be too small to measure.  Pikchr
could perhaps be optimized to increase its performance, but it is so fast
already (especially compared to the rest of the Markdown formatting
stream) that we don't see any point in that.  Contact the developers if
you uncover evidence that contradicts anything in this paragraph.

## Fuzz Testing

You can build a [libFuzzer][1]-based fuzz tester for Pikchr by
compiling like this (or similarly):

~~~~
   clang -g -O3 -fsanitize=fuzzer,undefined,address -o fuzz -DPIKCHR_FUZZ pikchr.c
~~~~

Gather a bunch of Pikchr scripts to be used as seeds (perhaps from the
tests/ or examples/ subdirectories of the source tree) and put them in
a subdirectory, which we will call "fz".  Then run:

~~~~
   fuzz fz
~~~~

We have run this for hundreds of millions of tests already.  You
are welcomed to run more.


[1]: https://www.llvm.org/docs/LibFuzzer.html
