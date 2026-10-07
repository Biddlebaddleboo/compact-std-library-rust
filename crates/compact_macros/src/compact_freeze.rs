//! Generate immutable frozen companions for compact structs and enums.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, GenericArgument, GenericParam, PathArguments, Result, Type};

pub(crate) fn expand(input: DeriveInput) -> Result<TokenStream> {
    match input.data {
        Data::Struct(data) => expand_struct(input.vis, input.ident, input.generics, data.fields),
        Data::Enum(data) => expand_enum(input.vis, input.ident, input.generics, data.variants),
        Data::Union(union) => Err(syn::Error::new_spanned(
            union.union_token,
            "CompactFreeze cannot be derived for unions",
        )),
    }
}

fn expand_struct(
    visibility: syn::Visibility,
    name: syn::Ident,
    generics: syn::Generics,
    fields: Fields,
) -> Result<TokenStream> {
    let Fields::Named(fields) = fields else {
        return Err(syn::Error::new_spanned(
            name,
            "CompactFreeze currently requires a struct with named fields",
        ));
    };
    let has_arena_lifetime = validate_generics(&generics)?;
    let frozen_name = format_ident!("{}Frozen", name);
    let source_type = if has_arena_lifetime {
        quote!(#name<'arena>)
    } else {
        quote!(#name)
    };

    let mut source_fields = Vec::new();
    let mut frozen_fields = Vec::new();
    let mut frozen_types = Vec::new();
    for field in fields.named {
        let ident = field.ident.expect("named field");
        let source_ty = field.ty;
        let frozen_ty = frozen_type(&source_ty)?;
        source_fields.push((ident.clone(), source_ty));
        frozen_fields.push((field.vis, ident, frozen_ty.clone()));
        frozen_types.push(frozen_ty);
    }

    let field_bounds = source_fields
        .iter()
        .zip(frozen_types.iter())
        .map(|((_, source_ty), frozen_ty)| {
            quote!(
                #source_ty: ::compact_std::__private::frozen::FreezeIn<
                    'arena,
                    Frozen = #frozen_ty,
                >
            )
        })
        .collect::<Vec<_>>();
    let output_fields = frozen_fields
        .iter()
        .map(|(visibility, ident, ty)| quote!(#visibility #ident: #ty,));
    let accessors = frozen_fields.iter().map(|(_, ident, ty)| {
        quote! {
            pub fn #ident(&self) -> #ty {
                self.#ident
            }
        }
    });
    let field_initializers = source_fields.iter().map(|(ident, ty)| {
        quote! {
            #ident: <#ty as ::compact_std::__private::frozen::FreezeIn<'arena>>::freeze_in(
                &self.#ident,
                source,
                destination,
            )?,
        }
    });
    let frozen_bounds = frozen_types
        .iter()
        .map(|ty| quote!(#ty: ::compact_std::__private::frozen::FrozenValue))
        .collect::<Vec<_>>();
    let frozen_where = if frozen_bounds.is_empty() {
        TokenStream::new()
    } else {
        quote!(where #(#frozen_bounds),*)
    };
    let field_where = if field_bounds.is_empty() {
        TokenStream::new()
    } else {
        quote!(where #(#field_bounds),*)
    };

    Ok(quote! {
        #[derive(Clone, Copy)]
        #visibility struct #frozen_name {
            #(#output_fields)*
        }

        impl #frozen_name {
            #(#accessors)*
        }

        // SAFETY: every generated field is a frozen value descriptor or a
        // library-owned immutable scalar type; the derive bounds ensure each
        // field satisfies the public frozen-value contract.
        unsafe impl ::compact_std::__private::frozen::FrozenValue for #frozen_name
            #frozen_where
        {}

        impl<'arena> ::compact_std::__private::frozen::FreezeIn<'arena> for #source_type
            #field_where
        {
            type Frozen = #frozen_name;

            fn freeze_in<'memory>(
                &self,
                source: &::compact_std::__private::core::Arena<'arena, 'memory>,
                destination: &mut ::compact_std::__private::frozen::FrozenBuilder,
            ) -> ::compact_std::__private::frozen::FrozenResult<Self::Frozen> {
                Ok(#frozen_name {
                    #(#field_initializers)*
                })
            }
        }
    })
}

fn expand_enum(
    visibility: syn::Visibility,
    name: syn::Ident,
    generics: syn::Generics,
    variants: syn::punctuated::Punctuated<syn::Variant, syn::Token![,]>,
) -> Result<TokenStream> {
    validate_generics(&generics)?;
    if !generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            generics,
            "CompactFreeze enums do not support generic parameters",
        ));
    }
    for variant in &variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                "CompactFreeze currently supports only unit enum variants",
            ));
        }
    }

    let frozen_name = format_ident!("{}Frozen", name);
    Ok(quote! {
        #visibility type #frozen_name = #name;

        // SAFETY: a fieldless enum has no payload references or destructor
        // obligations; the conditional bound requires an immutable Copy type.
        unsafe impl ::compact_std::__private::frozen::FrozenValue for #name
        where
            #name: Copy + Send + Sync + 'static,
        {}

        impl<'arena> ::compact_std::__private::frozen::FreezeIn<'arena> for #name
        where
            #name: ::compact_std::__private::frozen::FrozenValue,
        {
            type Frozen = Self;

            fn freeze_in<'memory>(
                &self,
                _source: &::compact_std::__private::core::Arena<'arena, 'memory>,
                _destination: &mut ::compact_std::__private::frozen::FrozenBuilder,
            ) -> ::compact_std::__private::frozen::FrozenResult<Self::Frozen> {
                Ok(*self)
            }
        }
    })
}

fn validate_generics(generics: &syn::Generics) -> Result<bool> {
    if generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            generics,
            "CompactFreeze does not support where clauses yet",
        ));
    }
    let mut has_arena_lifetime = false;
    for parameter in &generics.params {
        match parameter {
            GenericParam::Lifetime(lifetime)
                if lifetime.lifetime.ident == "arena" && !has_arena_lifetime =>
            {
                has_arena_lifetime = true;
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    parameter,
                    "CompactFreeze supports only an optional `'arena` lifetime parameter",
                ));
            }
        }
    }
    Ok(has_arena_lifetime)
}

fn frozen_type(ty: &Type) -> Result<Type> {
    match ty {
        Type::Paren(paren) => frozen_type(&paren.elem),
        Type::Group(group) => frozen_type(&group.elem),
        Type::Tuple(tuple) => {
            let elements = tuple
                .elems
                .iter()
                .map(frozen_type)
                .collect::<Result<syn::punctuated::Punctuated<_, syn::Token![,]>>>()?;
            Ok(syn::parse_quote!((#elements)))
        }
        Type::Array(array) => {
            let element = frozen_type(&array.elem)?;
            let length = &array.len;
            Ok(syn::parse_quote!([#element; #length]))
        }
        Type::Path(path) if path.qself.is_none() => frozen_path_type(path),
        _ => Err(syn::Error::new_spanned(
            ty,
            "CompactFreeze does not support this field type",
        )),
    }
}

fn frozen_path_type(path: &syn::TypePath) -> Result<Type> {
    let Some(segment) = path.path.segments.last() else {
        return Err(syn::Error::new_spanned(path, "unsupported empty type path"));
    };
    let name = segment.ident.to_string();
    let types = type_arguments(&segment.arguments)?;

    match name.as_str() {
        "Option" if types.len() == 1 => {
            let inner = frozen_type(types[0])?;
            Ok(syn::parse_quote!(::core::option::Option<#inner>))
        }
        "Result" if types.len() == 2 => {
            let ok = frozen_type(types[0])?;
            let error = frozen_type(types[1])?;
            Ok(syn::parse_quote!(::core::result::Result<#ok, #error>))
        }
        "CompactString" => Ok(syn::parse_quote!(::compact_std::FrozenString)),
        "CompactBytes" => Ok(syn::parse_quote!(::compact_std::FrozenBytes)),
        "CompactOsString" => Ok(syn::parse_quote!(::compact_std::FrozenOsString)),
        "CompactPathBuf" => Ok(syn::parse_quote!(::compact_std::FrozenPathBuf)),
        "CompactVec" | "Vec" if types.len() == 1 => {
            let element = frozen_type(types[0])?;
            Ok(syn::parse_quote!(::compact_std::FrozenVec<#element>))
        }
        "CompactVecDeque" | "VecDeque" if types.len() == 1 => {
            let element = frozen_type(types[0])?;
            Ok(syn::parse_quote!(::compact_std::FrozenVecDeque<#element>))
        }
        "CompactHashMap" | "HashMap" if types.len() >= 2 => {
            let key = frozen_type(types[0])?;
            let value = frozen_type(types[1])?;
            Ok(syn::parse_quote!(::compact_std::FrozenMap<#key, #value>))
        }
        "CompactHashSet" | "HashSet" if !types.is_empty() => {
            let value = frozen_type(types[0])?;
            Ok(syn::parse_quote!(::compact_std::FrozenSet<#value>))
        }
        "String" if has_lifetime_argument(&segment.arguments) => {
            Ok(syn::parse_quote!(::compact_std::FrozenString))
        }
        "Bytes" if has_lifetime_argument(&segment.arguments) => {
            Ok(syn::parse_quote!(::compact_std::FrozenBytes))
        }
        "OsString" if has_lifetime_argument(&segment.arguments) => {
            Ok(syn::parse_quote!(::compact_std::FrozenOsString))
        }
        "PathBuf" if has_lifetime_argument(&segment.arguments) => {
            Ok(syn::parse_quote!(::compact_std::FrozenPathBuf))
        }
        "Vec" | "VecDeque" | "HashMap" | "HashSet" | "CompactVec" | "CompactVecDeque"
        | "CompactHashMap" | "CompactHashSet"
            if has_lifetime_argument(&segment.arguments) =>
        {
            let compact_types = type_arguments_after_lifetime(&segment.arguments)?;
            match name.as_str() {
                "Vec" | "CompactVec" if compact_types.len() == 1 => {
                    let element = frozen_type(compact_types[0])?;
                    Ok(syn::parse_quote!(::compact_std::FrozenVec<#element>))
                }
                "VecDeque" | "CompactVecDeque" if compact_types.len() == 1 => {
                    let element = frozen_type(compact_types[0])?;
                    Ok(syn::parse_quote!(::compact_std::FrozenVecDeque<#element>))
                }
                "HashMap" | "CompactHashMap" if compact_types.len() >= 2 => {
                    let key = frozen_type(compact_types[0])?;
                    let value = frozen_type(compact_types[1])?;
                    Ok(syn::parse_quote!(::compact_std::FrozenMap<#key, #value>))
                }
                "HashSet" | "CompactHashSet" if !compact_types.is_empty() => {
                    let value = frozen_type(compact_types[0])?;
                    Ok(syn::parse_quote!(::compact_std::FrozenSet<#value>))
                }
                _ => Err(syn::Error::new_spanned(
                    path,
                    "unsupported compact collection field type",
                )),
            }
        }
        _ if is_scalar(&name) && types.is_empty() => Ok(ty_from_path(path)),
        _ if has_lifetime_argument(&segment.arguments) => {
            let frozen_ident = format_ident!("{}Frozen", segment.ident);
            Ok(syn::parse_quote!(#frozen_ident))
        }
        _ if types.is_empty() && path.path.segments.len() == 1 => {
            // A local derived compact struct or fieldless enum has a generated
            // frozen companion in the same module.
            let frozen_ident = format_ident!("{}Frozen", segment.ident);
            Ok(syn::parse_quote!(#frozen_ident))
        }
        _ => Err(syn::Error::new_spanned(
            path,
            "CompactFreeze does not know how to represent this field type",
        )),
    }
}

fn type_arguments(arguments: &PathArguments) -> Result<Vec<&Type>> {
    let PathArguments::AngleBracketed(arguments) = arguments else {
        return Ok(Vec::new());
    };
    arguments
        .args
        .iter()
        .filter_map(|argument| match argument {
            GenericArgument::Type(ty) => Some(Ok(ty)),
            GenericArgument::Lifetime(_) | GenericArgument::Const(_) => None,
            _ => Some(Err(syn::Error::new_spanned(
                argument,
                "unsupported generic argument in CompactFreeze field",
            ))),
        })
        .collect()
}

fn type_arguments_after_lifetime(arguments: &PathArguments) -> Result<Vec<&Type>> {
    let PathArguments::AngleBracketed(arguments) = arguments else {
        return Ok(Vec::new());
    };
    let mut saw_lifetime = false;
    let mut types = Vec::new();
    for argument in &arguments.args {
        match argument {
            GenericArgument::Lifetime(_) if !saw_lifetime => saw_lifetime = true,
            GenericArgument::Lifetime(_) => {
                return Err(syn::Error::new_spanned(
                    argument,
                    "CompactFreeze supports only the compact arena lifetime",
                ));
            }
            GenericArgument::Type(ty) => types.push(ty),
            GenericArgument::Const(_) => {}
            _ => {
                return Err(syn::Error::new_spanned(
                    argument,
                    "unsupported generic argument in CompactFreeze field",
                ));
            }
        }
    }
    if !saw_lifetime {
        return Err(syn::Error::new_spanned(
            arguments,
            "expected a compact arena lifetime",
        ));
    }
    Ok(types)
}

fn has_lifetime_argument(arguments: &PathArguments) -> bool {
    matches!(arguments, PathArguments::AngleBracketed(arguments) if arguments.args.iter().any(|arg| matches!(arg, GenericArgument::Lifetime(_))))
}

fn is_scalar(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
    )
}

fn ty_from_path(path: &syn::TypePath) -> Type {
    Type::Path(path.clone())
}
