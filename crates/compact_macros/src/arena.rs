//! Lexical arena constructor/method rewriting.

use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::visit_mut::{self, VisitMut};
use syn::{Block, Expr, ExprCall, ExprMethodCall, Ident, PathArguments, Result, Token};

pub(crate) struct ArenaInput {
    arena: Ident,
    block: Block,
}

impl Parse for ArenaInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let arena = input.parse()?;
        input.parse::<Token![,]>()?;
        let block = input.parse()?;
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after arena block"));
        }
        Ok(Self { arena, block })
    }
}

pub(crate) fn expand(input: ArenaInput) -> Result<TokenStream> {
    let mut block = input.block;
    let mut rewrite = ArenaRewrite {
        arena: input.arena,
        locals: HashMap::new(),
        errors: Vec::new(),
    };
    rewrite.visit_block_mut(&mut block);
    if !rewrite.errors.is_empty() {
        let mut errors = rewrite.errors.into_iter();
        let mut combined = errors.next().expect("non-empty error list");
        for error in errors {
            combined.combine(error);
        }
        return Err(combined);
    }
    Ok(quote!(#block))
}

#[derive(Clone, Copy)]
enum LocalKind {
    Vec,
    String,
}

struct ArenaRewrite {
    arena: Ident,
    locals: HashMap<String, LocalKind>,
    errors: Vec<syn::Error>,
}

impl VisitMut for ArenaRewrite {
    fn visit_block_mut(&mut self, block: &mut Block) {
        for statement in &mut block.stmts {
            if let syn::Stmt::Local(local) = statement {
                let kind = local
                    .init
                    .as_ref()
                    .and_then(|init| classify_constructor(&init.expr));
                visit_mut::visit_local_mut(self, local);
                if let (Some(kind), syn::Pat::Ident(pattern)) = (kind, &local.pat) {
                    self.locals.insert(pattern.ident.to_string(), kind);
                }
            } else {
                self.visit_stmt_mut(statement);
            }
        }
    }

    fn visit_expr_mut(&mut self, expression: &mut Expr) {
        if let Expr::Assign(assignment) = expression {
            if let Expr::Index(index) = assignment.left.as_ref() {
                if let Some(indexed) = self.rewrite_vector_index(index, true) {
                    *assignment.left = indexed;
                    self.visit_expr_mut(&mut assignment.right);
                    return;
                }
            }
        }
        if let Expr::Index(indexed) = expression {
            if let Some(rewritten) = self.rewrite_vector_index(indexed, false) {
                *expression = rewritten;
                return;
            }
        }
        if let Expr::Call(call) = expression {
            if let Some(rewritten) = rewrite_constructor(call, &self.arena) {
                *expression = rewritten;
            }
        }
        if let Expr::MethodCall(call) = expression {
            if let Expr::Path(receiver) = call.receiver.as_ref() {
                if receiver.qself.is_none() && receiver.path.segments.len() == 1 {
                    let local = receiver.path.segments[0].ident.to_string();
                    if let Some(kind) = self.locals.get(&local).copied() {
                        rewrite_method(call, kind, &self.arena);
                    }
                }
            }
        }
        visit_mut::visit_expr_mut(self, expression);
    }

    fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
        if mac.path.is_ident("vec") {
            self.errors.push(syn::Error::new_spanned(
                mac,
                "vec! allocates a native Vec; use Vec::new() and push compact values inside arena!",
            ));
            return;
        }
        let name = mac
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string());
        if matches!(
            name.as_deref(),
            Some("assert")
                | Some("assert_eq")
                | Some("assert_ne")
                | Some("debug_assert")
                | Some("debug_assert_eq")
                | Some("debug_assert_ne")
        ) {
            let parser = Punctuated::<Expr, Token![,]>::parse_terminated;
            if let Ok(mut expressions) = parser.parse2(mac.tokens.clone()) {
                for expression in &mut expressions {
                    self.visit_expr_mut(expression);
                }
                mac.tokens = quote!(#expressions);
            }
        }
        visit_mut::visit_macro_mut(self, mac);
    }
}

impl ArenaRewrite {
    fn rewrite_vector_index(&mut self, index: &syn::ExprIndex, mutable: bool) -> Option<Expr> {
        let Expr::Path(receiver) = index.expr.as_ref() else {
            return None;
        };
        if receiver.qself.is_some() || receiver.path.segments.len() != 1 {
            return None;
        }
        let name = receiver.path.segments[0].ident.to_string();
        if !matches!(self.locals.get(&name), Some(LocalKind::Vec)) {
            return None;
        }
        let receiver = &index.expr;
        let subscript = &index.index;
        let arena = &self.arena;
        let method = if mutable {
            quote!(get_mut)
        } else {
            quote!(get)
        };
        Some(syn::parse_quote! {
            *#receiver.#method(#subscript, #arena)?
                .ok_or(::compact_std::__private::collections::CollectionError::Core(
                    ::compact_std::__private::core::Error::OutOfBounds,
                ))?
        })
    }
}

fn classify_constructor(expression: &Expr) -> Option<LocalKind> {
    let mut expression = expression;
    loop {
        expression = match expression {
            Expr::Try(wrapper) => &wrapper.expr,
            Expr::Paren(wrapper) => &wrapper.expr,
            Expr::Group(wrapper) => &wrapper.expr,
            _ => break,
        };
    }
    let Expr::Call(call) = expression else {
        return None;
    };
    let Expr::Path(path) = call.func.as_ref() else {
        return None;
    };
    let segments = &path.path.segments;
    if segments.len() != 2
        || segments
            .iter()
            .any(|segment| !matches!(segment.arguments, PathArguments::None))
    {
        return None;
    }
    let owner = segments[0].ident.to_string();
    let method = segments[1].ident.to_string();
    match (owner.as_str(), method.as_str()) {
        ("Vec", "new" | "with_capacity") => Some(LocalKind::Vec),
        ("String", "new" | "from") => Some(LocalKind::String),
        _ => None,
    }
}

fn rewrite_constructor(call: &ExprCall, arena: &Ident) -> Option<Expr> {
    let Expr::Path(path) = call.func.as_ref() else {
        return None;
    };
    let segments = &path.path.segments;
    if segments.len() != 2
        || segments
            .iter()
            .any(|segment| !matches!(segment.arguments, PathArguments::None))
    {
        return None;
    }
    let owner = segments[0].ident.to_string();
    let method = segments[1].ident.to_string();
    let new_method = match (owner.as_str(), method.as_str(), call.args.len()) {
        ("Vec", "new", 0) => "new_in",
        ("Vec", "with_capacity", 1) => "with_capacity_in",
        ("String", "new", 0) => "new_in",
        ("String", "from", 1) => "from_str_in",
        ("Box", "new", 1) => "new_in",
        _ => return None,
    };
    let mut rewritten_path = path.clone();
    let last = rewritten_path.path.segments.last_mut()?;
    last.ident = Ident::new(new_method, last.ident.span());
    let mut arguments = call.args.clone();
    arguments.push(syn::parse_quote!(#arena));
    Some(Expr::Call(ExprCall {
        attrs: call.attrs.clone(),
        func: Box::new(Expr::Path(rewritten_path)),
        paren_token: call.paren_token,
        args: arguments,
    }))
}

fn rewrite_method(call: &mut ExprMethodCall, kind: LocalKind, arena: &Ident) {
    let method = call.method.to_string();
    let replacement = match (kind, method.as_str()) {
        (LocalKind::Vec, "push") => Some("push_in"),
        (LocalKind::Vec, "pop") => Some("pop_in"),
        (LocalKind::Vec, "reserve") => Some("reserve_in"),
        (LocalKind::Vec, "get" | "get_mut" | "as_slice" | "as_mut_slice" | "iter") => None,
        (LocalKind::String, "push_str") => Some("push_str_in"),
        (LocalKind::String, "push_char") => Some("push_char_in"),
        (LocalKind::String, "truncate") => Some("truncate_in"),
        (LocalKind::String, "as_str" | "as_bytes") => None,
        _ => return,
    };
    if let Some(name) = replacement {
        call.method = Ident::new(name, call.method.span());
    }
    let requires_arena = replacement.is_some()
        || (matches!(kind, LocalKind::Vec)
            && matches!(
                method.as_str(),
                "get" | "get_mut" | "as_slice" | "as_mut_slice" | "iter"
            ))
        || (matches!(kind, LocalKind::String) && matches!(method.as_str(), "as_str" | "as_bytes"));
    if requires_arena {
        call.args.push(syn::parse_quote!(#arena));
    }
}
