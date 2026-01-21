// Zero-cost tracing macros for html-diff-tests
//
// These macros forward to tracing when the `tracing` feature is enabled or in tests,
// and compile to nothing otherwise.

#[cfg(any(test, feature = "tracing"))]
macro_rules! trace {
    ($($arg:tt)*) => { ::tracing::trace!($($arg)*) }
}

#[cfg(not(any(test, feature = "tracing")))]
macro_rules! trace {
    ($($arg:tt)*) => {};
}

#[cfg(any(test, feature = "tracing"))]
macro_rules! debug {
    ($($arg:tt)*) => { ::tracing::debug!($($arg)*) }
}

#[cfg(not(any(test, feature = "tracing")))]
macro_rules! debug {
    ($($arg:tt)*) => {};
}

pub(crate) use trace;
pub(crate) use debug;
