//! Procedural macros for compact arena layouts and lexical constructor
//! rewriting, preserving the documented V2.1.0 source contract.

extern crate proc_macro;

mod arena;
mod compact;
mod compact_deserialize;
mod compact_freeze;

use proc_macro::TokenStream;
use syn::{parse_macro_input, Item};

/// Generate a compact byte layout and checked accessors for a struct or
/// fieldless enum.
///
/// Supported struct fields are booleans, fixed-width integer scalars,
/// `String`, and fieldless enums also annotated with `#[compact]`. Add
/// `#[max = CONST_EXPR]` to a nonnegative integer field to pack its proven
/// `0..=max` range. `#[hot]` and `#[cold]` fields are placed in separate
/// arena allocations. `#[compact(soa)]` additionally generates a
/// primitive-column SoA collection.
///
/// Named, non-generic structs are the supported struct surface. Unsupported
/// pointers, references, generic layouts, payload enums, and unsupported
/// bounds produce compile errors.
#[proc_macro_attribute]
pub fn compact(attributes: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as Item);
    let result = match item {
        Item::Struct(item) => compact::expand_struct(attributes.into(), item),
        Item::Enum(item) => compact::expand_enum(attributes.into(), item),
        other => Err(syn::Error::new_spanned(
            other,
            "#[compact] supports structs and fieldless enums",
        )),
    };
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Derive direct arena-backed Serde deserialization for compact field types.
#[proc_macro_derive(CompactDeserialize, attributes(serde))]
pub fn compact_deserialize(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    compact_deserialize::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Generate an immutable frozen companion type for a named compact struct or
/// unit enum.
#[proc_macro_derive(CompactFreeze)]
pub fn compact_freeze(input: TokenStream) -> TokenStream {
    compact_freeze::expand(syn::parse_macro_input!(input as syn::DeriveInput))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Rewrite supported compact constructors and methods inside one lexical block.
///
/// The named arena must already be an `&mut compact_core::Arena`. Supported
/// constructors include `Vec::new`, `Vec::with_capacity`, `String::new`,
/// `String::from`, and `Box::new`; standard `format!` calls are rewritten to
/// fallible arena-backed formatting. Supported local compact methods receive
/// the arena argument at the rewritten call site.
///
/// Binding analysis is conservative. Helper-returned compact values may need an
/// explicit compact type annotation. Ambiguous receivers and moving or
/// mutating closure captures require explicit `*_in(..., arena)` APIs.
/// `vec!`, known compact-owner `.clone()`, compact-string `.to_string()`, and
/// explicitly typed compact `.collect()` calls are rewritten through the
/// arena-aware collection traits. Allocation-bearing rewrites propagate
/// errors with `?`. Allocation-bearing sugar inside closures must use explicit
/// arena-aware APIs so mutable arena capture stays visible.
#[proc_macro]
pub fn arena(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as arena::ArenaInput);
    arena::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
