//! Type-erased tokio spawn helper to reduce monomorphization.
//!
//! Each unique future type passed to `tokio::spawn` creates monomorphized copies
//! of the entire task infrastructure (~30k+ lines per type). By boxing futures
//! before spawning, we erase the type and share a single implementation.

use std::future::Future;
use std::pin::Pin;

/// Spawn a future with type erasure to reduce monomorphization.
///
/// This boxes the future before passing to tokio::spawn, so all spawned
/// futures share the same `Pin<Box<dyn Future>>` type instead of each
/// having a unique monomorphized task infrastructure.
#[inline(always)]
pub fn spawn<T>(f: impl Future<Output = T> + Send + 'static) -> tokio::task::JoinHandle<T>
where
    T: Send + 'static,
{
    tokio::spawn(Box::pin(f) as Pin<Box<dyn Future<Output = T> + Send>>)
}
