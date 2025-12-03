# Download Options

## Building Pikchr from source code

The `pikchr.c` source file and its accompanying `pikchr.h` header
are build products.  They are not found directly in the version control
system.  You have to download the source tree and run a makefile in order
to construct those files.  Fortunately, that is not difficult.  There
are no external dependencies apart from a C-compiler.

Once you have the source tree available, as described below,
run one of the following "make" command to build pikchr.c on
Unix-like systems including MacOS:

> ~~~
make pikchr.c
~~~

On Windows, using MSVC do:

> ~~~
nmake /f Makefile.msc pikchr.c
~~~

To build the Pikchr command-line shell, use the `pikchr` or `pikchr.exe`
makefile target.  To run tests, use the `test` makefile target.

## Obtaining the complete Pikchr source tree

A tarball or ZIP archive of the latest source code is available
at the following links:

  *  <https://pikchr.org/home/tarball/trunk/pikchr.tar.gz>
  *  <https://pikchr.org/home/zip/trunk/pikchr.zip>

With the complete source tree on your local machine, you can run
"`make`" or "`nmake`" to build and test Pikchr.

## Clone the Fossil Repository

Pikchr uses [Fossil](https://fossil-scm.org/home) for version control.
You can clone the entire repository (which includes everything on
this website, including all the documentation and test cases) as follows:

  *  [Install Fossil](https://fossil-scm.org/home/uv/download.html)
      if you haven't done so already

  *  `fossil clone https://pikchr.org/ pikchr.fossil`

  *  `fossil open pikchr.fossil`

After you have the repository cloned, you can bring in any updates using:

  *  `fossil up trunk`

Once you have a clone of the Fossil repository, you can bring up a
copy of this website on your local machine by typing:

  *  `fossil ui`

See the [Fossil documentation][fossil-doc] for more information on how
manage a Fossil repository.

[fossil-doc]: https://fossil-scm.org/home/doc/trunk/www/permutedindex.html

## Clone the GitHub Mirror

There is a (read-only) mirror of this repository
[on GitHub](https://github.com/drhsqlite/pikchr).
