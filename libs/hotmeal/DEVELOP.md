# Development Guide

This document captures the contributor workflow for fuzzing, turning fuzz output into repro tests, and using tracing to debug parser mismatches and regressions.

## 1) Fuzzing

Hotmeal uses cargo-fuzz (libFuzzer) to find bugs in the diff/patch implementation and parser/browser mismatches.

### Run the fuzzer (roundtrip target)

You must run from the `hotmeal/fuzz` subdirectory:

```bash
cd hotmeal/fuzz
cargo fuzz run roundtrip -- -max_total_time=60
```

This fuzzer:
- Generates random HTML structures
- Computes diffs between old and new HTML
- Applies patches to transform old → new
- Verifies the result matches the expected output

When a crash is found, libFuzzer will:
- Print the failing input and assertion details
- Save the crash to `artifacts/roundtrip/crash-<hash>`
- Show the `Debug` representation of the input
- Provide commands to reproduce and minimize

### Browser-side fuzzing (`just fuzz-browser`)

Run the browser harness (from repo root):

```bash
just fuzz-browser
```

Under the hood this runs:

```bash
cd hotmeal/fuzz && cargo fuzz run browser -- -dict=html.dict {{ ARGS }}
```

This compares the DOM produced by the browser with the DOM produced by `html5ever`, and reports parser mismatches.

### Minimizing a crash artifact (browser target)

From `hotmeal/fuzz`:

```bash
cd ./hotmeal/fuzz && cargo fuzz tmin browser artifacts/browser/crash-14e7471987ca5f1056fed714f64d5f16a3906273
```

### Showcase: minimized repro run output

```bash
› cargo fuzz run browser artifacts/browser/minimized-from-c6ac40c130114b9c6804f3ec53916a530cb1c5e3
    Finished `release` profile [optimized + debuginfo] target(s) in 0.10s
    Finished `release` profile [optimized + debuginfo] target(s) in 0.08s
     Running `target/aarch64-apple-darwin/release/browser -artifact_prefix=/Users/amos/bearcove/hotmeal/hotmeal/fuzz/artifacts/browser/ artifacts/browser/minimized-from-c6ac40c130114b9c6804f3ec53916a530cb1c5e3`
INFO: Running with entropic power schedule (0xFF, 100).
INFO: Seed: 694628087
INFO: Loaded 1 modules   (653595 inline 8-bit counters): 653595 [0x103a9c930, 0x103b3c24b),
INFO: Loaded 1 PC tables (653595 PCs): 653595 [0x103b3c250,0x104535400),
target/aarch64-apple-darwin/release/browser: Running 1 inputs 1 time(s) each.
Running: artifacts/browser/minimized-from-c6ac40c130114b9c6804f3ec53916a530cb1c5e3
[browser-fuzz] WebSocket server listening on port 52997
[browser-fuzz] Chrome launched (pid 28716)
[browser-fuzz] Loading bundle from: file:///Users/amos/bearcove/hotmeal/hotmeal/fuzz/browser-bundle/dist/index.html#52997
[browser-fuzz] Page loaded, WASM will connect automatically
[browser-fuzz] Waiting for browser to connect on Ok(127.0.0.1:52997)...
[browser-fuzz] TCP accept from 127.0.0.1:53002, upgrading to WebSocket...
[browser-fuzz] WebSocket upgrade complete, starting roam handshake...
[browser-fuzz] Roam handshake complete
[browser-fuzz] Ready to process fuzz requests
[browser-fuzz] Waiting for browser to connect on Ok(127.0.0.1:52997)...

========== PARSER MISMATCH ==========
Input: "</html></'"

--- html5ever tree ---
<body>
</body>

--- browser tree ---
<body>
  COMMENT: "'</body"
</body>


--- diff ---
 <body>
+  COMMENT: "'</body"
 </body>
=====================================

[browser-fuzz] Killing Chrome (pid 28716)

thread '<unnamed>' (26916050) panicked at fuzz_targets/browser.rs:386:9:
Parser mismatch detected! Fix html5ever to match browser.
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
==28715== ERROR: libFuzzer: deadly signal
    #0 0x000105dd53c4 in __sanitizer_print_stack_trace+0x28 (librustc-nightly_rt.asan.dylib:arm64+0x5d3c4)
    #1 0x000101183470 in fuzzer::PrintStackTrace()+0x30 (browser:arm64+0x1003b3470)
    #2 0x000101177670 in fuzzer::Fuzzer::CrashCallback()+0x54 (browser:arm64+0x1003a7670)
    #3 0x0001997f3740 in _sigtramp+0x34 (libsystem_platform.dylib:arm64+0x3740)
    #4 0x0001997e9884 in pthread_kill+0x124 (libsystem_pthread.dylib:arm64+0x6884)
    #5 0x0001996ee84c in abort+0x78 (libsystem_c.dylib:arm64+0x7984c)
    #6 0x000102fae864 in _RNvNtNtNtCs5sEH5CPMdak_3std3sys3pal4unix14abort_internal+0x8 (browser:arm64+0x1021de864)
    #7 0x000102fae5e8 in _RNvNtCs5sEH5CPMdak_3std7process5abort+0x8 (browser:arm64+0x1021de5e8)
    #8 0x000102f13a8c in _RNCNvCse0tYFqfIeAn_13libfuzzer_sys10initialize0B3_+0xb8 (browser:arm64+0x102143a8c)
    #9 0x000100f2d9fc in _RNCNCNvCs27naSEvP82y_7browser19setup_cleanup_hooks00B5_ browser.rs:125
    #10 0x000102ee09f4 in _RNvNtCs5sEH5CPMdak_3std9panicking15panic_with_hook+0x264 (browser:arm64+0x1021109f4)
    #11 0x000102ecd13c in _RNCNvNtCs5sEH5CPMdak_3std9panicking13panic_handler0B5_+0x6c (browser:arm64+0x1020fd13c)
    #12 0x000102ec2bc8 in _RINvNtNtCs5sEH5CPMdak_3std3sys9backtrace26___rust_end_short_backtraceNCNvNtB6_9panicking13panic_handler0zEB6_+0x8 (browser:arm64+0x1020f2bc8)
    #13 0x000102ecd964 in _RNvCsKhRCbHf33p_7___rustc17rust_begin_unwind+0x1c (browser:arm64+0x1020fd964)
    #14 0x000102faf134 in _RNvNtCsjMrxcFdYDNN_4core9panicking9panic_fmt+0x24 (browser:arm64+0x1021df134)
    #15 0x000101075628 in _RNvNvCs27naSEvP82y_7browser1__19___libfuzzer_sys_run browser.rs:386
    #16 0x0001010c0ec0 in rust_fuzzer_test_input lib.rs:276
    #17 0x000101175c24 in _RINvNvNtCs5sEH5CPMdak_3std9panicking12catch_unwind7do_callNCNvCse0tYFqfIeAn_13libfuzzer_sys15test_input_wrap0lEBY_+0xc4 (browser:arm64+0x1003a5c24)
    #18 0x0001011768ec in __rust_try+0x18 (browser:arm64+0x1003a68ec)
    #19 0x000101175524 in LLVMFuzzerTestOneInput+0x16c (browser:arm64+0x1003a5524)
    #20 0x000101178f20 in fuzzer::Fuzzer::ExecuteCallback(unsigned char const*, unsigned long)+0x158 (browser:arm64+0x1003a8f20)
    #21 0x000101194a1c in fuzzer::RunOneTest(fuzzer::Fuzzer*, char const*, unsigned long)+0xd8 (browser:arm64+0x1003c4a1c)
    #22 0x00010119968c in fuzzer::FuzzerDriver(int*, char***, int (*)(unsigned char const*, unsigned long))+0x1b8c (browser:arm64+0x1003c968c)
    #23 0x0001011a5fac in main+0x24 (browser:arm64+0x1003d5fac)
    #24 0x000199421d50  (<unknown module>)

NOTE: libFuzzer has rudimentary signal handlers.
      Combine libFuzzer with AddressSanitizer or similar for better crash reports.
SUMMARY: libFuzzer: deadly signal
────────────────────────────────────────────────────────────────────────────────

Error: Fuzz target exited with exit status: 77
```

## 2) Turning fuzz output into a repro test

### Repro-first workflow (mandatory)

For parser mismatches or fuzz regressions, always follow this order:

1. Add a focused unit test (use `trace!` where it helps explain tree state).
2. Run it via nextest with tracing enabled and confirm it fails.
3. Only then start hunting for root cause.

### Steps to build the repro test

1. **Extract the HTML from the fuzzer output:**
   - The fuzzer prints `Old:` and `New:` HTML strings
   - Or look at the `Debug` output showing the `FuzzInput` structure

2. **Add a test near existing tests using the project’s test helper attribute:**
   - Examples of where this is used today: `hotmeal/src/dom.rs` and `hotmeal/src/diff.rs`
   - Use `use facet_testhelpers::test;` and apply `#[test]` from that helper

```bash
#[test]
fn test_fuzzer_<description>() {
    let old_html = r#"<html>...</html>"#;
    let new_html = r#"<html>...</html>"#;

    let patches = super::super::diff_html(old_html, new_html)
        .expect("diff failed");
    trace!(?patches, "patches");

    let mut tree = super::super::apply::parse_html(old_html)
        .expect("parse old failed");
    super::super::apply::apply_patches(&mut tree, &patches)
        .expect("apply failed");

    let result = tree.to_html();
    let expected_tree = super::super::apply::parse_html(new_html)
        .expect("parse new failed");
    let expected = expected_tree.to_html();

    trace!(%result, %expected, "roundtrip result");
    assert_eq!(result, expected, "HTML output should match");
}
```

3. **Run the test with nextest and tracing enabled (confirm it fails):**

```bash
FACET_LOG=trace cargo nextest run --no-capture test_fuzzer_<description> -F tracing
```

## 3) Tracing setup

Hotmeal defines its own tracing macros that compile to no-ops unless tracing is enabled. Tracing is enabled when the `tracing` feature is on, and in tests it’s always available. Use `FACET_LOG=trace` to surface trace output.

### Run tests with tracing enabled (nextest)

```bash
FACET_LOG=trace cargo nextest run --no-capture test_parser_mismatch_li_u_svg -F tracing
```

### Prefer the project’s trace macro with lazy tree dumps

Use the project’s `trace!` macro and pass a lazy tree dump so it only renders when tracing is enabled.

```bash
trace!(
    parent_id = %node_id_short(*parent),
    node_id = %node_id_short(node),
    tree = %LazyTreeDump::new(&arena, *parent, &highlights),
    "append: before insert"
);
```

This matches the style already used in `src/dom.rs` and avoids expensive debug output unless tracing is active.