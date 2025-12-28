//! Conditional logging macros.
//!
//! When the `tracing` feature is enabled, these re-export `tracing` macros.
//! When disabled, they expand to no-ops for zero runtime overhead.

#[cfg(feature = "tracing")]
pub use tracing::{debug, warn};

#[cfg(not(feature = "tracing"))]
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {};
}

#[cfg(not(feature = "tracing"))]
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {};
}

#[cfg(not(feature = "tracing"))]
pub use crate::{debug, warn};
