//! Direct cage-backed Serde derive.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, LitStr, Result, Token};

#[derive(Default)]
struct ContainerAttrs {
    rename_all: Option<String>,
    deny_unknown_fields: bool,
}

#[derive(Default)]
struct FieldAttrs {
    rename: Option<String>,
    default: Option<DefaultValue>,
    skip: bool,
}

enum DefaultValue {
    Trait,
    Function(syn::Path),
}

pub(crate) fn expand(input: DeriveInput) -> Result<TokenStream> {
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            input.generics,
            "CompactDeserialize does not support generic types",
        ));
    }
    let attrs = container_attrs(&input.attrs)?;
    let name = &input.ident;
    match &input.data {
        Data::Struct(data) => expand_struct(name, &data.fields, &attrs),
        Data::Enum(data) => expand_enum(name, data, &attrs),
        Data::Union(data) => Err(syn::Error::new_spanned(
            data.union_token,
            "CompactDeserialize cannot be derived for unions",
        )),
    }
}

fn expand_struct(
    name: &syn::Ident,
    fields: &Fields,
    attrs: &ContainerAttrs,
) -> Result<TokenStream> {
    let Fields::Named(fields) = fields else {
        return Err(syn::Error::new_spanned(
            fields,
            "CompactDeserialize requires named struct fields",
        ));
    };

    let visitor = format_ident!("__{}CompactVisitor", name);
    let mut declarations = Vec::new();
    let mut key_arms = Vec::new();
    let mut sequence_values = Vec::new();
    let mut initializers = Vec::new();
    let mut keys = Vec::new();

    for field in &fields.named {
        let ident = field.ident.as_ref().expect("named field");
        let ty = &field.ty;
        let field_attrs = field_attrs(&field.attrs)?;
        let slot = format_ident!("__field_{}", ident);

        if field_attrs.skip {
            let default = default_expr(field_attrs.default.as_ref());
            initializers.push(quote!(#ident: #default,));
            sequence_values.push(quote! {
                let _: Option<::compact_std::__private::serde::__private::serde::de::IgnoredAny> = seq.next_element()?;
            });
            continue;
        }

        let spelling = field_attrs
            .rename
            .unwrap_or_else(|| rename_name(&ident.to_string(), attrs.rename_all.as_deref()));
        let key = LitStr::new(&spelling, ident.span());
        declarations.push(quote!(let mut #slot: Option<#ty> = None;));
        key_arms.push(quote! {
            #key => {
                if #slot.is_some() {
                    return Err(::compact_std::__private::serde::__private::serde::de::Error::duplicate_field(#key));
                }
                #slot = Some(map.next_value_seed(
                    ::compact_std::__private::serde::CompactDeserializeSeed::<#ty>::new()
                )?);
            }
        });
        sequence_values.push(quote! {
            #slot = seq.next_element_seed(
                ::compact_std::__private::serde::CompactDeserializeSeed::<#ty>::new()
            )?;
        });
        if let Some(default) = default_expr_option(field_attrs.default.as_ref()) {
            initializers.push(quote!(#ident: #slot.unwrap_or_else(|| #default),));
        } else {
            initializers.push(quote! {
                #ident: #slot.ok_or_else(||
                    ::compact_std::__private::serde::__private::serde::de::Error::missing_field(#key)
                )?,
            });
        }
        keys.push(key);
    }

    let unknown_field = if attrs.deny_unknown_fields {
        quote! {
            return Err(::compact_std::__private::serde::__private::serde::de::Error::unknown_field(
                &key,
                &[#(#keys),*]
            ));
        }
    } else {
        quote! {
            let _: ::compact_std::__private::serde::__private::serde::de::IgnoredAny = map.next_value()?;
        }
    };

    Ok(quote! {
        impl<'__de> ::compact_std::__private::serde::CompactDeserialize<'__de> for #name {
            fn deserialize<__D>(
                deserializer: __D
            ) -> ::core::result::Result<Self, __D::Error>
            where
                __D: ::compact_std::__private::serde::__private::serde::Deserializer<'__de>
            {
                struct #visitor;
                impl<'__de> ::compact_std::__private::serde::__private::serde::de::Visitor<'__de> for #visitor {
                    type Value = #name;

                    fn expecting(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                        f.write_str("a compact struct")
                    }

                    fn visit_map<__A>(
                        self,
                        mut map: __A
                    ) -> ::core::result::Result<Self::Value, __A::Error>
                    where
                        __A: ::compact_std::__private::serde::__private::serde::de::MapAccess<'__de>
                    {
                        #(#declarations)*
                        while let Some(key) = map.next_key::<::std::string::String>()? {
                            match key.as_str() {
                                #(#key_arms)*
                                _ => { #unknown_field }
                            }
                        }
                        Ok(#name { #(#initializers)* })
                    }

                    fn visit_seq<__A>(
                        self,
                        mut seq: __A
                    ) -> ::core::result::Result<Self::Value, __A::Error>
                    where
                        __A: ::compact_std::__private::serde::__private::serde::de::SeqAccess<'__de>
                    {
                        #(#declarations)*
                        #(#sequence_values)*
                        Ok(#name { #(#initializers)* })
                    }
                }

                deserializer.deserialize_struct(stringify!(#name), &[#(#keys),*], #visitor)
            }
        }
    })
}

fn expand_enum(
    name: &syn::Ident,
    data: &syn::DataEnum,
    attrs: &ContainerAttrs,
) -> Result<TokenStream> {
    if data.variants.is_empty()
        || data
            .variants
            .iter()
            .any(|variant| !matches!(variant.fields, Fields::Unit))
    {
        return Err(syn::Error::new_spanned(
            name,
            "CompactDeserialize supports nonempty fieldless enums only",
        ));
    }

    let visitor = format_ident!("__{}CompactEnumVisitor", name);
    let mut arms = Vec::new();
    let mut variants = Vec::new();
    for variant in &data.variants {
        let ident = &variant.ident;
        let variant_attrs = variant_attrs(&variant.attrs)?;
        let spelling = variant_attrs
            .rename
            .unwrap_or_else(|| rename_name(&ident.to_string(), attrs.rename_all.as_deref()));
        let lit = LitStr::new(&spelling, ident.span());
        arms.push(quote!(#lit => Ok(#name::#ident),));
        variants.push(lit);
    }

    Ok(quote! {
        impl<'__de> ::compact_std::__private::serde::CompactDeserialize<'__de> for #name {
            fn deserialize<__D>(
                deserializer: __D
            ) -> ::core::result::Result<Self, __D::Error>
            where
                __D: ::compact_std::__private::serde::__private::serde::Deserializer<'__de>
            {
                struct #visitor;
                impl<'__de> ::compact_std::__private::serde::__private::serde::de::Visitor<'__de> for #visitor {
                    type Value = #name;

                    fn expecting(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                        f.write_str("a compact enum variant")
                    }

                    fn visit_str<__E>(
                        self,
                        value: &str
                    ) -> ::core::result::Result<Self::Value, __E>
                    where
                        __E: ::compact_std::__private::serde::__private::serde::de::Error
                    {
                        match value {
                            #(#arms)*
                            _ => Err(__E::unknown_variant(value, &[#(#variants),*]))
                        }
                    }
                }

                deserializer.deserialize_identifier(#visitor)
            }
        }
    })
}

fn container_attrs(attributes: &[syn::Attribute]) -> Result<ContainerAttrs> {
    let mut result = ContainerAttrs::default();
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("serde"))
    {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                let rule = meta.value()?.parse::<LitStr>()?.value();
                if !valid_rename_rule(&rule) {
                    return Err(meta.error("unsupported CompactDeserialize rename_all rule"));
                }
                result.rename_all = Some(rule);
            } else if meta.path.is_ident("deny_unknown_fields") {
                result.deny_unknown_fields = true;
            } else {
                return Err(meta.error("unsupported CompactDeserialize container attribute"));
            }
            Ok(())
        })?;
    }
    Ok(result)
}

fn field_attrs(attributes: &[syn::Attribute]) -> Result<FieldAttrs> {
    let mut result = FieldAttrs::default();
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("serde"))
    {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                result.rename = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("default") {
                result.default = Some(if meta.input.peek(Token![=]) {
                    let path = meta.value()?.parse::<LitStr>()?;
                    DefaultValue::Function(syn::parse_str(&path.value())?)
                } else {
                    DefaultValue::Trait
                });
            } else if meta.path.is_ident("skip") || meta.path.is_ident("skip_deserializing") {
                result.skip = true;
            } else {
                return Err(meta.error("unsupported CompactDeserialize field attribute"));
            }
            Ok(())
        })?;
    }
    Ok(result)
}

fn variant_attrs(attributes: &[syn::Attribute]) -> Result<FieldAttrs> {
    let mut result = FieldAttrs::default();
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("serde"))
    {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                result.rename = Some(meta.value()?.parse::<LitStr>()?.value());
            } else {
                return Err(meta.error("unsupported CompactDeserialize variant attribute"));
            }
            Ok(())
        })?;
    }
    Ok(result)
}

fn default_expr(value: Option<&DefaultValue>) -> TokenStream {
    default_expr_option(value).unwrap_or_else(|| quote!(::core::default::Default::default()))
}

fn default_expr_option(value: Option<&DefaultValue>) -> Option<TokenStream> {
    match value {
        Some(DefaultValue::Trait) => Some(quote!(::core::default::Default::default())),
        Some(DefaultValue::Function(path)) => Some(quote!(#path())),
        None => None,
    }
}

fn rename_name(name: &str, rule: Option<&str>) -> String {
    let Some(rule) = rule else {
        return name.to_owned();
    };
    let words = split_words(name);
    let lower: Vec<_> = words.iter().map(|word| word.to_lowercase()).collect();
    match rule {
        "lowercase" => name.to_lowercase(),
        "UPPERCASE" => name.to_uppercase(),
        "snake_case" => lower.join("_"),
        "SCREAMING_SNAKE_CASE" => lower.join("_").to_uppercase(),
        "kebab-case" => lower.join("-"),
        "SCREAMING-KEBAB-CASE" => lower.join("-").to_uppercase(),
        "PascalCase" => lower.iter().map(|word| capitalize(word)).collect(),
        "camelCase" => {
            let mut result = String::new();
            for (index, word) in lower.iter().enumerate() {
                if index == 0 {
                    result.push_str(word);
                } else {
                    result.push_str(&capitalize(word));
                }
            }
            result
        }
        _ => name.to_owned(),
    }
}

fn valid_rename_rule(rule: &str) -> bool {
    matches!(
        rule,
        "lowercase"
            | "UPPERCASE"
            | "snake_case"
            | "SCREAMING_SNAKE_CASE"
            | "kebab-case"
            | "SCREAMING-KEBAB-CASE"
            | "PascalCase"
            | "camelCase"
    )
}

fn split_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_lowercase = false;
    for character in name.chars() {
        if character == '_' || character == '-' {
            if !current.is_empty() {
                words.push(core::mem::take(&mut current));
            }
            previous_lowercase = false;
        } else {
            if character.is_uppercase() && previous_lowercase && !current.is_empty() {
                words.push(core::mem::take(&mut current));
            }
            previous_lowercase = character.is_lowercase() || character.is_ascii_digit();
            current.push(character);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
