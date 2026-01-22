// Zero-cost tracing macros for hotmeal diff
//
// These compile to nothing - can be extended later for debugging.

macro_rules! trace {
    ($($arg:tt)*) => {};
}

macro_rules! debug {
    ($($arg:tt)*) => {};
}

#[allow(unused_imports)]
pub(crate) use trace;

#[allow(unused_imports)]
pub(crate) use debug;
