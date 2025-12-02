//! Dodeca baseline plugin - simple transforms for testing the plugin system

use plugcard::plugcard;

/// Reverse a string
#[plugcard]
pub fn reverse_string(input: String) -> String {
    input.chars().rev().collect()
}

/// Add two numbers
#[plugcard]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use plugcard::{list_methods, MethodCallData, MethodCallResult};

    #[test]
    fn test_methods_registered() {
        let methods = list_methods();
        assert!(methods.len() >= 2, "Expected at least 2 methods, got {}", methods.len());

        let names: Vec<_> = methods.iter().map(|m| m.name).collect();
        assert!(names.contains(&"reverse_string"), "Missing reverse_string");
        assert!(names.contains(&"add"), "Missing add");
    }

    #[test]
    fn test_reverse_string_dispatch() {
        let methods = list_methods();
        let reverse = methods.iter().find(|m| m.name == "reverse_string").unwrap();

        // Serialize input: { input: "hello" }
        #[derive(plugcard::serde::Serialize)]
        #[serde(crate = "plugcard::serde")]
        struct Input { input: String }

        let input = Input { input: "hello".to_string() };
        let input_bytes = plugcard::postcard::to_allocvec(&input).unwrap();

        let mut output_buf = [0u8; 256];
        let mut data = MethodCallData {
            key: reverse.key,
            input_ptr: input_bytes.as_ptr(),
            input_len: input_bytes.len(),
            output_ptr: output_buf.as_mut_ptr(),
            output_cap: output_buf.len(),
            output_len: 0,
            result: MethodCallResult::default(),
        };

        // Call the method
        unsafe { (reverse.call)(&mut data) };

        assert_eq!(data.result, MethodCallResult::Success);

        // Deserialize output
        let output: String = plugcard::postcard::from_bytes(&output_buf[..data.output_len]).unwrap();
        assert_eq!(output, "olleh");
    }

    #[test]
    fn test_add_dispatch() {
        let methods = list_methods();
        let add_method = methods.iter().find(|m| m.name == "add").unwrap();

        // Serialize input: { a: 2, b: 3 }
        #[derive(plugcard::serde::Serialize)]
        #[serde(crate = "plugcard::serde")]
        struct Input { a: i32, b: i32 }

        let input = Input { a: 2, b: 3 };
        let input_bytes = plugcard::postcard::to_allocvec(&input).unwrap();

        let mut output_buf = [0u8; 256];
        let mut data = MethodCallData {
            key: add_method.key,
            input_ptr: input_bytes.as_ptr(),
            input_len: input_bytes.len(),
            output_ptr: output_buf.as_mut_ptr(),
            output_cap: output_buf.len(),
            output_len: 0,
            result: MethodCallResult::default(),
        };

        // Call the method
        unsafe { (add_method.call)(&mut data) };

        assert_eq!(data.result, MethodCallResult::Success);

        // Deserialize output
        let output: i32 = plugcard::postcard::from_bytes(&output_buf[..data.output_len]).unwrap();
        assert_eq!(output, 5);
    }
}
