//! Plugcard: Dynamic library plugin system with serialized method calls.
//!
//! # Overview
//!
//! Plugcard provides a way to define plugin interfaces using regular Rust functions,
//! then load and call those functions across dynamic library boundaries. It uses
//! [facet-postcard] for serialization and [facet] for introspection.
//!
//! # Defining a plugin
//!
//! ```rust,ignore
//! use plugcard::plugcard;
//!
//! #[plugcard]
//! pub fn greet(name: String) -> String {
//!     format!("Hello, {name}!")
//! }
//! ```
//!
//! The `#[plugcard]` macro generates:
//! - A wrapper function with C ABI that handles serialization/deserialization
//! - Registration in a global method table via [linkme]
//! - Type name information for the input and output types
//!
//! # Current limitations
//!
//! All arguments are currently serialized via postcard, which means large buffers
//! (like image pixel data) are copied. This is fine for small payloads but suboptimal
//! for multi-megabyte data.
//!
//! # Future: Transport layer
//!
//! The plan is to add a transport abstraction on the caller side that can choose
//! between different strategies based on context:
//!
//! - **Local transport** (same-process dylib): For `&[u8]` arguments, pass raw
//!   pointers instead of serializing. The borrow is held for the duration of the
//!   call, so lifetime safety is guaranteed by Rust's type system. Zero-copy.
//!
//! - **Remote transport** (IPC, network, WASM): Full postcard serialization.
//!   Borrowed data is copied into the message. Higher overhead but gains isolation
//!   (crash safety, sandboxing, cross-machine).
//!
//! The plugin code is identical either way—the transport layer is an adapter on
//! the host side that speaks the right protocol for the situation. This means you
//! could load the same plugin both ways: local for hot paths, remote for untrusted
//! input that needs sandboxing.
//!
//! The key insight: for local transport, the synchronous function call *is* the
//! lifetime scope. No capability tokens or blob stores needed—Rust's borrow checker
//! already enforces that the caller can't free the data while the plugin is using it.

use facet::Facet;
use linkme::distributed_slice;

/// ABI version for plugcard plugins.
///
/// This version is checked when loading plugins to detect stale .so files
/// built against incompatible versions of plugcard. Bump this when:
/// - MethodSignature struct layout changes
/// - MethodCallData struct layout changes
/// - The plugin export interface changes
///
/// Format: 0xMMMMmmpp (Major, minor, patch as u32)
pub const ABI_VERSION: u32 = 0x0001_0002; // 1.0.2 - added host callback

/// Log level for plugin log messages
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

/// Type for the log callback function.
///
/// Arguments: (level, message_ptr, message_len)
/// The message is UTF-8 encoded.
pub type LogCallback = extern "C" fn(LogLevel, *const u8, usize);

/// Result of a host service call.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostCallResult {
    /// Call succeeded, output contains serialized result
    #[default]
    Success,
    /// Unknown service ID
    UnknownService,
    /// Failed to deserialize input
    DeserializeError,
    /// Failed to serialize output (buffer too small)
    BufferTooSmall,
    /// Service returned an error
    ServiceError,
}

/// Data for host service calls (plugin -> host).
#[repr(C)]
pub struct HostCallData {
    /// Service identifier
    pub service_id: u32,
    /// Pointer to serialized input data
    pub input_ptr: *const u8,
    /// Length of input data
    pub input_len: usize,
    /// Pointer to output buffer (plugin-allocated)
    pub output_ptr: *mut u8,
    /// Capacity of output buffer
    pub output_cap: usize,
    /// Actual length written to output (set by host)
    pub output_len: usize,
    /// Result status (set by host)
    pub result: HostCallResult,
}

/// Type for the host callback function.
///
/// Plugins call this to request services from the host (e.g., syntax highlighting).
/// The callback reads input from input_ptr/input_len, writes output to output_ptr,
/// sets output_len and result.
pub type HostCallback = extern "C" fn(*mut HostCallData);

/// Well-known host service IDs.
pub mod host_services {
    /// Syntax highlighting service.
    /// Input: HighlightRequest { code: String, language: String }
    /// Output: HighlightResult { html: String, highlighted: bool }
    pub const HIGHLIGHT_CODE: u32 = 1;
}

/// A Result type that can be serialized with facet-postcard.
///
/// Use this instead of `std::result::Result<T, String>` in plugin functions
/// since `Result` doesn't implement `Facet`.
#[derive(Facet, Debug, Clone)]
#[repr(u8)]
pub enum PlugResult<T> {
    /// Operation succeeded
    Ok(T),
    /// Operation failed with an error message
    Err(String),
}

impl<T> From<Result<T, String>> for PlugResult<T> {
    fn from(result: Result<T, String>) -> Self {
        match result {
            Ok(v) => PlugResult::Ok(v),
            Err(e) => PlugResult::Err(e),
        }
    }
}

impl<T> From<PlugResult<T>> for Result<T, String> {
    fn from(result: PlugResult<T>) -> Self {
        match result {
            PlugResult::Ok(v) => Ok(v),
            PlugResult::Err(e) => Err(e),
        }
    }
}

/// Distributed slice containing all registered method signatures
#[distributed_slice]
pub static METHODS: [MethodSignature];

/// A method signature with type information for introspection
#[derive(Debug, Clone)]
pub struct MethodSignature {
    /// Unique key derived from method name and type names
    pub key: u64,
    /// Human-readable method name
    pub name: &'static str,
    /// Type name for the input type
    pub input_type_name: &'static str,
    /// Type name for the output type
    pub output_type_name: &'static str,
    /// The wrapper function to call
    pub call: unsafe extern "C" fn(*mut MethodCallData),
}

/// Data passed to method wrappers across FFI boundary
#[repr(C)]
pub struct MethodCallData {
    /// Method signature key (for validation)
    pub key: u64,
    /// Pointer to serialized input data
    pub input_ptr: *const u8,
    /// Length of input data
    pub input_len: usize,
    /// Pointer to output buffer (caller-allocated)
    pub output_ptr: *mut u8,
    /// Capacity of output buffer
    pub output_cap: usize,
    /// Actual length written to output (set by callee)
    pub output_len: usize,
    /// Optional log callback for plugin to send log messages to host
    pub log_callback: Option<LogCallback>,
    /// Optional host callback for plugin to call host services
    pub host_callback: Option<HostCallback>,
    /// Result status
    pub result: MethodCallResult,
}

/// Result of a method call
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MethodCallResult {
    /// Call succeeded, output contains serialized result
    #[default]
    Success,
    /// Failed to deserialize input
    DeserializeError,
    /// Failed to serialize output (buffer too small?)
    SerializeError,
    /// Method returned an error (serialized in output)
    MethodError,
    /// Unknown method key
    UnknownMethod,
}

/// Find a method by its key
pub fn find_method(key: u64) -> Option<&'static MethodSignature> {
    METHODS.iter().find(|m| m.key == key)
}

/// Dispatch a method call by key
///
/// # Safety
/// - `data` must point to valid MethodCallData
/// - input_ptr/input_len must be valid
/// - output_ptr/output_cap must be valid writable buffer
pub unsafe fn dispatch(data: *mut MethodCallData) {
    unsafe {
        let data_ref = &mut *data;

        if let Some(method) = find_method(data_ref.key) {
            (method.call)(data);
        } else {
            data_ref.result = MethodCallResult::UnknownMethod;
        }
    }
}

/// Compute a method key from name and type names (const-compatible FNV-1a hash)
pub const fn compute_key(name: &str, input_type_name: &str, output_type_name: &str) -> u64 {
    // FNV-1a hash constants
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;

    // Hash name
    let name_bytes = name.as_bytes();
    let mut i = 0;
    while i < name_bytes.len() {
        hash ^= name_bytes[i] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }

    // Hash input type name
    let input_bytes = input_type_name.as_bytes();
    let mut i = 0;
    while i < input_bytes.len() {
        hash ^= input_bytes[i] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }

    // Hash output type name
    let output_bytes = output_type_name.as_bytes();
    let mut i = 0;
    while i < output_bytes.len() {
        hash ^= output_bytes[i] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }

    hash
}

/// List all available methods (for introspection)
pub fn list_methods() -> &'static [MethodSignature] {
    &METHODS
}

// ============================================================================
// Plugin-side logging and host callback support
// ============================================================================

use std::cell::Cell;

thread_local! {
    /// Thread-local storage for the current log callback.
    /// Set by the wrapper before calling the plugin function.
    static CURRENT_LOG_CALLBACK: Cell<Option<LogCallback>> = const { Cell::new(None) };

    /// Thread-local storage for the current host callback.
    /// Set by the wrapper before calling the plugin function.
    static CURRENT_HOST_CALLBACK: Cell<Option<HostCallback>> = const { Cell::new(None) };
}

/// Set the current log callback (called by generated wrapper code).
/// Returns the previous callback so it can be restored.
#[doc(hidden)]
pub fn set_log_callback(callback: Option<LogCallback>) -> Option<LogCallback> {
    CURRENT_LOG_CALLBACK.with(|c| c.replace(callback))
}

/// Set the current host callback (called by generated wrapper code).
/// Returns the previous callback so it can be restored.
#[doc(hidden)]
pub fn set_host_callback(callback: Option<HostCallback>) -> Option<HostCallback> {
    CURRENT_HOST_CALLBACK.with(|c| c.replace(callback))
}

/// Log a message at the specified level.
/// This sends the message to the host via the callback if one is set.
pub fn log(level: LogLevel, message: &str) {
    CURRENT_LOG_CALLBACK.with(|c| {
        if let Some(callback) = c.get() {
            callback(level, message.as_ptr(), message.len());
        }
    });
}

/// Log a trace message
pub fn trace(message: &str) {
    log(LogLevel::Trace, message);
}

/// Log a debug message
pub fn debug(message: &str) {
    log(LogLevel::Debug, message);
}

/// Log an info message
pub fn info(message: &str) {
    log(LogLevel::Info, message);
}

/// Log a warning message
pub fn warn(message: &str) {
    log(LogLevel::Warn, message);
}

/// Log an error message
pub fn error(message: &str) {
    log(LogLevel::Error, message);
}

/// Log a message with formatting (like println!)
#[macro_export]
macro_rules! log {
    ($level:expr, $($arg:tt)*) => {{
        $crate::log($level, &format!($($arg)*));
    }};
}

/// Log an info message with formatting
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {{
        $crate::info(&format!($($arg)*));
    }};
}

/// Log a warning message with formatting
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {{
        $crate::warn(&format!($($arg)*));
    }};
}

/// Log an error message with formatting
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {{
        $crate::error(&format!($($arg)*));
    }};
}

// ============================================================================
// Plugin-side host service calls
// ============================================================================

/// Error from calling a host service.
#[derive(Debug)]
pub enum HostCallError {
    /// No host callback is set (host doesn't support callbacks)
    NoCallback,
    /// Unknown service ID
    UnknownService,
    /// Failed to serialize input
    SerializeError,
    /// Failed to deserialize output
    DeserializeError,
    /// Output buffer too small (internal error, should retry)
    BufferTooSmall,
    /// Service returned an error
    ServiceError,
}

impl std::fmt::Display for HostCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostCallError::NoCallback => write!(f, "no host callback available"),
            HostCallError::UnknownService => write!(f, "unknown host service"),
            HostCallError::SerializeError => write!(f, "failed to serialize input"),
            HostCallError::DeserializeError => write!(f, "failed to deserialize output"),
            HostCallError::BufferTooSmall => write!(f, "output buffer too small"),
            HostCallError::ServiceError => write!(f, "host service error"),
        }
    }
}

impl std::error::Error for HostCallError {}

/// Call a host service with typed input and output.
///
/// This is the primary API for plugins to call back into the host.
///
/// # Example
/// ```rust,ignore
/// use plugcard::{call_host, host_services};
///
/// #[derive(Facet)]
/// struct HighlightRequest { code: String, language: String }
///
/// #[derive(Facet)]
/// struct HighlightResult { html: String, highlighted: bool }
///
/// let result: HighlightResult = call_host(
///     host_services::HIGHLIGHT_CODE,
///     &HighlightRequest { code: "fn main() {}".into(), language: "rust".into() }
/// )?;
/// ```
pub fn call_host<I, O>(service_id: u32, input: &I) -> Result<O, HostCallError>
where
    I: Facet<'static>,
    O: Facet<'static>,
{
    let callback = CURRENT_HOST_CALLBACK.with(|c| c.get());
    let Some(callback) = callback else {
        return Err(HostCallError::NoCallback);
    };

    // Serialize input
    let input_bytes = facet_postcard::to_vec(input).map_err(|_| HostCallError::SerializeError)?;

    // Start with a reasonable buffer, grow if needed
    let mut output = vec![0u8; 64 * 1024]; // 64KB initial

    loop {
        let mut data = HostCallData {
            service_id,
            input_ptr: input_bytes.as_ptr(),
            input_len: input_bytes.len(),
            output_ptr: output.as_mut_ptr(),
            output_cap: output.len(),
            output_len: 0,
            result: HostCallResult::Success,
        };

        callback(&mut data);

        match data.result {
            HostCallResult::Success => {
                output.truncate(data.output_len);
                return facet_postcard::from_bytes(&output)
                    .map_err(|_| HostCallError::DeserializeError);
            }
            HostCallResult::BufferTooSmall => {
                // Double buffer and retry
                if output.len() >= 256 * 1024 * 1024 {
                    // 256MB limit
                    return Err(HostCallError::BufferTooSmall);
                }
                output.resize(output.len() * 2, 0);
                continue;
            }
            HostCallResult::UnknownService => return Err(HostCallError::UnknownService),
            HostCallResult::DeserializeError => return Err(HostCallError::DeserializeError),
            HostCallResult::ServiceError => return Err(HostCallError::ServiceError),
        }
    }
}

/// Check if host callbacks are available.
///
/// Returns true if a host callback is set, meaning `call_host` can be used.
pub fn has_host_callback() -> bool {
    CURRENT_HOST_CALLBACK.with(|c| c.get().is_some())
}

// Re-exports for macro use
pub use facet;
pub use facet_postcard;
pub use libloading;
pub use linkme;

// Re-export the proc macro
pub use plugcard_macros::plugcard;

// Host-side plugin loading
mod loader;
pub use loader::{LoadError, Plugin, PluginMethod};

/// Export plugin entry points. Call this once in your plugin's lib.rs.
///
/// ```rust,ignore
/// plugcard::export_plugin!();
/// ```
#[macro_export]
macro_rules! export_plugin {
    () => {
        /// Returns the ABI version this plugin was built with
        #[unsafe(no_mangle)]
        pub extern "C" fn __plugcard_abi_version() -> u32 {
            $crate::ABI_VERSION
        }

        /// Returns pointer to the methods array
        #[unsafe(no_mangle)]
        pub extern "C" fn __plugcard_methods_ptr() -> *const $crate::MethodSignature {
            $crate::list_methods().as_ptr()
        }

        /// Returns length of the methods array
        #[unsafe(no_mangle)]
        pub extern "C" fn __plugcard_methods_len() -> usize {
            $crate::list_methods().len()
        }

        /// Dispatch a method call
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn __plugcard_dispatch(data: *mut $crate::MethodCallData) {
            unsafe { $crate::dispatch(data) }
        }
    };
}
