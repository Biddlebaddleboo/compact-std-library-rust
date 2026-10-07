//! Compact layout generation.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{
    Attribute, Expr, Field, Fields, Ident, ItemEnum, ItemStruct, Meta, Result, Token, Type,
    Visibility,
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Section {
    Main,
    Hot,
    Cold,
}

enum FieldKind {
    Bool,
    Integer { max: Option<Expr>, signed: bool },
    Enum,
    String,
}

struct FieldSpec {
    ident: Ident,
    ty: Type,
    kind: FieldKind,
    section: Section,
    bits_ident: Ident,
    offset_ident: Ident,
    width_expr: TokenStream,
}

pub(crate) fn expand_struct(attributes: TokenStream, mut item: ItemStruct) -> Result<TokenStream> {
    let soa = parse_compact_args(attributes)?;
    if !item.generics.params.is_empty() || item.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &item.generics,
            "generic #[compact] structs are not supported yet; use a concrete wrapper type",
        ));
    }
    let Fields::Named(named) = &mut item.fields else {
        return Err(syn::Error::new_spanned(
            &item.fields,
            "#[compact] requires a struct with named fields",
        ));
    };

    let mut specs = Vec::new();
    for field in &mut named.named {
        if field.attrs.iter().any(|attr| attr.path().is_ident("cfg")) {
            return Err(syn::Error::new_spanned(
                field,
                "conditional fields are not supported in #[compact] layouts",
            ));
        }
        let (max, section) = field_metadata(field)?;
        let ident = field.ident.clone().expect("named field");
        let kind = classify_field(&field.ty, max, &ident)?;
        let width_expr = match &kind {
            FieldKind::Bool => quote!(1u8),
            FieldKind::Integer {
                max: Some(maximum),
                signed: false,
            } => {
                let ty = &field.ty;
                quote!({
                    assert!((#maximum as u128) <= (<#ty>::MAX as u128), "#[max] exceeds the field type's maximum");
                    ::compact_std::__private::core::bits_required((#maximum) as u64)
                })
            }
            FieldKind::Integer {
                signed: _,
                max: None,
            } => {
                let bits = integer_bits(&field.ty).expect("classified integer");
                quote!(#bits as u8)
            }
            FieldKind::String => quote!(64u8),
            FieldKind::Enum => {
                let ty = &field.ty;
                quote!(< #ty as ::compact_std::__private::collections::CompactEnum >::BITS)
            }
            FieldKind::Integer {
                signed: true,
                max: Some(_),
            } => unreachable!("signed bounds are rejected by classify_field"),
        };
        let bits_ident = format_ident!("__COMPACT_{}_BITS", ident.to_string().to_uppercase());
        let offset_ident = format_ident!("{}_BIT_OFFSET", ident.to_string().to_uppercase());
        specs.push(FieldSpec {
            ident,
            ty: field.ty.clone(),
            kind,
            section,
            bits_ident,
            offset_ident,
            width_expr,
        });
    }

    for field in &mut named.named {
        if specs.iter().any(|spec| {
            &spec.ident == field.ident.as_ref().unwrap() && matches!(spec.kind, FieldKind::String)
        }) {
            field.ty = syn::parse_quote!(::std::string::String);
        }
    }

    let name = &item.ident;
    let compact_name = format_ident!("{}Compact", name);
    let visibility = &item.vis;
    let constants = layout_constants(&specs);
    let constructor_validations = specs.iter().filter_map(constructor_validation);
    let allocations = quote! {
        let __compact_main = arena.alloc_zeroed_bytes(#compact_name::<'arena>::MAIN_STORAGE_BYTES)?;
        let __compact_hot = arena.alloc_zeroed_bytes(#compact_name::<'arena>::HOT_STORAGE_BYTES)?;
        let __compact_cold = arena.alloc_zeroed_bytes(#compact_name::<'arena>::COLD_STORAGE_BYTES)?;
        let compact = #compact_name {
            main: __compact_main,
            hot: __compact_hot,
            cold: __compact_cold,
        };
    };
    let encoders = specs.iter().map(|spec| field_encoder(spec, &compact_name));
    let accessors = specs.iter().map(field_accessor);
    let setters = specs.iter().map(field_setter);
    let soa_items = if soa {
        Some(generate_soa(name, visibility, &specs)?)
    } else {
        None
    };
    let source_value_impl = if soa && derives_copy(&item.attrs) {
        quote! {
            #[allow(clippy::undocumented_unsafe_blocks)]
            unsafe impl ::compact_std::__private::core::CompactValue for #name {}
        }
    } else {
        TokenStream::new()
    };

    let original = quote!(#item);
    let soa_tokens = soa_items.unwrap_or_default();
    Ok(quote! {
        #original

        #[doc = "Arena-resident packed representation generated by `#[compact]`."]
        #[derive(Clone, Copy)]
        #visibility struct #compact_name<'arena> {
            main: ::compact_std::__private::core::ByteRange32<'arena>,
            hot: ::compact_std::__private::core::ByteRange32<'arena>,
            cold: ::compact_std::__private::core::ByteRange32<'arena>,
        }

        #[allow(clippy::undocumented_unsafe_blocks)]
        unsafe impl<'arena> ::compact_std::__private::core::CompactValue for #compact_name<'arena> {}

        impl<'arena> #compact_name<'arena> {
            #constants
        }

        impl #name {
            /// Encode this logical value into the compact arena layout.
            pub fn compact_in<'arena>(
                &self,
                arena: &mut ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<#compact_name<'arena>> {
                #(#constructor_validations)*
                #allocations
                #(#encoders)*
                Ok(compact)
            }
        }

        impl<'arena> #compact_name<'arena> {
            #(#accessors)*
            #(#setters)*
        }

        #soa_tokens
        #source_value_impl
    })
}

pub(crate) fn expand_enum(attributes: TokenStream, item: ItemEnum) -> Result<TokenStream> {
    if !attributes.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "#[compact] enum does not accept arguments",
        ));
    }
    if !item.generics.params.is_empty() || item.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &item.generics,
            "generic #[compact] enums are not supported yet",
        ));
    }
    for variant in &item.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                "#[compact] enum currently supports fieldless variants only",
            ));
        }
    }
    let name = &item.ident;
    let compact_name = format_ident!("{}Compact", name);
    let visibility = &item.vis;
    let variants: Vec<_> = item.variants.iter().map(|variant| &variant.ident).collect();
    let count = variants.len() as u64;
    let maximum = count.saturating_sub(1);
    let width = quote!(::compact_std::__private::core::bits_required(#maximum));
    let encode_arms: Vec<_> = variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let index = index as u64;
            quote!(#name::#variant => #index)
        })
        .collect();
    let decode_arms: Vec<_> = variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let index = index as u64;
            quote!(#index => Ok(#name::#variant))
        })
        .collect();
    let trait_encode_arms: Vec<_> = variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let index = index as u64;
            quote!(#name::#variant => #index)
        })
        .collect();
    let trait_decode_arms: Vec<_> = variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let index = index as u64;
            quote!(#index => Some(#name::#variant))
        })
        .collect();
    let constructor_write = if maximum == 0 {
        quote! {}
    } else {
        quote! {
            ::compact_std::__private::core::write_bits(
                arena.get_bytes_mut(range)?,
                0,
                #compact_name::<'arena>::DISCRIMINANT_BITS,
                encoded,
            )?;
        }
    };
    let setter_write = if maximum == 0 {
        quote! {}
    } else {
        quote! {
            ::compact_std::__private::core::write_bits(
                arena.get_bytes_mut(self.range)?,
                0,
                Self::DISCRIMINANT_BITS,
                encoded,
            )?;
        }
    };
    let read_expression = if maximum == 0 {
        quote!(0_u64)
    } else {
        quote!(::compact_std::__private::core::read_bits(
            arena.get_bytes(self.range)?,
            0,
            Self::DISCRIMINANT_BITS,
        )?)
    };
    let output = quote! {
        #item

        impl ::compact_std::__private::collections::CompactEnum for #name {
            const BITS: u8 = #width;

            fn compact_bits(&self) -> u64 {
                match self { #(#trait_encode_arms,)* }
            }

            fn from_compact_bits(value: u64) -> Option<Self> {
                match value { #(#trait_decode_arms,)* _ => None }
            }
        }

        #[doc = "Compact discriminant handle generated by `#[compact]`."]
        #[derive(Clone, Copy)]
        #visibility struct #compact_name<'arena> {
            range: ::compact_std::__private::core::ByteRange32<'arena>,
        }

        #[allow(clippy::undocumented_unsafe_blocks)]
        unsafe impl<'arena> ::compact_std::__private::core::CompactValue for #compact_name<'arena> {}

        impl<'arena> #compact_name<'arena> {
            /// Number of bits required by the enum discriminant.
            pub const DISCRIMINANT_BITS: u8 = #width;
            /// Number of payload bytes in the compact discriminant.
            pub const STORAGE_BYTES: usize = (Self::DISCRIMINANT_BITS as usize).div_ceil(8);
        }

        impl #name {
            /// Encode one enum value into compact arena storage.
            pub fn compact_in<'arena>(
                self,
                arena: &mut ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<#compact_name<'arena>> {
                let range = arena.alloc_zeroed_bytes(#compact_name::<'arena>::STORAGE_BYTES)?;
                let encoded = match self { #(#encode_arms,)* };
                #constructor_write
                Ok(#compact_name { range })
            }
        }

        impl<'arena> #compact_name<'arena> {
            /// Decode the compact discriminant, rejecting invalid bit patterns.
            pub fn get(
                &self,
                arena: &::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<#name> {
                let value = #read_expression;
                match value {
                    #(#decode_arms,)*
                    _ => Err(::compact_std::__private::collections::CollectionError::InvalidCompactValue),
                }
            }

            /// Replace this discriminant with another declared variant.
            pub fn set(
                &self,
                value: #name,
                arena: &mut ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<()> {
                let encoded = match value { #(#encode_arms,)* };
                #setter_write
                Ok(())
            }
        }
    };
    Ok(output)
}

fn derives_copy(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        if !attribute.path().is_ident("derive") {
            return false;
        }
        attribute
            .parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
            .map(|derives| derives.iter().any(|path| path.is_ident("Copy")))
            .unwrap_or(false)
    })
}

fn parse_compact_args(attributes: TokenStream) -> Result<bool> {
    let parser = Punctuated::<Ident, Token![,]>::parse_terminated;
    let args = parser.parse2(attributes)?;
    let mut soa = false;
    for argument in args {
        if argument == "soa" && !soa {
            soa = true;
        } else {
            return Err(syn::Error::new_spanned(
                argument,
                "the only supported #[compact(...)] option is `soa`",
            ));
        }
    }
    Ok(soa)
}

fn field_metadata(field: &mut Field) -> Result<(Option<Expr>, Section)> {
    let mut maximum = None;
    let mut section = Section::Main;
    let mut retained = Vec::new();
    for attribute in core::mem::take(&mut field.attrs) {
        if attribute.path().is_ident("max") {
            if maximum.is_some() {
                return Err(syn::Error::new_spanned(attribute, "duplicate #[max] bound"));
            }
            maximum = Some(parse_max(&attribute)?);
        } else if attribute.path().is_ident("hot") {
            if section != Section::Main {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "a field cannot have both #[hot] and #[cold]",
                ));
            }
            section = Section::Hot;
        } else if attribute.path().is_ident("cold") {
            if section != Section::Main {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "a field cannot have both #[hot] and #[cold]",
                ));
            }
            section = Section::Cold;
        } else {
            retained.push(attribute);
        }
    }
    field.attrs = retained;
    Ok((maximum, section))
}

fn parse_max(attribute: &Attribute) -> Result<Expr> {
    match &attribute.meta {
        Meta::NameValue(value) => Ok(value.value.clone()),
        _ => Err(syn::Error::new_spanned(
            attribute,
            "write a field bound as `#[max = CONST_EXPR]`",
        )),
    }
}

fn classify_field(ty: &Type, max: Option<Expr>, field: &Ident) -> Result<FieldKind> {
    let Type::Path(path) = ty else {
        return Err(syn::Error::new_spanned(
            ty,
            "unsupported compact field type; use bool, an integer scalar, or String",
        ));
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            ty,
            "unsupported compact field type",
        ));
    };
    let name = segment.ident.to_string();
    match name.as_str() {
        "bool" => {
            if max.is_some() {
                return Err(syn::Error::new_spanned(
                    field,
                    "bool fields do not use #[max]",
                ));
            }
            Ok(FieldKind::Bool)
        }
        "String" => {
            if max.is_some() {
                return Err(syn::Error::new_spanned(
                    field,
                    "String fields do not use #[max]",
                ));
            }
            Ok(FieldKind::String)
        }
        "u8" | "u16" | "u32" | "u64" | "usize" => Ok(FieldKind::Integer { max, signed: false }),
        "i8" | "i16" | "i32" | "i64" | "isize" => {
            if max.is_some() {
                return Err(syn::Error::new_spanned(
                    field,
                    "#[max] currently applies only to unsigned fields; signed bounds need an explicit minimum",
                ));
            }
            Ok(FieldKind::Integer {
                max: None,
                signed: true,
            })
        }
        _ if max.is_some() => Err(syn::Error::new_spanned(
            field,
            "#[max] requires an unsigned integer field",
        )),
        _ if path
            .path
            .segments
            .iter()
            .all(|segment| matches!(segment.arguments, syn::PathArguments::None)) =>
        {
            Ok(FieldKind::Enum)
        }
        _ => Err(syn::Error::new_spanned(
            ty,
            "unsupported compact field type; use a scalar, String, or a #[compact] fieldless enum",
        )),
    }
}

fn integer_bits(ty: &Type) -> Option<TokenStream> {
    let Type::Path(path) = ty else { return None };
    let ident = &path.path.segments.last()?.ident;
    Some(quote!(#ident::BITS))
}

fn layout_constants(specs: &[FieldSpec]) -> TokenStream {
    let width_constants = specs.iter().map(|field| {
        let ident = &field.bits_ident;
        let expression = &field.width_expr;
        quote!(#[doc(hidden)] pub const #ident: u8 = #expression;)
    });
    let offsets = specs.iter().enumerate().map(|(index, field)| {
        let name = &field.offset_ident;
        let prior: Vec<_> = specs[..index]
            .iter()
            .filter(|previous| previous.section == field.section)
            .map(|previous| {
                let bits = &previous.bits_ident;
                quote!(Self::#bits as usize)
            })
            .collect();
        let offset = quote!(0usize #(+ #prior)*);
        quote!(pub const #name: usize = #offset;)
    });
    let main_widths: Vec<_> = specs
        .iter()
        .filter(|field| field.section == Section::Main)
        .map(|field| &field.bits_ident)
        .collect();
    let hot_widths: Vec<_> = specs
        .iter()
        .filter(|field| field.section == Section::Hot)
        .map(|field| &field.bits_ident)
        .collect();
    let cold_widths: Vec<_> = specs
        .iter()
        .filter(|field| field.section == Section::Cold)
        .map(|field| &field.bits_ident)
        .collect();
    let main_bits = bits_sum(&main_widths);
    let hot_bits = bits_sum(&hot_widths);
    let cold_bits = bits_sum(&cold_widths);
    quote! {
        #(#width_constants)*
        #(#offsets)*
        /// Number of bytes used by ordinary compact fields.
        pub const MAIN_STORAGE_BYTES: usize = (#main_bits).div_ceil(8);
        /// Number of bytes used by explicitly hot fields.
        pub const HOT_STORAGE_BYTES: usize = (#hot_bits).div_ceil(8);
        /// Number of bytes used by explicitly cold fields.
        pub const COLD_STORAGE_BYTES: usize = (#cold_bits).div_ceil(8);
        /// Total bytes across this object's packed field sections.
        pub const STORAGE_BYTES: usize =
            Self::MAIN_STORAGE_BYTES + Self::HOT_STORAGE_BYTES + Self::COLD_STORAGE_BYTES;
    }
}

fn bits_sum(widths: &[&Ident]) -> TokenStream {
    quote!(0usize #(+ (Self::#widths as usize))*)
}

fn constructor_validation(spec: &FieldSpec) -> Option<TokenStream> {
    let FieldKind::Integer {
        max: Some(maximum),
        signed: false,
    } = &spec.kind
    else {
        return None;
    };
    let field = &spec.ident;
    Some(quote! {
        if (self.#field as u64) > ((#maximum) as u64) {
            return Err(::compact_std::__private::collections::CollectionError::Core(
                ::compact_std::__private::core::Error::ValueDoesNotFit,
            ));
        }
    })
}

fn segment_ident(section: Section) -> Ident {
    Ident::new(
        match section {
            Section::Main => "main",
            Section::Hot => "hot",
            Section::Cold => "cold",
        },
        Span::call_site(),
    )
}

fn field_encoder(spec: &FieldSpec, compact_name: &Ident) -> TokenStream {
    let field = &spec.ident;
    let segment = segment_ident(spec.section);
    let offset = &spec.offset_ident;
    let bits = &spec.bits_ident;
    match &spec.kind {
        FieldKind::Bool => quote! {
            ::compact_std::__private::core::write_bits(
                arena.get_bytes_mut(compact.#segment)?,
                #compact_name::<'arena>::#offset,
                #compact_name::<'arena>::#bits,
                self.#field as u64,
            )?;
        },
        FieldKind::Integer { signed: true, .. } => quote! {
            let mut __compact_value = self.#field as u64;
            if #compact_name::<'arena>::#bits < 64 {
                __compact_value &= (1_u64 << #compact_name::<'arena>::#bits) - 1;
            }
            ::compact_std::__private::core::write_bits(
                arena.get_bytes_mut(compact.#segment)?,
                #compact_name::<'arena>::#offset,
                #compact_name::<'arena>::#bits,
                __compact_value,
            )?;
        },
        FieldKind::Integer { signed: false, .. } => quote! {
            ::compact_std::__private::core::write_bits(
                arena.get_bytes_mut(compact.#segment)?,
                #compact_name::<'arena>::#offset,
                #compact_name::<'arena>::#bits,
                self.#field as u64,
            )?;
        },
        FieldKind::Enum => quote! {
            let __compact_value =
                ::compact_std::__private::collections::CompactEnum::compact_bits(&self.#field);
            ::compact_std::__private::core::write_bits(
                arena.get_bytes_mut(compact.#segment)?,
                #compact_name::<'arena>::#offset,
                #compact_name::<'arena>::#bits,
                __compact_value,
            )?;
        },
        FieldKind::String => quote! {
            let __compact_payload = arena.alloc_bytes(self.#field.as_bytes())?;
            let __compact_encoded = (__compact_payload.offset() as u64)
                | ((__compact_payload.len() as u64) << 32);
            ::compact_std::__private::core::write_bits(
                arena.get_bytes_mut(compact.#segment)?,
                #compact_name::<'arena>::#offset,
                #compact_name::<'arena>::#bits,
                __compact_encoded,
            )?;
        },
    }
}

fn field_accessor(spec: &FieldSpec) -> TokenStream {
    let field = &spec.ident;
    let ty = &spec.ty;
    let segment = segment_ident(spec.section);
    let offset = &spec.offset_ident;
    let bits = &spec.bits_ident;
    match &spec.kind {
        FieldKind::Bool => quote! {
            /// Read this packed boolean field.
            pub fn #field<'view>(
                &self,
                arena: &'view ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<bool> {
                let value = ::compact_std::__private::core::read_bits(
                    arena.get_bytes(self.#segment)?,
                    Self::#offset,
                    Self::#bits,
                )?;
                Ok(value != 0)
            }
        },
        FieldKind::Integer { .. } => quote! {
            /// Read this compact integer field as its declared logical type.
            pub fn #field<'view>(
                &self,
                arena: &'view ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<#ty> {
                let value = ::compact_std::__private::core::read_bits(
                    arena.get_bytes(self.#segment)?,
                    Self::#offset,
                    Self::#bits,
                )?;
                Ok(value as #ty)
            }
        },
        FieldKind::Enum => quote! {
            /// Decode this compact fieldless enum discriminant.
            pub fn #field<'view>(
                &self,
                arena: &'view ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<#ty> {
                let value = ::compact_std::__private::core::read_bits(
                    arena.get_bytes(self.#segment)?,
                    Self::#offset,
                    Self::#bits,
                )?;
                <#ty as ::compact_std::__private::collections::CompactEnum>::from_compact_bits(value)
                    .ok_or(::compact_std::__private::collections::CollectionError::InvalidCompactValue)
            }
        },
        FieldKind::String => quote! {
            /// Borrow this compact UTF-8 field without copying its payload.
            pub fn #field<'view>(
                &self,
                arena: &'view ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<&'view str> {
                let encoded = ::compact_std::__private::core::read_bits(
                    arena.get_bytes(self.#segment)?,
                    Self::#offset,
                    Self::#bits,
                )?;
                let len = (encoded >> 32) as u32;
                if len == 0 {
                    return Ok("");
                }
                let offset = encoded as u32;
                // SAFETY: compact_in and set_* store only ranges produced by
                // arena.alloc_bytes; the record range is private to this API.
                let bytes = unsafe {
                    ::compact_std::__private::core::ByteRange32::from_raw_parts_unchecked(offset, len)
                };
                ::core::str::from_utf8(arena.get_bytes(bytes)?)
                    .map_err(|_| ::compact_std::__private::collections::CollectionError::InvalidUtf8)
            }
        },
    }
}

fn field_setter(spec: &FieldSpec) -> TokenStream {
    let field = &spec.ident;
    let setter = format_ident!("set_{}", field);
    let ty = &spec.ty;
    let segment = segment_ident(spec.section);
    let offset = &spec.offset_ident;
    let bits = &spec.bits_ident;
    match &spec.kind {
        FieldKind::Bool => quote! {
            /// Update this boolean while preserving neighboring packed bits.
            pub fn #setter(
                &self,
                value: bool,
                arena: &mut ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<()> {
                ::compact_std::__private::core::write_bits(
                    arena.get_bytes_mut(self.#segment)?,
                    Self::#offset,
                    Self::#bits,
                    value as u64,
                )?;
                Ok(())
            }
        },
        FieldKind::Integer {
            max: Some(maximum),
            signed: false,
        } => quote! {
            /// Update this bounded field, returning an error rather than truncating.
            pub fn #setter(
                &self,
                value: #ty,
                arena: &mut ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<()> {
                if (value as u64) > ((#maximum) as u64) {
                    return Err(::compact_std::__private::collections::CollectionError::Core(
                        ::compact_std::__private::core::Error::ValueDoesNotFit,
                    ));
                }
                ::compact_std::__private::core::write_bits(
                    arena.get_bytes_mut(self.#segment)?,
                    Self::#offset,
                    Self::#bits,
                    value as u64,
                )?;
                Ok(())
            }
        },
        FieldKind::Integer { signed: true, .. } => quote! {
            /// Update this integer field.
            pub fn #setter(
                &self,
                value: #ty,
                arena: &mut ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<()> {
                let mut encoded = value as u64;
                if Self::#bits < 64 {
                    encoded &= (1_u64 << Self::#bits) - 1;
                }
                ::compact_std::__private::core::write_bits(
                    arena.get_bytes_mut(self.#segment)?,
                    Self::#offset,
                    Self::#bits,
                    encoded,
                )?;
                Ok(())
            }
        },
        FieldKind::Integer {
            signed: false,
            max: None,
        } => quote! {
            /// Update this integer field.
            pub fn #setter(
                &self,
                value: #ty,
                arena: &mut ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<()> {
                ::compact_std::__private::core::write_bits(
                    arena.get_bytes_mut(self.#segment)?,
                    Self::#offset,
                    Self::#bits,
                    value as u64,
                )?;
                Ok(())
            }
        },
        FieldKind::Enum => quote! {
            /// Update this compact fieldless enum discriminant.
            pub fn #setter(
                &self,
                value: #ty,
                arena: &mut ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<()> {
                let encoded =
                    ::compact_std::__private::collections::CompactEnum::compact_bits(&value);
                ::compact_std::__private::core::write_bits(
                    arena.get_bytes_mut(self.#segment)?,
                    Self::#offset,
                    Self::#bits,
                    encoded,
                )?;
                Ok(())
            }
        },
        FieldKind::String => quote! {
            /// Replace this UTF-8 field with a newly allocated compact payload.
            pub fn #setter(
                &self,
                value: &str,
                arena: &mut ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<()> {
                let payload = arena.alloc_bytes(value.as_bytes())?;
                let encoded = (payload.offset() as u64) | ((payload.len() as u64) << 32);
                ::compact_std::__private::core::write_bits(
                    arena.get_bytes_mut(self.#segment)?,
                    Self::#offset,
                    Self::#bits,
                    encoded,
                )?;
                Ok(())
            }
        },
    }
}

fn generate_soa(name: &Ident, visibility: &Visibility, specs: &[FieldSpec]) -> Result<TokenStream> {
    for spec in specs {
        if matches!(spec.kind, FieldKind::String | FieldKind::Enum) {
            return Err(syn::Error::new_spanned(
                &spec.ty,
                "#[compact(soa)] currently supports Copy scalar fields only; String/enum columns are unsupported",
            ));
        }
    }
    let soa_name = format_ident!("{}Soa", name);
    let columns: Vec<_> = specs
        .iter()
        .map(|spec| {
            let field = &spec.ident;
            let ty = &spec.ty;
            if matches!(spec.kind, FieldKind::Bool) {
                quote!(#field: ::compact_std::__private::collections::CompactBitVec<'arena>)
            } else {
                quote!(#field: ::compact_std::__private::collections::CompactVec<'arena, #ty>)
            }
        })
        .collect();
    let column_initializers: Vec<_> = specs
        .iter()
        .map(|spec| {
            let field = &spec.ident;
            if matches!(spec.kind, FieldKind::Bool) {
                quote!(#field: ::compact_std::__private::collections::CompactBitVec::new_in(arena))
            } else {
                quote!(#field: ::compact_std::__private::collections::CompactVec::new_in(arena))
            }
        })
        .collect();
    let capacity_initializers: Vec<_> = specs
        .iter()
        .map(|spec| {
            let field = &spec.ident;
            if matches!(spec.kind, FieldKind::Bool) {
                quote!(#field: ::compact_std::__private::collections::CompactBitVec::with_capacity_in(capacity, arena)?)
            } else {
                quote!(#field: ::compact_std::__private::collections::CompactVec::with_capacity_in(capacity, arena)?)
            }
        })
        .collect();
    let reserves = specs.iter().map(|spec| {
        let field = &spec.ident;
        quote!(self.#field.reserve_in(1, arena)?;)
    });
    let push_steps: Vec<_> = specs
        .iter()
        .map(|spec| {
            let field = &spec.ident;
            let rollback: Vec<_> = specs
                .iter()
                .map(|column| {
                    let name = &column.ident;
                    quote!(self.#name.truncate(old_len);)
                })
                .collect();
            quote! {
                if let Err(error) = self.#field.push_in(value.#field, arena) {
                    #(#rollback)*
                    return Err(error.into());
                }
            }
        })
        .collect();
    let getters = specs.iter().map(|spec| {
        let field = &spec.ident;
        if matches!(spec.kind, FieldKind::Bool) {
            quote! {
                #field: self.#field.get(index, arena)?
                    .ok_or(::compact_std::__private::collections::CollectionError::Core(
                        ::compact_std::__private::core::Error::OutOfBounds,
                    ))?,
            }
        } else {
            quote! {
                #field: *self.#field.get(index, arena)?
                    .ok_or(::compact_std::__private::collections::CollectionError::Core(
                        ::compact_std::__private::core::Error::OutOfBounds,
                    ))?,
            }
        }
    });
    Ok(quote! {
        /// Columnar arena collection generated by `#[compact(soa)]`.
        #visibility struct #soa_name<'arena> {
            #(#columns,)*
            len: u32,
        }

        impl<'arena> #soa_name<'arena> {
            /// Construct empty compact columns.
            pub fn new_in(arena: &::compact_std::__private::core::Arena<'arena, '_>) -> Self {
                Self { #(#column_initializers,)* len: 0 }
            }

            /// Construct compact columns with capacity reserved for `capacity` records.
            pub fn with_capacity_in(
                capacity: usize,
                arena: &mut ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<Self> {
                Ok(Self { #(#capacity_initializers,)* len: 0 })
            }

            /// Return the number of logical records.
            pub const fn len(&self) -> usize { self.len as usize }

            /// Return whether the column collection is empty.
            pub const fn is_empty(&self) -> bool { self.len == 0 }

            /// Append one logical record, packing boolean columns into bits.
            pub fn push_in(
                &mut self,
                value: #name,
                arena: &mut ::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<()> {
                let old_len = self.len();
                let next_len = old_len.checked_add(1)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or(::compact_std::__private::collections::CollectionError::CapacityOverflow)?;
                #(#reserves)*
                #(#push_steps)*
                self.len = next_len;
                Ok(())
            }

            /// Reconstruct one logical record by index.
            pub fn get(
                &self,
                index: usize,
                arena: &::compact_std::__private::core::Arena<'arena, '_>,
            ) -> ::compact_std::__private::collections::Result<Option<#name>> {
                if index >= self.len() { return Ok(None); }
                Ok(Some(#name { #(#getters)* }))
            }
        }
    })
}
