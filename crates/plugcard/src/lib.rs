//! Plugcard: Dynamic library plugin system with serialized method calls
//!
//! Inspired by postcard-rpc, this provides a way to load plugins as dynamic
//! libraries and call methods using serialized data across the FFI boundary.

use linkme::distributed_slice;

/// Distributed slice containing all registered method signatures
#[distributed_slice]
pub static METHODS: [MethodSignature];

/// A method signature with schema information for introspection
#[derive(Debug, Clone)]
pub struct MethodSignature {
    /// Unique key derived from method name and type schemas
    pub key: u64,
    /// Human-readable method name
    pub name: &'static str,
    /// Schema for the input type
    pub input_schema: &'static postcard_schema::schema::NamedType,
    /// Schema for the output type
    pub output_schema: &'static postcard_schema::schema::NamedType,
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

/// Compute a method key from name and schemas (const-compatible FNV-1a hash)
pub const fn compute_key(name: &str, input_schema: &postcard_schema::schema::NamedType, output_schema: &postcard_schema::schema::NamedType) -> u64 {
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

    // Hash input schema name
    let input_bytes = input_schema.name.as_bytes();
    let mut i = 0;
    while i < input_bytes.len() {
        hash ^= input_bytes[i] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }

    // Hash output schema name
    let output_bytes = output_schema.name.as_bytes();
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

// Re-exports for macro use
pub use linkme;
pub use postcard;
pub use postcard_schema;
pub use serde;

// Re-export the proc macro
pub use plugcard_macros::plugcard;
