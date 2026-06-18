# This GNU makefile exists primarily to simplify/speed up development
# from emacs. It is not part of the canonical build process.
default: fiddle

fiddle_module_js := pikchr.js
fiddle_module_wasm := pikchr.wasm
fiddle_generated := $(fiddle_module_js) $(fiddle_module_wasm)
pikchr_c := ../pikchr.c

ifneq (,$(wildcard /home/stephan))
  emcc_opt ?= -O0
else
  emcc_opt ?= -Oz
endif

$(pikchr_c):
	$(MAKE) -C ..

$(fiddle_module_js): $(pikchr_c) GNUmakefile
	emcc -o $@ $(emcc_opt) --no-entry \
        -sEXPORTED_RUNTIME_METHODS=cwrap,setValue,getValue,stackSave,stackRestore \
        -sEXPORTED_FUNCTIONS=_pikchr $(pikchr_c) \
        -sENVIRONMENT=web \
        -sMODULARIZE \
        -sEXPORT_NAME=initPikchrModule \
        --minify 0;
	gzip -c < $@ > $@.gz
	gzip -c < pikchr.wasm > pikchr.wasm.gz
pikchr.wasm: $(fiddle_module_js)
pikchr.js.gz: $(fiddle_module_js)
pikchr.wasm.gz: $(fiddle_module_js)
fiddle.js:
fiddle.js.gz: fiddle.js
	gzip -c < $< > $@

fiddle_compressed := pikchr.js.gz pikchr.wasm.gz fiddle.js.gz
fiddle_generated += $(fiddle_compressed)
fiddle_files = emscripten.css fiddle.html \
             fiddle.js pikchr.js \
             pikchr.wasm pikchr-worker.js \
             $(fiddle_compressed)

fiddle: $(fiddle_compressed)
clean:
	rm -f $(fiddle_generated)
disclean: clean

# fiddle_remote is the remote destination for the fiddle app. It
# must be a [user@]HOST:/path for rsync.
# Note that the target "should probably" contain a symlink of
# index.html -> fiddle.html.
fiddle_remote ?=
ifeq (,$(fiddle_remote))
ifneq (,$(wildcard /home/stephan))
  fiddle_remote = wh2:www/wh/pikchr/.
else ifneq (,$(wildcard /home/drh))
  #fiddle_remote = if appropriate, add that user@host:/path here
endif
endif

push-fiddle: $(fiddle_files)
	@if [ x = "x$(fiddle_remote)" ]; then \
		echo "fiddle_remote must be a [user@]HOST:/path for rsync"; \
		exit 1; \
	fi
	rsync -va $(fiddle_files) $(fiddle_remote)
