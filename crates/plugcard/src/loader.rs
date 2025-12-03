//! Host-side plugin loading via libloading.

use crate::{MethodCallData, MethodCallResult, MethodSignature};
use facet::Facet;
use libloading::{Library, Symbol};
use std::path::Path;

/// A loaded plugin.
pub struct Plugin {
    #[allow(dead_code)] // Kept alive to maintain library loaded
    library: Library,
    methods: &'static [MethodSignature],
    dispatch: unsafe extern "C" fn(*mut MethodCallData),
}

/// A reference to a method within a plugin.
#[derive(Debug, Clone, Copy)]
pub struct PluginMethod {
    /// Method key for dispatch
    pub key: u64,
    /// Human-readable name
    pub name: &'static str,
}

impl Plugin {
    /// Load a plugin from a dynamic library path.
    ///
    /// # Safety
    /// The plugin must be a valid plugcard plugin built with `export_plugin!()`.
    pub unsafe fn load(path: impl AsRef<Path>) -> Result<Self, libloading::Error> {
        let library = unsafe { Library::new(path.as_ref()) }?;

        // Get the methods slice
        let methods_ptr: Symbol<extern "C" fn() -> *const MethodSignature> =
            unsafe { library.get(b"__plugcard_methods_ptr")? };
        let methods_len: Symbol<extern "C" fn() -> usize> =
            unsafe { library.get(b"__plugcard_methods_len")? };

        let ptr = methods_ptr();
        let len = methods_len();
        let methods = unsafe { std::slice::from_raw_parts(ptr, len) };

        // Get the dispatch function
        let dispatch: Symbol<unsafe extern "C" fn(*mut MethodCallData)> =
            unsafe { library.get(b"__plugcard_dispatch")? };
        let dispatch = *dispatch;

        Ok(Plugin {
            library,
            methods,
            dispatch,
        })
    }

    /// List all methods exported by this plugin.
    pub fn methods(&self) -> impl Iterator<Item = PluginMethod> + '_ {
        self.methods.iter().map(|m| PluginMethod {
            key: m.key,
            name: m.name,
        })
    }

    /// Find a method by name.
    pub fn find_method(&self, name: &str) -> Option<PluginMethod> {
        self.methods
            .iter()
            .find(|m| m.name == name)
            .map(|m| PluginMethod {
                key: m.key,
                name: m.name,
            })
    }

    /// Call a method with serialized input, returning serialized output.
    ///
    /// This is the low-level interface. For ergonomic use, see `call()`.
    pub fn call_raw(&self, key: u64, input: &[u8]) -> Result<Vec<u8>, CallError> {
        // Start with a reasonable buffer, grow if needed
        let mut output = vec![0u8; 64 * 1024]; // 64KB initial

        loop {
            let mut data = MethodCallData {
                key,
                input_ptr: input.as_ptr(),
                input_len: input.len(),
                output_ptr: output.as_mut_ptr(),
                output_cap: output.len(),
                output_len: 0,
                result: MethodCallResult::Success,
            };

            unsafe { (self.dispatch)(&mut data) };

            match data.result {
                MethodCallResult::Success => {
                    output.truncate(data.output_len);
                    return Ok(output);
                }
                MethodCallResult::SerializeError => {
                    // Output buffer too small, double it and retry
                    if output.len() >= 256 * 1024 * 1024 {
                        // 256MB limit
                        return Err(CallError::OutputTooLarge);
                    }
                    output.resize(output.len() * 2, 0);
                    continue;
                }
                MethodCallResult::DeserializeError => return Err(CallError::DeserializeError),
                MethodCallResult::MethodError => {
                    output.truncate(data.output_len);
                    return Err(CallError::MethodError(output));
                }
                MethodCallResult::UnknownMethod => return Err(CallError::UnknownMethod),
            }
        }
    }

    /// Call a method with typed input and output.
    ///
    /// ```rust,ignore
    /// let result: String = plugin.call("greet", &"World".to_string())?;
    /// ```
    pub fn call<'a, I, O>(&self, name: &str, input: &'a I) -> Result<O, CallError>
    where
        I: Facet<'a>,
        O: Facet<'static>,
    {
        let method = self.find_method(name).ok_or(CallError::UnknownMethod)?;

        let input_bytes =
            crate::facet_postcard::to_vec(input).map_err(|_| CallError::SerializeError)?;

        let output_bytes = self.call_raw(method.key, &input_bytes)?;

        crate::facet_postcard::from_bytes(&output_bytes).map_err(|_| CallError::DeserializeError)
    }
}

/// Error from calling a plugin method.
#[derive(Debug)]
pub enum CallError {
    /// Failed to serialize input
    SerializeError,
    /// Failed to deserialize input in plugin
    DeserializeError,
    /// Method returned an error (contains serialized error)
    MethodError(Vec<u8>),
    /// Method not found
    UnknownMethod,
    /// Output exceeded size limit
    OutputTooLarge,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::SerializeError => write!(f, "failed to serialize input"),
            CallError::DeserializeError => write!(f, "failed to deserialize"),
            CallError::MethodError(data) => {
                // Try to deserialize as String for nice error messages
                if let Ok(msg) = crate::facet_postcard::from_bytes::<String>(data) {
                    write!(f, "method error: {msg}")
                } else {
                    write!(f, "method error: {} bytes", data.len())
                }
            }
            CallError::UnknownMethod => write!(f, "unknown method"),
            CallError::OutputTooLarge => write!(f, "output exceeded 256MB limit"),
        }
    }
}

impl std::error::Error for CallError {}
