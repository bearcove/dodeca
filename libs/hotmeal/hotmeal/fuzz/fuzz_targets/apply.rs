#![no_main]

//! Native diff+apply fuzzer.
//!
//! Tests that hotmeal can diff two DOMs and apply patches correctly.

use libfuzzer_sys::fuzz_target;
use std::sync::Once;

mod common;

static INIT: Once = Once::new();

fuzz_target!(|data: &[u8]| {
    INIT.call_once(|| {
        common::setup_tracing();
    });

    common::apply_roundtrip(data);
});
