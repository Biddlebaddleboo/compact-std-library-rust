//! Procedural macros for packed compact layouts and cage-backed deserialization.

extern crate proc_macro;

mod compact;
mod compact_deserialize;

use proc_macro::TokenStream;
use syn::{parse_macro_input, Item};

/// Generate a compact byte layout and checked accessors for a struct or
/// fieldless enum.
///
/// Supported struct fields are booleans, fixed-width integer scalars,
/// `String`, and fieldless enums also annotated with `#[compact]`. Add
/// `#[max = CONST_EXPR]` to a nonnegative integer field to pack its proven
/// `0..=max` range. `#[hot]` and `#[cold]` fields are placed in separate
/// packed sections. `#[compact(soa)]` additionally generates a
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

/// Derive direct cage-backed Serde deserialization for compact field types.
#[proc_macro_derive(CompactDeserialize, attributes(serde))]
pub fn compact_deserialize(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    compact_deserialize::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive the immutable frozen-value marker for a copyable, pointer-free type.
#[proc_macro_derive(FrozenValue)]
pub fn frozen_value(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    let name = &input.ident;
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return syn::Error::new_spanned(
            &input.generics,
            "FrozenValue derive does not support generic types",
        )
        .into_compile_error()
        .into();
    }
    if matches!(input.data, syn::Data::Union(_)) {
        return syn::Error::new_spanned(name, "FrozenValue cannot be derived for unions")
            .into_compile_error()
            .into();
    }
    let field_types: Vec<_> = match &input.data {
        syn::Data::Struct(data) => data.fields.iter().map(|field| &field.ty).collect(),
        syn::Data::Enum(data) => data
            .variants
            .iter()
            .flat_map(|variant| variant.fields.iter().map(|field| &field.ty))
            .collect(),
        syn::Data::Union(_) => unreachable!("unions were rejected above"),
    };
    let bounds = if field_types.is_empty() {
        quote::quote!()
    } else {
        quote::quote!(where #(#field_types: ::compact_std::__private::frozen::FrozenValue,)*)
    };
    quote::quote! {
        const _: () = assert!(
            ::core::mem::align_of::<#name>() <= 8,
            "FrozenValue types must have alignment at most eight"
        );
        unsafe impl ::compact_std::__private::frozen::FrozenValue for #name #bounds {}
    }
    .into()
}
