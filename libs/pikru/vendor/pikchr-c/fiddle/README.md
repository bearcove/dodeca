# Building and Running the pikchr Fiddle App

This application uses a [WebAssembly][wasm], a.k.a. WASM, build of
pikchr to provide a so-called "fiddle" app, for "fiddling around" with
pikchr. It requires:

- The [Emscripten][emscripten] SDK (see below)
- A modern browser

## Setting up Emscripten

First, install the Emscripten SDK, as documented
[here](https://emscripten.org/docs/getting_started/downloads.html) and summarized
below for Linux environments:

```
# Clone the emscripten repository:
$ git clone https://github.com/emscripten-core/emsdk.git
$ cd emsdk

# Download and install the latest SDK tools:
$ ./emsdk install latest

# Make the "latest" SDK "active" for the current user:
$ ./emsdk activate latest
```

Those parts only need to be run once. To update that tree to
the latest version, use:

```
$ git pull
$ ./emsdk install latest
$ ./emsdk activate latest
```

## Setting up the EMSDK Environment

The following needs to be run for each shell instance which needs the
`emcc` compiler:

```
# Activate PATH and other environment variables in the current terminal:
$ source /path/tp/emsdk/emsdk_env.sh

$ which emcc
/path/to/emsdk/upstream/emscripten/emcc
```

## Building with non-Emscripten WASM Toolchains

Adding alternate builds for non-Emscripten toolchains is on the
TODO list. Doing so requires:

- WASM builds of libc and libm. [wasi-sdk][] "should" be suitable but
  is as yet untested.
- Replacing much of the "glue code" which Emscripten installs. This
  work has already been done in other trees closely related to this
  project.


# Building Fiddle

The makefile for the fiddle app requires GNU Make (installed as "make"
on Linux and typically "gmake" on BSD systems).  From the top of the
pikchr tree, simply do:


```
$ source /path/to/emsdk/emsdk_env.sh # see previous section
$ cd fiddle
$ make
# or, depending on the platform (e.g. BSD):
$ gmake
```

The EMSDK environment setup is only needed for building, not running
the app.

# Running Fiddle (on Unix Systems)

Due to limitations in WASM module loading in some browsers,
the fiddle app must be run via an HTTP server rather than by
opening its HTML file directly. There are many options for doing so,
but one of the simplest is to use [althttpd][]:

```
$ althttpd -page fiddle.html
```

That will start up a local HTTP server on the next available network
port at or above 8080, start up your browser, and point it to
`http://localhost:THAT_PORT/fiddle.html`.

# Exploring the Code

From highest level to lowest:

- `fiddle.html` is the entry point for...
- `fiddle.js` is the main hand-written JS part of the app. It loads...
- `pikchr-worker.js` is a hand-written Worker-API binding to...
- `pikchr.js` is the Emscripten-generated module code. It loads
  `pikchr.wasm` (the WASM-compiled from of `pikchr.c`) and makes it
  available to JS.


[wasm]: https://developer.mozilla.org/en-US/docs/WebAssembly
[emscripten]: https://emscripten.org/
[althttpd]: https://sqlite.org/althttpd
[wasi-sdk]: https://github.com/WebAssembly/wasi-sdk
