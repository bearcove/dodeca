//! Proc macros for plugcard plugin system
//!
//! Provides the `#[plugcard]` attribute macro for exposing functions as plugin methods.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use unsynn::*;

// ============================================================================
// GRAMMAR DEFINITIONS (private to this crate)
// ============================================================================

keyword! {
    KFn = "fn"
}
keyword! {
    KPub = "pub"
}
keyword! {
    KCrate = "crate"
}
keyword! {
    KIn = "in"
}

operator! {
    RArrow = "->"
}
operator! {
    HashSign = "#"
}

type ModPath = Cons<Option<PathSep>, PathSepDelimited<Ident>>;
type VerbatimUntil<C> = Many<Cons<Except<C>, TokenTree>>;

// Outer attribute: #[...]
unsynn! {
    struct OuterAttr {
        _hash: HashSign,
        _content: BracketGroup,
    }

    enum Vis {
        PubIn(Cons<KPub, ParenthesisGroupContaining<Cons<Option<KIn>, ModPath>>>),
        Pub(KPub),
    }

    struct FnArg {
        name: Ident,
        _colon: Colon,
        ty: VerbatimUntil<Comma>,
    }

    struct ReturnType {
        _arrow: RArrow,
        ty: VerbatimUntil<BraceGroup>,
    }

    struct Function {
        attrs: Vec<OuterAttr>,
        vis: Option<Vis>,
        _fn: KFn,
        name: Ident,
        args: ParenthesisGroupContaining<CommaDelimitedVec<FnArg>>,
        ret: Option<ReturnType>,
        body: BraceGroup,
    }
}

// ============================================================================
// MACRO IMPLEMENTATION
// ============================================================================

/// Mark a function as a plugcard method
///
/// The function must have arguments and return type that implement
/// `Serialize`, `Deserialize`, and `Schema`.
///
/// # Example
/// ```ignore
/// #[plugcard]
/// pub fn add(a: i32, b: i32) -> i32 {
///     a + b
/// }
/// ```
#[proc_macro_attribute]
pub fn plugcard(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item2: proc_macro2::TokenStream = item.clone().into();

    match plugcard_impl(item2.clone()) {
        Ok(output) => output.into(),
        Err(err) => {
            let err_msg = err.to_string();
            quote! {
                compile_error!(#err_msg);
                #item2
            }
            .into()
        }
    }
}

fn plugcard_impl(item: proc_macro2::TokenStream) -> std::result::Result<proc_macro2::TokenStream, String> {
    // Parse the function
    let mut iter = item.clone().to_token_iter();
    let func: Cons<Function, EndOfStream> = iter
        .parse()
        .map_err(|e| format!("Failed to parse function: {e}"))?;
    let func = func.first;

    let fn_name = &func.name;
    let fn_body = func.body.to_token_stream();

    // Extract arguments
    let args: Vec<_> = func
        .args
        .content
        .iter()
        .map(|d| {
            let arg = &d.value;
            let name = &arg.name;
            // Collect type tokens
            let ty_tokens: proc_macro2::TokenStream = arg
                .ty
                .iter()
                .map(|t| t.value.second.clone())
                .collect();
            (name.clone(), ty_tokens)
        })
        .collect();

    // Extract return type
    let return_type: proc_macro2::TokenStream = if let Some(ret) = &func.ret {
        ret.ty.iter().map(|t| t.value.second.clone()).collect()
    } else {
        quote! { () }
    };

    // Generate names
    let wrapper_name = format_ident!("__plugcard_wrapper_{}", fn_name);
    let input_type_name = format_ident!("__PlugcardInput_{}", fn_name);
    let sig_name = format_ident!("__PLUGCARD_SIG_{}", fn_name);
    let method_name_str = fn_name.to_string();

    // Generate input struct fields
    let input_fields: Vec<_> = args
        .iter()
        .map(|(name, ty)| {
            quote! { pub #name: #ty }
        })
        .collect();

    // Generate argument list for original function
    let arg_names: Vec<_> = args.iter().map(|(name, _)| name.clone()).collect();
    let arg_types: Vec<_> = args.iter().map(|(_, ty)| ty.clone()).collect();

    // Generate visibility
    let vis = if func.vis.is_some() {
        quote! { pub }
    } else {
        quote! {}
    };

    let output = quote! {
        // Original function (unchanged)
        #vis fn #fn_name(#(#arg_names: #arg_types),*) -> #return_type
        #fn_body

        // Input composite type
        #[derive(::plugcard::serde::Serialize, ::plugcard::serde::Deserialize, ::plugcard::postcard_schema::Schema)]
        #[serde(crate = "::plugcard::serde")]
        #[allow(non_camel_case_types)]
        struct #input_type_name {
            #(#input_fields),*
        }

        // C-compatible wrapper
        #[allow(non_snake_case)]
        unsafe extern "C" fn #wrapper_name(data: *mut ::plugcard::MethodCallData) {
            unsafe {
                let data = &mut *data;

                // Deserialize input
                let input_slice = ::core::slice::from_raw_parts(data.input_ptr, data.input_len);
                let input: #input_type_name = match ::plugcard::postcard::from_bytes(input_slice) {
                    Ok(v) => v,
                    Err(_) => {
                        data.result = ::plugcard::MethodCallResult::DeserializeError;
                        return;
                    }
                };

                // Call the actual function
                let result = #fn_name(#(input.#arg_names),*);

                // Serialize output
                let output_slice = ::core::slice::from_raw_parts_mut(data.output_ptr, data.output_cap);
                match ::plugcard::postcard::to_slice(&result, output_slice) {
                    Ok(written) => {
                        data.output_len = written.len();
                        data.result = ::plugcard::MethodCallResult::Success;
                    }
                    Err(_) => {
                        data.result = ::plugcard::MethodCallResult::SerializeError;
                    }
                }
            }
        }

        // Register in distributed slice
        #[::plugcard::linkme::distributed_slice(::plugcard::METHODS)]
        #[allow(non_upper_case_globals)]
        static #sig_name: ::plugcard::MethodSignature = ::plugcard::MethodSignature {
            key: ::plugcard::compute_key(
                #method_name_str,
                <#input_type_name as ::plugcard::postcard_schema::Schema>::SCHEMA,
                <#return_type as ::plugcard::postcard_schema::Schema>::SCHEMA,
            ),
            name: #method_name_str,
            input_schema: <#input_type_name as ::plugcard::postcard_schema::Schema>::SCHEMA,
            output_schema: <#return_type as ::plugcard::postcard_schema::Schema>::SCHEMA,
            call: #wrapper_name,
        };
    };

    Ok(output)
}
