//! Direct compact Serde derive expansion.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::visit_mut::{self, VisitMut};
use syn::{
    Attribute, Data, DeriveInput, Fields, GenericParam, Ident, Lifetime, LitStr, Path, Result, Type,
};

#[derive(Default)]
struct ContainerOptions {
    rename_all: Option<String>,
    deny_unknown_fields: bool,
}

#[derive(Default)]
struct FieldOptions {
    rename: Option<String>,
    default: Option<DefaultValue>,
    skip: bool,
}

enum DefaultValue {
    Trait,
    Function(Path),
}

struct FieldInfo {
    ident: Ident,
    ty: Type,
    key: Ident,
    binding: Ident,
    serialized_name: String,
    default: Option<DefaultValue>,
    skip: bool,
}

pub(crate) fn expand(input: DeriveInput) -> Result<TokenStream> {
    let name = input.ident;
    match input.data {
        Data::Struct(data) => expand_struct(name, input.attrs, input.generics, data.fields),
        Data::Enum(data) => expand_enum(name, input.attrs, input.generics, data.variants),
        Data::Union(union) => Err(syn::Error::new_spanned(
            union.union_token,
            "CompactDeserialize cannot be derived for unions",
        )),
    }
}

fn expand_struct(
    name: Ident,
    attributes: Vec<Attribute>,
    generics: syn::Generics,
    fields: Fields,
) -> Result<TokenStream> {
    let Fields::Named(fields) = fields else {
        return Err(syn::Error::new_spanned(
            name,
            "CompactDeserialize currently requires a struct with named fields",
        ));
    };
    let has_arena_lifetime = validate_generics(&generics)?;
    let container = parse_container_options(&attributes)?;
    let mut infos = Vec::with_capacity(fields.named.len());
    for (index, field) in fields.named.into_iter().enumerate() {
        let ident = field.ident.expect("named field");
        let options = parse_field_options(&field.attrs)?;
        let serialized_name = match options.rename {
            Some(rename) => rename,
            None => match container.rename_all.as_deref() {
                Some(rule) => rename_identifier(&ident.to_string(), rule)?,
                None => ident.to_string(),
            },
        };
        let mut default = options.default;
        if options.skip && default.is_none() {
            default = Some(DefaultValue::Trait);
        }
        infos.push(FieldInfo {
            ident,
            ty: field.ty,
            key: format_ident!("__Field{index}"),
            binding: format_ident!("__compact_field_{index}"),
            serialized_name,
            default,
            skip: options.skip,
        });
    }

    let visitor = format_ident!("__{}CompactDeserializeVisitor", name);
    let field_key = format_ident!("__{}CompactDeserializeField", name);
    let field_key_visitor = format_ident!("__{}CompactDeserializeFieldVisitor", name);
    let name_text = name.to_string();
    let active_fields: Vec<_> = infos.iter().filter(|field| !field.skip).collect();
    let field_names: Vec<_> = active_fields
        .iter()
        .map(|field| LitStr::new(&field.serialized_name, field.ident.span()))
        .collect();
    let known_field_names = quote!(&[#(#field_names),*]);

    let field_variants = active_fields.iter().map(|field| &field.key);
    let mut field_match_arms: Vec<_> = active_fields
        .iter()
        .map(|field| {
            let key = &field.key;
            let serialized_name = LitStr::new(&field.serialized_name, field.ident.span());
            quote!(#serialized_name => Ok(#field_key::#key),)
        })
        .collect();
    field_match_arms.extend(infos.iter().filter(|field| field.skip).map(|field| {
        let serialized_name = LitStr::new(&field.serialized_name, field.ident.span());
        quote!(#serialized_name => Ok(#field_key::__Unknown),)
    }));
    let unknown_field = if container.deny_unknown_fields {
        quote!(Err(
            ::compact_std::__private::serde::__private::serde::de::Error::unknown_field(
                value, FIELDS
            )
        ))
    } else {
        quote!(Ok(#field_key::__Unknown))
    };

    let field_locals = active_fields.iter().map(|field| {
        let binding = &field.binding;
        let ty = with_arena_lifetime(&field.ty, has_arena_lifetime);
        quote!(let mut #binding: ::core::option::Option<#ty> = None;)
    });
    let field_assign_arms = active_fields.iter().map(|field| {
        let key = &field.key;
        let binding = &field.binding;
        let serialized_name = LitStr::new(&field.serialized_name, field.ident.span());
        let ty = with_arena_lifetime(&field.ty, has_arena_lifetime);
        quote! {
            #field_key::#key => {
                if #binding.is_some() {
                    return Err(::compact_std::__private::serde::__private::serde::de::Error::duplicate_field(#serialized_name));
                }
                #binding = Some(map.next_value_seed(
                    ::compact_std::__private::serde::CompactDeserializeSeed::<#ty>::new(
                        &mut *self.arena,
                    ),
                )?);
            }
        }
    });
    let field_initializers = infos.iter().map(|field| {
        let ident = &field.ident;
        let serialized_name = LitStr::new(&field.serialized_name, field.ident.span());
        if field.skip {
            let default = default_expression(field, &serialized_name);
            return quote!(#ident: #default,);
        }
        let binding = &field.binding;
        let default = default_expression(field, &serialized_name);
        quote! {
            #ident: match #binding {
                Some(value) => value,
                None => #default,
            },
        }
    });

    let target_type = if has_arena_lifetime {
        quote!(#name<'arena>)
    } else {
        quote!(#name)
    };
    let visitor_output = if has_arena_lifetime {
        quote!(#name<'__compact_arena>)
    } else {
        quote!(#name)
    };
    let visitor_construct = quote!(#name);
    let bounds: Vec<_> = active_fields
        .iter()
        .map(|field| {
            let ty = with_arena_lifetime(&field.ty, has_arena_lifetime);
            quote!(#ty: ::compact_std::__private::serde::CompactDeserialize<'de, '__compact_arena>)
        })
        .collect();
    let visitor_where = if bounds.is_empty() {
        TokenStream::new()
    } else {
        quote!(where #(#bounds),*)
    };
    let trait_bounds: Vec<_> = active_fields
        .iter()
        .map(|field| {
            let ty = &field.ty;
            quote!(#ty: ::compact_std::__private::serde::CompactDeserialize<'de, 'arena>)
        })
        .collect();
    let trait_where = if trait_bounds.is_empty() {
        TokenStream::new()
    } else {
        quote!(where #(#trait_bounds),*)
    };

    Ok(quote! {
        impl<'de, 'arena> ::compact_std::__private::serde::CompactDeserialize<'de, 'arena>
            for #target_type #trait_where
        {
            fn deserialize_in<'__memory, __Deserializer>(
                deserializer: __Deserializer,
                arena: &mut ::compact_std::__private::core::Arena<'arena, '__memory>,
            ) -> ::core::result::Result<Self, __Deserializer::Error>
            where
                __Deserializer: ::compact_std::__private::serde::__private::serde::Deserializer<'de>,
            {
                use ::compact_std::__private::serde::__private::serde::de::Error as _;
                use ::compact_std::__private::serde::__private::serde::de::DeserializeSeed as _;
                use ::compact_std::__private::serde::__private::serde::de::Deserializer as _;
                use ::compact_std::__private::serde::__private::serde::de::Visitor as _;

                const FIELDS: &[&str] = #known_field_names;

                #[allow(non_camel_case_types)]
                enum #field_key {
                    #(#field_variants,)*
                    __Unknown,
                }

                struct #field_key_visitor;

                impl<'de> ::compact_std::__private::serde::__private::serde::de::Visitor<'de>
                    for #field_key_visitor
                {
                    type Value = #field_key;

                    fn expecting(
                        &self,
                        formatter: &mut ::core::fmt::Formatter<'_>,
                    ) -> ::core::fmt::Result {
                        formatter.write_str("a compact struct field name")
                    }

                    fn visit_str<__Error>(self, value: &str) -> ::core::result::Result<Self::Value, __Error>
                    where
                        __Error: ::compact_std::__private::serde::__private::serde::de::Error,
                    {
                        match value {
                            #(#field_match_arms)*
                            _ => #unknown_field,
                        }
                    }
                }

                impl<'de> ::compact_std::__private::serde::__private::serde::Deserialize<'de>
                    for #field_key
                {
                    fn deserialize<__Deserializer>(
                        deserializer: __Deserializer,
                    ) -> ::core::result::Result<Self, __Deserializer::Error>
                    where
                        __Deserializer: ::compact_std::__private::serde::__private::serde::Deserializer<'de>,
                    {
                        deserializer.deserialize_identifier(#field_key_visitor)
                    }
                }

                struct #visitor<'__borrow, '__compact_arena, '__memory> {
                    arena: &'__borrow mut ::compact_std::__private::core::Arena<'__compact_arena, '__memory>,
                }

                impl<'de, '__borrow, '__compact_arena, '__memory>
                    ::compact_std::__private::serde::__private::serde::de::Visitor<'de>
                    for #visitor<'__borrow, '__compact_arena, '__memory>
                #visitor_where
                {
                    type Value = #visitor_output;

                    fn expecting(
                        &self,
                        formatter: &mut ::core::fmt::Formatter<'_>,
                    ) -> ::core::fmt::Result {
                        formatter.write_str(concat!("a map for compact ", #name_text))
                    }

                    fn visit_map<__Map>(
                        mut self,
                        mut map: __Map,
                    ) -> ::core::result::Result<Self::Value, __Map::Error>
                    where
                        __Map: ::compact_std::__private::serde::__private::serde::de::MapAccess<'de>,
                    {
                        use ::compact_std::__private::serde::__private::serde::de::MapAccess as _;
                        use ::compact_std::__private::serde::__private::serde::de::DeserializeSeed as _;
                        #(#field_locals)*
                        while let Some(key) = map.next_key::<#field_key>()? {
                            match key {
                                #(#field_assign_arms)*
                                #field_key::__Unknown => {
                                    let _ = map.next_value::<::compact_std::__private::serde::__private::serde::de::IgnoredAny>()?;
                                }
                            }
                        }
                        Ok(#visitor_construct {
                            #(#field_initializers)*
                        })
                    }
                }

                deserializer.deserialize_struct(
                    #name_text,
                    FIELDS,
                    #visitor { arena },
                )
            }
        }
    })
}

fn expand_enum(
    name: Ident,
    attributes: Vec<Attribute>,
    generics: syn::Generics,
    variants: syn::punctuated::Punctuated<syn::Variant, syn::Token![,]>,
) -> Result<TokenStream> {
    validate_generics(&generics)?;
    if !generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            generics,
            "CompactDeserialize enums do not support generic parameters",
        ));
    }
    let container = parse_container_options(&attributes)?;
    let mut variants_info = Vec::with_capacity(variants.len());
    for variant in variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                "CompactDeserialize currently supports only unit enum variants",
            ));
        }
        let options = parse_field_options(&variant.attrs)?;
        if options.default.is_some() || options.skip {
            return Err(syn::Error::new_spanned(
                variant,
                "CompactDeserialize supports only `rename` on enum variants",
            ));
        }
        let serialized_name = options.rename.unwrap_or_else(|| {
            container
                .rename_all
                .as_deref()
                .map(|rule| rename_identifier(&variant.ident.to_string(), rule))
                .transpose()
                .expect("validated rename rule")
                .unwrap_or_else(|| variant.ident.to_string())
        });
        variants_info.push((variant.ident, serialized_name));
    }

    let variant_type = format_ident!("__{}CompactDeserializeVariant", name);
    let variant_visitor = format_ident!("__{}CompactDeserializeVariantVisitor", name);
    let visitor = format_ident!("__{}CompactDeserializeVisitor", name);
    let variants_names: Vec<_> = variants_info
        .iter()
        .map(|(_, serialized)| LitStr::new(serialized, name.span()))
        .collect();
    let variants_list = quote!(&[#(#variants_names),*]);
    let variant_ids = variants_info
        .iter()
        .enumerate()
        .map(|(index, _)| format_ident!("__Variant{index}"))
        .collect::<Vec<_>>();
    let variant_defs = variant_ids.iter();
    let variant_arms =
        variants_info
            .iter()
            .zip(variant_ids.iter())
            .map(|((ident, serialized), variant_id)| {
                let serialized = LitStr::new(serialized, ident.span());
                quote!(#serialized => Ok(#variant_type::#variant_id),)
            });
    let output_arms =
        variants_info
            .iter()
            .zip(variant_ids.iter())
            .map(|((ident, _), variant_id)| {
                quote! {
                    #variant_type::#variant_id => {
                        access.unit_variant()?;
                        Ok(#name::#ident)
                    }
                }
            });
    let name_text = name.to_string();

    Ok(quote! {
        impl<'de, 'arena> ::compact_std::__private::serde::CompactDeserialize<'de, 'arena>
            for #name
        {
            fn deserialize_in<'__memory, __Deserializer>(
                deserializer: __Deserializer,
                _arena: &mut ::compact_std::__private::core::Arena<'arena, '__memory>,
            ) -> ::core::result::Result<Self, __Deserializer::Error>
            where
                __Deserializer: ::compact_std::__private::serde::__private::serde::Deserializer<'de>,
            {
                use ::compact_std::__private::serde::__private::serde::de::Error as _;
                use ::compact_std::__private::serde::__private::serde::de::EnumAccess as _;
                use ::compact_std::__private::serde::__private::serde::de::VariantAccess as _;

                const VARIANTS: &[&str] = #variants_list;

                enum #variant_type {
                    #(#variant_defs,)*
                }

                struct #variant_visitor;

                impl<'de> ::compact_std::__private::serde::__private::serde::de::Visitor<'de>
                    for #variant_visitor
                {
                    type Value = #variant_type;

                    fn expecting(
                        &self,
                        formatter: &mut ::core::fmt::Formatter<'_>,
                    ) -> ::core::fmt::Result {
                        formatter.write_str("a compact enum variant name")
                    }

                    fn visit_str<__Error>(self, value: &str) -> ::core::result::Result<Self::Value, __Error>
                    where
                        __Error: ::compact_std::__private::serde::__private::serde::de::Error,
                    {
                        match value {
                            #(#variant_arms)*
                            _ => Err(__Error::unknown_variant(value, VARIANTS)),
                        }
                    }
                }

                impl<'de> ::compact_std::__private::serde::__private::serde::Deserialize<'de>
                    for #variant_type
                {
                    fn deserialize<__Deserializer>(
                        deserializer: __Deserializer,
                    ) -> ::core::result::Result<Self, __Deserializer::Error>
                    where
                        __Deserializer: ::compact_std::__private::serde::__private::serde::Deserializer<'de>,
                    {
                        deserializer.deserialize_identifier(#variant_visitor)
                    }
                }

                struct #visitor;

                impl<'de> ::compact_std::__private::serde::__private::serde::de::Visitor<'de>
                    for #visitor
                {
                    type Value = #name;

                    fn expecting(
                        &self,
                        formatter: &mut ::core::fmt::Formatter<'_>,
                    ) -> ::core::fmt::Result {
                        formatter.write_str(concat!("a compact ", #name_text, " enum"))
                    }

                    fn visit_enum<__Enum>(self, data: __Enum) -> ::core::result::Result<Self::Value, __Enum::Error>
                    where
                        __Enum: ::compact_std::__private::serde::__private::serde::de::EnumAccess<'de>,
                    {
                        let (variant, access) = data.variant::<#variant_type>()?;
                        match variant {
                            #(#output_arms,)*
                        }
                    }
                }

                deserializer.deserialize_enum(#name_text, VARIANTS, #visitor)
            }
        }
    })
}

fn validate_generics(generics: &syn::Generics) -> Result<bool> {
    if generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            generics,
            "CompactDeserialize does not support where clauses yet",
        ));
    }
    let mut arena_lifetime = false;
    for parameter in &generics.params {
        match parameter {
            GenericParam::Lifetime(lifetime)
                if lifetime.lifetime.ident == "arena" && !arena_lifetime =>
            {
                arena_lifetime = true;
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    parameter,
                    "CompactDeserialize supports only an optional `'arena` lifetime parameter",
                ));
            }
        }
    }
    Ok(arena_lifetime)
}

fn parse_container_options(attributes: &[Attribute]) -> Result<ContainerOptions> {
    let mut options = ContainerOptions::default();
    for attribute in attributes
        .iter()
        .filter(|attr| attr.path().is_ident("serde"))
    {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                options.rename_all = Some(parse_string_value(&meta)?.value());
                Ok(())
            } else if meta.path.is_ident("deny_unknown_fields") {
                options.deny_unknown_fields = true;
                Ok(())
            } else {
                Err(meta.error("unsupported Serde container attribute for CompactDeserialize"))
            }
        })?;
    }
    if let Some(rule) = options.rename_all.as_deref() {
        validate_rename_rule(rule)?;
    }
    Ok(options)
}

fn parse_field_options(attributes: &[Attribute]) -> Result<FieldOptions> {
    let mut options = FieldOptions::default();
    for attribute in attributes
        .iter()
        .filter(|attr| attr.path().is_ident("serde"))
    {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                options.rename = Some(parse_string_value(&meta)?.value());
                Ok(())
            } else if meta.path.is_ident("default") {
                if meta.input.peek(syn::Token![=]) {
                    let literal = parse_string_value(&meta)?;
                    let path = syn::parse_str::<Path>(&literal.value()).map_err(|error| {
                        syn::Error::new(
                            literal.span(),
                            format!("invalid default function path: {error}"),
                        )
                    })?;
                    options.default = Some(DefaultValue::Function(path));
                } else {
                    options.default = Some(DefaultValue::Trait);
                }
                Ok(())
            } else if meta.path.is_ident("skip") || meta.path.is_ident("skip_deserializing") {
                options.skip = true;
                Ok(())
            } else {
                Err(meta.error("unsupported Serde field attribute for CompactDeserialize"))
            }
        })?;
    }
    Ok(options)
}

fn parse_string_value(meta: &syn::meta::ParseNestedMeta<'_>) -> Result<LitStr> {
    let value = meta.value()?;
    value.parse()
}

fn default_expression(field: &FieldInfo, serialized_name: &LitStr) -> TokenStream {
    match &field.default {
        Some(DefaultValue::Trait) => quote!(::core::default::Default::default()),
        Some(DefaultValue::Function(path)) => quote!(#path()),
        None if is_option_type(&field.ty) => quote!(::core::option::Option::None),
        None => quote!(return Err(__Map::Error::missing_field(#serialized_name))),
    }
}

fn is_option_type(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Option")
}

fn with_arena_lifetime(ty: &Type, has_arena_lifetime: bool) -> Type {
    if !has_arena_lifetime {
        return ty.clone();
    }
    struct ReplaceArenaLifetime;
    impl VisitMut for ReplaceArenaLifetime {
        fn visit_lifetime_mut(&mut self, lifetime: &mut Lifetime) {
            if lifetime.ident == "arena" {
                *lifetime = syn::parse_quote!('__compact_arena);
            } else {
                visit_mut::visit_lifetime_mut(self, lifetime);
            }
        }
    }
    let mut output = ty.clone();
    ReplaceArenaLifetime.visit_type_mut(&mut output);
    output
}

fn rename_identifier(input: &str, rule: &str) -> Result<String> {
    let words = identifier_words(input);
    let pascal = words
        .iter()
        .map(|word| capitalize(word))
        .collect::<String>();
    let lower = words.join("");
    match rule {
        "lowercase" => Ok(lower),
        "UPPERCASE" => Ok(lower.to_uppercase()),
        "PascalCase" => Ok(pascal),
        "camelCase" => {
            let mut chars = pascal.chars();
            Ok(chars
                .next()
                .map(|first| first.to_lowercase().collect::<String>() + chars.as_str())
                .unwrap_or_default())
        }
        "snake_case" => Ok(words.join("_")),
        "SCREAMING_SNAKE_CASE" => Ok(words.join("_").to_uppercase()),
        "kebab-case" => Ok(words.join("-")),
        "SCREAMING-KEBAB-CASE" => Ok(words.join("-").to_uppercase()),
        _ => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "unsupported rename_all rule; use lowercase, UPPERCASE, camelCase, PascalCase, snake_case, SCREAMING_SNAKE_CASE, kebab-case, or SCREAMING-KEBAB-CASE",
        )),
    }
}

fn validate_rename_rule(rule: &str) -> Result<()> {
    rename_identifier("field_name", rule).map(|_| ())
}

fn identifier_words(input: &str) -> Vec<String> {
    let chars: Vec<_> = input.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();
    for (index, character) in chars.iter().copied().enumerate() {
        if !character.is_ascii_alphanumeric() {
            if !current.is_empty() {
                words.push(core::mem::take(&mut current));
            }
            continue;
        }
        let previous = index
            .checked_sub(1)
            .and_then(|prior| chars.get(prior))
            .copied();
        let next = chars.get(index + 1).copied();
        let uppercase_boundary = character.is_ascii_uppercase()
            && !current.is_empty()
            && (previous.is_some_and(|value| value.is_ascii_lowercase() || value.is_ascii_digit())
                || (previous.is_some_and(|value| value.is_ascii_uppercase())
                    && next.is_some_and(|value| value.is_ascii_lowercase())));
        if uppercase_boundary {
            words.push(core::mem::take(&mut current));
        }
        current.push(character.to_ascii_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn capitalize(input: &str) -> String {
    let mut chars = input.chars();
    chars
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}
