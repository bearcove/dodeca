CC = cc
CFLAGS = -O0 -g -Wall -Wextra
LIBS = -lm

TCL_PACKAGE_NAME    = pikchr
TCL_PACKAGE_VERSION = 1.0
TCL_PACKAGE_FLAGS = \
  -DPACKAGE_NAME="\"$(TCL_PACKAGE_NAME)\"" \
  -DPACKAGE_VERSION="\"$(TCL_PACKAGE_VERSION)\""

all:	pikchr

pikchr:	pikchr.c
	$(CC) $(CFLAGS) -DPIKCHR_SHELL pikchr.c -o pikchr $(LIBS)

pikchrfuzz:	pikchr.c
	clang -g -O3 -fsanitize=fuzzer,undefined,address -o pikchrfuzz \
	  -DPIKCHR_FUZZ pikchr.c $(LIBS)

# NOTE: pikchr.c generation from pikchr.y has been disabled to preserve
# our debug modifications. If you need to regenerate, run manually:
#   ./lemon pikchr.y && cat pikchr.h.in >pikchr.h

VERSION.h:	VERSION manifest manifest.uuid mkversion.c
	$(CC) -o mkversion mkversion.c
	./mkversion manifest.uuid manifest VERSION >VERSION.h

piktcl: pikchr.c
	mkdir -p piktcl
	$(CC) -shared -fPIC -o piktcl/libpikchr.so \
	      $(CFLAGS) \
	      -DPIKCHR_TCL $(TCL_PACKAGE_FLAGS) \
	      -I /usr/include/tcl8.6 \
	      pikchr.c \
	      -lm \
	      -L /usr/lib/x86_64-linux-gnu/ \
	      -ltcl8.6
	echo >> piktcl/pkgIndex.tcl \
	      "package ifneeded $(TCL_PACKAGE_NAME) $(TCL_PACKAGE_VERSION) [list load [file join" '$$dir' "libpikchr.so]]"

lemon:	lemon.c
	$(CC) $(CFLAGS) lemon.c -o lemon

test:	pikchr
	./pikchr --dont-stop tests/*.pikchr >out.html
	open out.html
	./pikchr --dont-stop --dark-mode */*.pikchr >darkmode.html
	open darkmode.html

clean:
	@echo "❌ We do not clean here. Debug traces and generated files are kept for development."
	@echo "   The C implementation is not used in production - only the Rust port is."
	@echo "   If you really need to clean, delete files manually."
