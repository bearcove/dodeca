# Release History

## Version 1.1 (pending)

  *  Adjust the `<svg>` markup slightly to work around an issue in Safari.
  *  Fix a bug in the "with EDGE at POSITION" syntax reported in
     [forum post f9f5d90f33](/forumpost/f9f5d90f33).
  *  Add the --version argument to the "pikchr" command-line tool.
  *  Add the `pikchr_version()` API function that returns a string showing
     the Pikchr version number and the date/time of the source code check-in.
  *  Include the ISO timestamp of the source code check-in in a data-*
     field of the `<svg>` element.
  *  Add a new keyword "`pikchr_date`" to the language.  This keyword acts
     like a string literal that contains the check-in date of the source code.
     So another way to find the specific version of Pikchr that is running is
     to render a pikchr script like: "`box pikchr_date fit;`"
  *  The `pikchr.c` and `pikchr.h` sources are build products.  They were
     formerly under source code management, for convenience.  But as they now
     contain version-dependent information (such as the source code check-in
     timestamp) they are no longer versioned.  You need to run the Makefile
     to generate those files from canonical sources.

## Version 1.0 (2024-04-01) [20240401101739](/info/20240401101739)

  *  Add a version number (1.0 in this case) because
     Homebrew requires all the software they deliver to
     have a version number, and we want Pikchr to be
     available via Homebrew.
