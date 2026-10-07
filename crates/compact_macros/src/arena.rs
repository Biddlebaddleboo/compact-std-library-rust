//! Lexical arena constructor/method rewriting.

use std::collections::HashMap;

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::visit_mut::{self, VisitMut};
use syn::{Block, Expr, ExprCall, ExprMethodCall, Ident, Path, Result, Token, Type};

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

struct RepeatVecArgs {
    value: Expr,
    count: Expr,
}

impl Parse for RepeatVecArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let value = input.parse()?;
        input.parse::<Token![;]>()?;
        let count = input.parse()?;
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after vec! repeat count"));
        }
        Ok(Self { value, count })
    }
}

pub(crate) fn expand(input: ArenaInput) -> Result<TokenStream> {
    let mut block = input.block;
    let mut rewrite = ArenaRewrite {
        arena: input.arena,
        scopes: vec![HashMap::new()],
        closure_boundaries: Vec::new(),
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum LocalKind {
    Unknown,
    Ambiguous,
    KnownNonCompact,
    Moved,
    Vec,
    String,
    Box,
    Bytes,
    VecDeque,
    SmallVec,
    HashMap,
    HashSet,
    Ring,
    OsString,
    PathBuf,
    Tuple(Vec<LocalKind>),
}

impl LocalKind {
    fn is_compact(&self) -> bool {
        matches!(
            self,
            Self::Vec
                | Self::String
                | Self::Box
                | Self::Bytes
                | Self::VecDeque
                | Self::SmallVec
                | Self::HashMap
                | Self::HashSet
                | Self::Ring
                | Self::OsString
                | Self::PathBuf
        )
    }

    fn owns_compact_value(&self) -> bool {
        self.is_compact()
            || matches!(self, Self::Tuple(values) if values.iter().any(Self::owns_compact_value))
    }
}

struct ArenaRewrite {
    arena: Ident,
    /// One map per lexical scope; inner entries shadow outer bindings.
    scopes: Vec<HashMap<String, LocalKind>>,
    /// Scope depth at which each active closure's parameters and locals begin.
    closure_boundaries: Vec<(usize, bool)>,
    errors: Vec<syn::Error>,
}

impl VisitMut for ArenaRewrite {
    fn visit_block_mut(&mut self, block: &mut Block) {
        self.scopes.push(HashMap::new());
        for statement in &mut block.stmts {
            if let syn::Stmt::Local(local) = statement {
                self.visit_local_binding(local);
            } else {
                self.visit_stmt_mut(statement);
            }
        }
        self.scopes.pop();
    }

    fn visit_expr_mut(&mut self, expression: &mut Expr) {
        match expression {
            Expr::Assign(assignment) => {
                self.visit_assignment(assignment);
                return;
            }
            Expr::Binary(binary) if is_compound_assignment(&binary.op) => {
                if let Expr::Index(index) = binary.left.as_mut() {
                    if let Some(rewritten) = self.rewrite_vector_index(index, true) {
                        *binary.left = rewritten;
                    }
                }
                visit_mut::visit_expr_binary_mut(self, binary);
                return;
            }
            Expr::Reference(reference) if reference.mutability.is_some() => {
                if let Expr::Index(index) = reference.expr.as_mut() {
                    if let Some(rewritten) = self.rewrite_vector_index(index, true) {
                        *reference.expr = rewritten;
                    }
                }
                if let Expr::Path(path) = reference.expr.as_ref() {
                    self.reject_captured_mutation(path, reference);
                }
                visit_mut::visit_expr_reference_mut(self, reference);
                return;
            }
            Expr::Index(index) => {
                if let Some(rewritten) = self.rewrite_vector_index(index, false) {
                    *expression = rewritten;
                    self.visit_expr_mut(expression);
                    return;
                }
            }
            Expr::If(if_expr) => {
                self.visit_if(if_expr);
                return;
            }
            Expr::Match(match_expr) => {
                self.visit_match(match_expr);
                return;
            }
            Expr::ForLoop(loop_expr) => {
                self.visit_for_loop(loop_expr);
                return;
            }
            Expr::While(loop_expr) => {
                self.visit_while_loop(loop_expr);
                return;
            }
            Expr::Loop(loop_expr) => {
                self.visit_loop(loop_expr);
                return;
            }
            Expr::Closure(closure) => {
                self.visit_closure(closure);
                return;
            }
            Expr::Call(call) => {
                if let Some(rewritten) = rewrite_constructor(call, &self.arena) {
                    *expression = rewritten;
                    self.visit_expr_mut(expression);
                } else {
                    visit_mut::visit_expr_call_mut(self, call);
                }
                return;
            }
            Expr::MethodCall(call) => {
                if let Some(rewritten) = self.rewrite_arena_method(call) {
                    *expression = rewritten;
                    self.visit_expr_mut(expression);
                    return;
                }
                self.rewrite_method_call(call);
                visit_mut::visit_expr_method_call_mut(self, call);
                return;
            }
            Expr::Macro(expression_macro) if is_vec_macro(&expression_macro.mac.path) => {
                let tokens = expression_macro.mac.tokens.clone();
                if let Some(rewritten) = self.rewrite_vec_macro(tokens, expression_macro) {
                    *expression = rewritten;
                }
                return;
            }
            Expr::Macro(expression_macro) if is_format_macro(&expression_macro.mac.path) => {
                self.rewrite_format_macro(expression_macro);
                return;
            }
            Expr::Path(path) => {
                self.visit_path_use(path);
                return;
            }
            _ => {}
        }
        visit_mut::visit_expr_mut(self, expression);
    }

    fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
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
        // Unknown macro token streams are deliberately left alone.
    }
}

impl ArenaRewrite {
    fn rewrite_vec_macro(
        &mut self,
        tokens: TokenStream,
        expression_macro: &syn::ExprMacro,
    ) -> Option<Expr> {
        if !self.closure_boundaries.is_empty() {
            self.errors.push(syn::Error::new_spanned(
                expression_macro,
                "arena! cannot rewrite vec! inside a closure; construct the compact vector explicitly inside the closure",
            ));
            return None;
        }

        if let Ok(mut repeated) = syn::parse2::<RepeatVecArgs>(tokens.clone()) {
            self.classify_expr(&repeated.value, true);
            self.visit_expr_mut(&mut repeated.value);
            self.visit_expr_mut(&mut repeated.count);

            let value_ident = Ident::new("__compact_vec_value", Span::mixed_site());
            let count_ident = Ident::new("__compact_vec_count", Span::mixed_site());
            let vector_ident = Ident::new("__compact_vec_output", Span::mixed_site());
            let clone_ident = Ident::new("__compact_vec_clone", Span::mixed_site());
            let value = repeated.value;
            let count = repeated.count;
            let arena = &self.arena;
            return Some(syn::parse_quote!({
                let #value_ident = #value;
                let #count_ident: usize = #count;
                let mut #vector_ident = ::compact_std::CompactVec::with_capacity_in(
                    #count_ident,
                    #arena,
                )?;
                if #count_ident > 0 {
                    #vector_ident.push_in(#value_ident, #arena)?;
                    for _ in 1..#count_ident {
                        let #clone_ident = ::compact_std::__private::collections::CloneIn::clone_in(
                            #vector_ident
                                .get(0, #arena)?
                                .expect("repeated vec! has an initial element"),
                            #arena,
                        )?;
                        #vector_ident.push_in(#clone_ident, #arena)?;
                    }
                }
                #vector_ident
            }));
        }

        let parser = Punctuated::<Expr, Token![,]>::parse_terminated;
        let mut values = match parser.parse2(tokens) {
            Ok(values) => values,
            Err(error) => {
                self.errors.push(error);
                return None;
            }
        };
        for value in &values {
            self.classify_expr(value, true);
        }
        for value in &mut values {
            self.visit_expr_mut(value);
        }
        let arena = &self.arena;
        Some(syn::parse_quote!(
            <::compact_std::CompactVec<'_, _> as
                ::compact_std::__private::collections::FromIteratorIn<'_, _>>::from_iter_in(
                    [#values],
                    #arena,
                )?
        ))
    }

    fn rewrite_arena_method(&mut self, call: &ExprMethodCall) -> Option<Expr> {
        if !call.args.is_empty() {
            return None;
        }
        if call.method == "collect"
            && collect_target(call).is_some_and(|target| classify_type(target).is_compact())
        {
            if !self.closure_boundaries.is_empty() {
                self.errors.push(syn::Error::new_spanned(
                    call,
                    "arena! cannot allocate while rewriting `collect()` inside a closure; call FromIteratorIn explicitly there",
                ));
                return Some(Expr::MethodCall(call.clone()));
            }
            let target = collect_target(call)?;
            let iterator = call.receiver.as_ref();
            let arena = &self.arena;
            return Some(syn::parse_quote!(
                <#target as ::compact_std::__private::collections::FromIteratorIn<'_, _>>::from_iter_in(
                    #iterator,
                    #arena,
                )?
            ));
        }
        let kind = self.classify_expr(&call.receiver, false);
        let method = call.method.to_string();
        let rewrite = match (kind, method.as_str()) {
            (LocalKind::String, "to_string") => Some("to_string"),
            (kind, "clone") if kind.is_compact() => Some("clone"),
            _ => None,
        }?;

        if !self.closure_boundaries.is_empty() {
            self.errors.push(syn::Error::new_spanned(
                call,
                format!(
                    "arena! cannot allocate while rewriting `{method}()` inside a closure; call the explicit arena-aware API there"
                ),
            ));
            return Some(Expr::MethodCall(call.clone()));
        }

        let arena = &self.arena;
        let receiver_expr = call.receiver.as_ref();
        match rewrite {
            "to_string" => Some(syn::parse_quote!(
                ::compact_std::__private::collections::ToCompactStringIn::to_compact_string_in(
                    &(#receiver_expr),
                    #arena,
                )?
            )),
            "clone" => Some(syn::parse_quote!(
                ::compact_std::__private::collections::CloneIn::clone_in(
                    &(#receiver_expr),
                    #arena,
                )?
            )),
            _ => None,
        }
    }

    fn rewrite_format_macro(&mut self, expression_macro: &mut syn::ExprMacro) {
        if !self.closure_boundaries.is_empty() {
            self.errors.push(syn::Error::new_spanned(
                expression_macro,
                "arena! cannot rewrite format! inside a closure; call format_in!(arena, ...) explicitly",
            ));
            return;
        }

        let parser = Punctuated::<Expr, Token![,]>::parse_terminated;
        let mut arguments = match parser.parse2(expression_macro.mac.tokens.clone()) {
            Ok(arguments) => arguments,
            Err(error) => {
                self.errors.push(error);
                return;
            }
        };
        for argument in &mut arguments {
            self.visit_expr_mut(argument);
        }

        let arena = &self.arena;
        expression_macro.mac.path = syn::parse_quote!(::compact_std::format_in);
        expression_macro.mac.tokens = quote!(#arena, #arguments);
    }

    fn visit_local_binding(&mut self, local: &mut syn::Local) {
        let mut kind = local
            .init
            .as_ref()
            .map(|initializer| self.classify_expr(&initializer.expr, true))
            .unwrap_or(LocalKind::Unknown);
        if let Some(annotation) = pattern_annotation(&local.pat) {
            let annotated = classify_type(annotation);
            if annotated != LocalKind::Unknown {
                kind = annotated;
            }
        }

        if let Some(initializer) = &mut local.init {
            self.visit_expr_mut(&mut initializer.expr);
            if let Some((_, diverge)) = &mut initializer.diverge {
                self.visit_expr_mut(diverge);
            }
        }
        visit_mut::visit_pat_mut(self, &mut local.pat);
        self.bind_pattern(&local.pat, kind);
    }

    fn visit_assignment(&mut self, assignment: &mut syn::ExprAssign) {
        if let Expr::Index(index) = assignment.left.as_ref() {
            if let Some(rewritten) = self.rewrite_vector_index(index, true) {
                *assignment.left = rewritten;
                self.visit_expr_mut(&mut assignment.left);
                self.visit_expr_mut(&mut assignment.right);
                return;
            }
        }

        let updated = if let Expr::Path(path) = assignment.left.as_ref() {
            if let Some(name) = self.path_local_name(path) {
                if let Some((kind, depth)) = self.lookup(&name) {
                    if self.captured_at(depth) && kind.owns_compact_value() {
                        self.errors.push(syn::Error::new_spanned(
                            &*assignment,
                            "arena! cannot track reassignment of a compact value captured by a closure; use explicit `_in` APIs inside the closure",
                        ));
                    }
                }
                Some((name, self.classify_expr(&assignment.right, true)))
            } else {
                None
            }
        } else {
            None
        };

        visit_mut::visit_expr_assign_mut(self, assignment);
        if let Some((name, kind)) = updated {
            self.assign(&name, kind);
        }
    }

    fn rewrite_method_call(&mut self, call: &mut ExprMethodCall) {
        let Expr::Path(receiver) = call.receiver.as_ref() else {
            return;
        };
        let Some(name) = self.path_local_name(receiver) else {
            return;
        };
        let Some((kind, depth)) = self.lookup(&name) else {
            return;
        };
        if matches!(kind, LocalKind::Vec)
            && matches!(call.method.to_string().as_str(), "push" | "push_in")
        {
            if let Some(value) = call.args.first() {
                self.classify_expr(value, true);
            }
        }
        if self.captured_at(depth) && method_mutates(&kind, &call.method.to_string()) {
            self.errors.push(syn::Error::new_spanned(
                call,
                "arena! cannot rewrite a mutating method on a compact value captured by a closure; use the explicit `_in` method inside the closure",
            ));
            return;
        }
        if let Some((_, move_capture)) = self.closure_boundaries.last().copied() {
            if move_capture && self.captured_at(depth) && kind.owns_compact_value() {
                // The path visitor below reports this ownership boundary and
                // marks the outer binding moved for subsequent syntax.
            }
        }

        if kind.is_compact() {
            rewrite_method(call, &kind, &self.arena);
        } else if matches!(kind, LocalKind::Unknown | LocalKind::Ambiguous)
            && method_needs_compact_resolution(&call.method.to_string())
            && !has_explicit_arena_arg(call, &self.arena)
        {
            self.errors.push(syn::Error::new_spanned(
                call,
                format!(
                    "arena! cannot prove `{name}` is a compact collection; add an explicit compact type annotation or call the corresponding `*_in(..., arena)` API"
                ),
            ));
        }
    }

    fn visit_if(&mut self, expression: &mut syn::ExprIf) {
        self.visit_expr_mut(&mut expression.cond);
        let baseline = self.scopes.clone();

        self.scopes = baseline.clone();
        self.visit_block_mut(&mut expression.then_branch);
        let then_state = self.scopes.clone();

        self.scopes = baseline.clone();
        let else_state = if let Some((_, branch)) = &mut expression.else_branch {
            self.visit_expr_mut(branch);
            self.scopes.clone()
        } else {
            baseline.clone()
        };
        self.scopes = join_states(&then_state, &else_state);
    }

    fn visit_match(&mut self, expression: &mut syn::ExprMatch) {
        let matched_kind = self.classify_expr(&expression.expr, true);
        self.visit_expr_mut(&mut expression.expr);
        let baseline = self.scopes.clone();
        let mut branches = Vec::with_capacity(expression.arms.len());
        for arm in &mut expression.arms {
            self.scopes = baseline.clone();
            self.scopes.push(HashMap::new());
            self.bind_pattern(&arm.pat, matched_kind.clone());
            if let Some((_, guard)) = &mut arm.guard {
                self.visit_expr_mut(guard);
            }
            self.visit_expr_mut(&mut arm.body);
            self.scopes.pop();
            branches.push(self.scopes.clone());
        }
        self.scopes = branches
            .into_iter()
            .reduce(|left, right| join_states(&left, &right))
            .unwrap_or(baseline);
    }

    fn visit_for_loop(&mut self, expression: &mut syn::ExprForLoop) {
        self.visit_expr_mut(&mut expression.expr);
        let baseline = self.scopes.clone();
        self.scopes = baseline.clone();
        self.scopes.push(HashMap::new());
        self.bind_unknown_pattern(&expression.pat);
        self.visit_block_mut(&mut expression.body);
        self.scopes.pop();
        let body_state = self.scopes.clone();
        self.scopes = join_states(&baseline, &body_state);
    }

    fn visit_while_loop(&mut self, expression: &mut syn::ExprWhile) {
        self.visit_expr_mut(&mut expression.cond);
        let condition_state = self.scopes.clone();
        self.scopes = condition_state.clone();
        self.visit_block_mut(&mut expression.body);
        let body_state = self.scopes.clone();
        self.scopes = join_states(&condition_state, &body_state);
    }

    fn visit_loop(&mut self, expression: &mut syn::ExprLoop) {
        let baseline = self.scopes.clone();
        self.scopes = baseline.clone();
        self.visit_block_mut(&mut expression.body);
        let body_state = self.scopes.clone();
        self.scopes = join_states(&baseline, &body_state);
    }

    fn visit_closure(&mut self, expression: &mut syn::ExprClosure) {
        let move_capture = expression.capture.is_some();
        self.scopes.push(HashMap::new());
        let boundary = self.scopes.len();
        self.closure_boundaries.push((boundary, move_capture));
        for input in &mut expression.inputs {
            self.bind_unknown_pattern(input);
        }
        self.visit_expr_mut(&mut expression.body);
        self.closure_boundaries.pop();
        self.scopes.pop();
    }

    fn visit_path_use(&mut self, path: &syn::ExprPath) {
        let Some(name) = self.path_local_name(path) else {
            return;
        };
        let Some((kind, depth)) = self.lookup(&name) else {
            return;
        };
        if let Some((_, true)) = self.closure_boundaries.last().copied() {
            if self.captured_at(depth) && kind.owns_compact_value() {
                self.errors.push(syn::Error::new_spanned(
                    path,
                    "arena! cannot track a compact value captured by a `move` closure; use explicit `_in` APIs and keep the value outside the closure",
                ));
                self.assign(&name, LocalKind::Moved);
            }
        }
    }

    fn reject_captured_mutation(&mut self, path: &syn::ExprPath, span: &impl quote::ToTokens) {
        let Some(name) = self.path_local_name(path) else {
            return;
        };
        let Some((kind, depth)) = self.lookup(&name) else {
            return;
        };
        if self.captured_at(depth) && kind.owns_compact_value() {
            self.errors.push(syn::Error::new_spanned(
                span,
                "arena! cannot rewrite a mutable borrow of a compact value captured by a closure; use the explicit `_in` APIs inside the closure",
            ));
        }
    }

    fn classify_expr(&mut self, expression: &Expr, consume: bool) -> LocalKind {
        match expression {
            Expr::Try(wrapper) => self.classify_expr(&wrapper.expr, consume),
            Expr::Paren(wrapper) => self.classify_expr(&wrapper.expr, consume),
            Expr::Group(wrapper) => self.classify_expr(&wrapper.expr, consume),
            Expr::Call(call) => {
                let kind = classify_constructor(call);
                if kind == LocalKind::Box {
                    if let Some(value) = call.args.first() {
                        self.classify_expr(value, true);
                    }
                }
                kind
            }
            Expr::MethodCall(call) => match call.method.to_string().as_str() {
                "clone" => {
                    let kind = self.classify_expr(&call.receiver, false);
                    if kind.is_compact() {
                        kind
                    } else {
                        LocalKind::Unknown
                    }
                }
                "to_string" if self.classify_expr(&call.receiver, false) == LocalKind::String => {
                    LocalKind::String
                }
                "collect" => collect_target(call)
                    .map(classify_type)
                    .filter(LocalKind::is_compact)
                    .unwrap_or(LocalKind::Unknown),
                _ => LocalKind::Unknown,
            },
            Expr::Macro(expression_macro) if is_vec_macro(&expression_macro.mac.path) => {
                LocalKind::Vec
            }
            Expr::Macro(expression_macro) if is_format_macro(&expression_macro.mac.path) => {
                LocalKind::String
            }
            Expr::Path(path) => {
                let Some(name) = self.path_local_name(path) else {
                    return LocalKind::Unknown;
                };
                let Some((kind, depth)) = self.lookup(&name) else {
                    return LocalKind::Unknown;
                };
                if consume && kind.owns_compact_value() {
                    if self.captured_at(depth) {
                        self.errors.push(syn::Error::new_spanned(
                            path,
                            "arena! cannot move a compact value into a closure; use explicit `_in` APIs inside the closure",
                        ));
                    }
                    self.assign(&name, LocalKind::Moved);
                }
                kind
            }
            Expr::Tuple(tuple) => LocalKind::Tuple(
                tuple
                    .elems
                    .iter()
                    .map(|element| self.classify_expr(element, consume))
                    .collect(),
            ),
            Expr::Block(block) => classify_block_tail(self, &block.block, consume),
            Expr::If(if_expr) => {
                let then_kind = classify_block_tail(self, &if_expr.then_branch, consume);
                let else_kind = if let Some((_, otherwise)) = &if_expr.else_branch {
                    self.classify_expr(otherwise, consume)
                } else {
                    LocalKind::KnownNonCompact
                };
                join_kind(then_kind, else_kind)
            }
            Expr::Match(match_expr) => match_expr
                .arms
                .iter()
                .map(|arm| self.classify_expr(&arm.body, consume))
                .reduce(join_kind)
                .unwrap_or(LocalKind::Unknown),
            _ => LocalKind::Unknown,
        }
    }

    fn bind_pattern(&mut self, pattern: &syn::Pat, kind: LocalKind) {
        match pattern {
            syn::Pat::Ident(pattern) if pattern.subpat.is_none() => {
                self.insert(pattern.ident.to_string(), kind);
            }
            syn::Pat::Ident(pattern) => {
                self.insert(pattern.ident.to_string(), LocalKind::Unknown);
                if let Some((_, subpat)) = &pattern.subpat {
                    self.bind_unknown_pattern(subpat);
                }
            }
            syn::Pat::Tuple(pattern) => {
                self.bind_tuple_elements(&pattern.elems, kind);
            }
            syn::Pat::TupleStruct(pattern) => {
                self.bind_tuple_elements(&pattern.elems, kind);
            }
            syn::Pat::Paren(pattern) => self.bind_pattern(&pattern.pat, kind),
            syn::Pat::Type(pattern) => {
                let annotated = classify_type(&pattern.ty);
                self.bind_pattern(
                    &pattern.pat,
                    if annotated == LocalKind::Unknown {
                        kind
                    } else {
                        annotated
                    },
                );
            }
            syn::Pat::Struct(pattern) => {
                for field in &pattern.fields {
                    self.bind_unknown_pattern(&field.pat);
                }
            }
            syn::Pat::Reference(pattern) => {
                self.bind_pattern(&pattern.pat, LocalKind::Unknown);
            }
            syn::Pat::Or(pattern) => {
                if let Some(first) = pattern.cases.first() {
                    self.bind_unknown_pattern(first);
                }
            }
            syn::Pat::Slice(pattern) => {
                for child in &pattern.elems {
                    self.bind_unknown_pattern(child);
                }
            }
            syn::Pat::Wild(_) | syn::Pat::Rest(_) => {}
            other => self.bind_unknown_pattern(other),
        }
    }

    fn bind_tuple_elements(&mut self, patterns: &Punctuated<syn::Pat, Token![,]>, kind: LocalKind) {
        let LocalKind::Tuple(values) = kind else {
            for pattern in patterns {
                self.bind_unknown_pattern(pattern);
            }
            return;
        };
        let rest = patterns
            .iter()
            .position(|pattern| matches!(pattern, syn::Pat::Rest(_)));
        let mapping_is_valid = match rest {
            Some(_) => values.len() >= patterns.len().saturating_sub(1),
            None => values.len() == patterns.len(),
        };
        if !mapping_is_valid {
            for pattern in patterns {
                self.bind_unknown_pattern(pattern);
            }
            return;
        }
        let suffix_count = rest.map_or(0, |rest| patterns.len() - rest - 1);
        for (index, pattern) in patterns.iter().enumerate() {
            if matches!(pattern, syn::Pat::Rest(_)) {
                continue;
            }
            let value_index = match rest {
                Some(rest) if index > rest => values.len() - suffix_count + index - rest - 1,
                _ => index,
            };
            self.bind_pattern(pattern, values[value_index].clone());
        }
    }

    fn bind_unknown_pattern(&mut self, pattern: &syn::Pat) {
        match pattern {
            syn::Pat::Ident(pattern) => {
                self.insert(pattern.ident.to_string(), LocalKind::Unknown);
                if let Some((_, subpat)) = &pattern.subpat {
                    self.bind_unknown_pattern(subpat);
                }
            }
            syn::Pat::Tuple(pattern) => {
                for child in &pattern.elems {
                    self.bind_unknown_pattern(child);
                }
            }
            syn::Pat::TupleStruct(pattern) => {
                for child in &pattern.elems {
                    self.bind_unknown_pattern(child);
                }
            }
            syn::Pat::Struct(pattern) => {
                for field in &pattern.fields {
                    self.bind_unknown_pattern(&field.pat);
                }
            }
            syn::Pat::Paren(pattern) => self.bind_unknown_pattern(&pattern.pat),
            syn::Pat::Type(pattern) => self.bind_unknown_pattern(&pattern.pat),
            syn::Pat::Reference(pattern) => self.bind_unknown_pattern(&pattern.pat),
            syn::Pat::Or(pattern) => {
                if let Some(first) = pattern.cases.first() {
                    self.bind_unknown_pattern(first);
                }
            }
            syn::Pat::Slice(pattern) => {
                for child in &pattern.elems {
                    self.bind_unknown_pattern(child);
                }
            }
            syn::Pat::Wild(_) | syn::Pat::Rest(_) => {}
            _ => {}
        }
    }

    fn rewrite_vector_index(&mut self, index: &syn::ExprIndex, mutable: bool) -> Option<Expr> {
        let Expr::Path(receiver) = index.expr.as_ref() else {
            return None;
        };
        let name = self.path_local_name(receiver)?;
        let (kind, depth) = self.lookup(&name)?;
        if !matches!(kind, LocalKind::Vec) {
            if matches!(kind, LocalKind::Unknown | LocalKind::Ambiguous) {
                self.errors.push(syn::Error::new_spanned(
                    index,
                    format!(
                        "arena! cannot prove `{name}` is a compact Vec for indexing; add an explicit compact type annotation or use `get_in`/`get_mut_in`"
                    ),
                ));
            }
            return None;
        }
        if mutable && self.captured_at(depth) {
            self.errors.push(syn::Error::new_spanned(
                index,
                "arena! cannot rewrite mutable indexing of a compact value captured by a closure; use explicit `_in` APIs inside the closure",
            ));
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

    fn path_local_name(&self, path: &syn::ExprPath) -> Option<String> {
        if path.qself.is_some() || path.path.segments.len() != 1 {
            return None;
        }
        Some(path.path.segments[0].ident.to_string())
    }

    fn insert(&mut self, name: String, kind: LocalKind) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, kind);
        }
    }

    fn lookup(&self, name: &str) -> Option<(LocalKind, usize)> {
        self.scopes
            .iter()
            .enumerate()
            .rev()
            .find_map(|(depth, scope)| scope.get(name).cloned().map(|kind| (kind, depth)))
    }

    fn assign(&mut self, name: &str, kind: LocalKind) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_owned(), kind);
                return;
            }
        }
        self.insert(name.to_owned(), kind);
    }

    fn captured_at(&self, depth: usize) -> bool {
        self.closure_boundaries
            .last()
            .is_some_and(|(boundary, _)| depth < *boundary)
    }
}

fn is_format_macro(path: &Path) -> bool {
    let names: Vec<_> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    matches!(names.as_slice(), [name] if name == "format")
        || matches!(names.as_slice(), [root, name] if (root == "std" || root == "alloc") && name == "format")
}

fn is_vec_macro(path: &Path) -> bool {
    let names: Vec<_> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    matches!(names.as_slice(), [name] if name == "vec")
}

fn collect_target(call: &ExprMethodCall) -> Option<&Type> {
    if call.method != "collect" || !call.args.is_empty() {
        return None;
    }
    let arguments = call.turbofish.as_ref()?;
    let mut types = arguments.args.iter().filter_map(|argument| match argument {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    });
    let target = types.next()?;
    types.next().is_none().then_some(target)
}

fn classify_block_tail(rewrite: &mut ArenaRewrite, block: &Block, consume: bool) -> LocalKind {
    match block.stmts.last() {
        Some(syn::Stmt::Expr(expression, None)) => rewrite.classify_expr(expression, consume),
        _ => LocalKind::KnownNonCompact,
    }
}

fn join_states(
    left: &[HashMap<String, LocalKind>],
    right: &[HashMap<String, LocalKind>],
) -> Vec<HashMap<String, LocalKind>> {
    left.iter()
        .zip(right)
        .map(|(left_scope, right_scope)| {
            let mut joined = HashMap::new();
            for name in left_scope.keys().chain(right_scope.keys()) {
                let left_kind = left_scope.get(name).cloned().unwrap_or(LocalKind::Unknown);
                let right_kind = right_scope.get(name).cloned().unwrap_or(LocalKind::Unknown);
                joined.insert(name.clone(), join_kind(left_kind, right_kind));
            }
            joined
        })
        .collect()
}

fn join_kind(left: LocalKind, right: LocalKind) -> LocalKind {
    if left == right {
        return left;
    }
    if left.owns_compact_value()
        || right.owns_compact_value()
        || matches!(left, LocalKind::Ambiguous)
        || matches!(right, LocalKind::Ambiguous)
    {
        LocalKind::Ambiguous
    } else {
        LocalKind::Unknown
    }
}

fn pattern_annotation(pattern: &syn::Pat) -> Option<&Type> {
    match pattern {
        syn::Pat::Type(pattern) => Some(&pattern.ty),
        _ => None,
    }
}

fn classify_type(ty: &Type) -> LocalKind {
    match ty {
        Type::Paren(ty) => classify_type(&ty.elem),
        Type::Group(ty) => classify_type(&ty.elem),
        Type::Tuple(ty) => LocalKind::Tuple(ty.elems.iter().map(classify_type).collect()),
        Type::Path(ty) if ty.qself.is_none() => classify_type_path(&ty.path),
        _ => LocalKind::Unknown,
    }
}

fn classify_type_path(path: &Path) -> LocalKind {
    let names: Vec<_> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    let last = names.last().map(String::as_str).unwrap_or_default();
    if is_std_type_path(&names) {
        return LocalKind::KnownNonCompact;
    }
    if names.len() > 1
        && !(names.first().is_some_and(|name| name == "compact_std")
            && (names.len() == 2
                || (names.len() == 3 && names.get(1).is_some_and(|name| name == "prelude"))))
    {
        return LocalKind::KnownNonCompact;
    }
    match last {
        "Vec" | "CompactVec" => LocalKind::Vec,
        "String" | "CompactString" => LocalKind::String,
        "Box" | "CompactBox" => LocalKind::Box,
        "CompactBytes" => LocalKind::Bytes,
        "CompactVecDeque" => LocalKind::VecDeque,
        "CompactSmallVec" => LocalKind::SmallVec,
        "HashMap" | "CompactHashMap" => LocalKind::HashMap,
        "HashSet" | "CompactHashSet" => LocalKind::HashSet,
        "CompactRing" => LocalKind::Ring,
        "OsString" | "CompactOsString" => LocalKind::OsString,
        "PathBuf" | "CompactPathBuf" => LocalKind::PathBuf,
        "Option" | "Result" => LocalKind::KnownNonCompact,
        _ => LocalKind::Unknown,
    }
}

fn is_std_type_path(names: &[String]) -> bool {
    names.starts_with(&["std".to_owned(), "vec".to_owned()])
        || names.starts_with(&["std".to_owned(), "string".to_owned()])
        || names.starts_with(&["std".to_owned(), "boxed".to_owned()])
        || names.starts_with(&["std".to_owned(), "collections".to_owned()])
        || names.starts_with(&["alloc".to_owned(), "vec".to_owned()])
        || names.starts_with(&["alloc".to_owned(), "string".to_owned()])
        || names.starts_with(&["alloc".to_owned(), "boxed".to_owned()])
        || names.starts_with(&["alloc".to_owned(), "collections".to_owned()])
}

fn classify_constructor(expression: &ExprCall) -> LocalKind {
    let Expr::Path(path) = expression.func.as_ref() else {
        return LocalKind::Unknown;
    };
    let Some((owner, compact)) = constructor_owner(&path.path) else {
        return LocalKind::Unknown;
    };
    let method = path.path.segments.last().unwrap().ident.to_string();
    let recognized = matches!(
        (owner, method.as_str()),
        (
            "Vec",
            "new" | "with_capacity" | "new_in" | "with_capacity_in"
        ) | ("String", "new" | "from" | "new_in" | "from_str_in")
            | ("Box", "new" | "new_in")
            | (
                "OsString",
                "new" | "from" | "from_os_str" | "from_os_string" | "new_in"
            )
            | ("PathBuf", "new" | "from" | "from_path" | "new_in")
            | (
                "HashMap" | "HashSet",
                "new" | "with_hasher" | "with_capacity" | "with_capacity_and_hasher"
            )
    );
    if !recognized {
        LocalKind::Unknown
    } else if compact {
        match owner {
            "Vec" => LocalKind::Vec,
            "String" => LocalKind::String,
            "Box" => LocalKind::Box,
            "OsString" => LocalKind::OsString,
            "PathBuf" => LocalKind::PathBuf,
            "HashMap" => LocalKind::HashMap,
            "HashSet" => LocalKind::HashSet,
            _ => LocalKind::Unknown,
        }
    } else {
        LocalKind::KnownNonCompact
    }
}

fn constructor_owner(path: &Path) -> Option<(&'static str, bool)> {
    let segments = &path.segments;
    if segments.len() < 2 {
        return None;
    }
    let owner_segment = &segments[segments.len() - 2];
    let method_segment = segments.last()?;
    if !matches!(method_segment.arguments, syn::PathArguments::None) {
        return None;
    }
    let owner = owner_segment.ident.to_string();
    let owner = match owner.as_str() {
        "Vec" | "CompactVec" => "Vec",
        "String" | "CompactString" => "String",
        "Box" | "CompactBox" => "Box",
        "OsString" | "CompactOsString" => "OsString",
        "PathBuf" | "CompactPathBuf" => "PathBuf",
        "HashMap" | "CompactHashMap" => "HashMap",
        "HashSet" | "CompactHashSet" => "HashSet",
        _ => return None,
    };
    let prefix: Vec<_> = segments
        .iter()
        .take(segments.len() - 1)
        .map(|segment| segment.ident.to_string())
        .collect();
    let compact = match prefix.as_slice() {
        [_] => true,
        [root, alias] if root == "compact_std" => {
            matches!(
                alias.as_str(),
                "Vec"
                    | "String"
                    | "Box"
                    | "CompactVec"
                    | "CompactString"
                    | "CompactBox"
                    | "OsString"
                    | "PathBuf"
                    | "CompactOsString"
                    | "CompactPathBuf"
                    | "HashMap"
                    | "HashSet"
                    | "CompactHashMap"
                    | "CompactHashSet"
            )
        }
        [root, module, alias] if root == "compact_std" && module == "prelude" => {
            matches!(
                alias.as_str(),
                "Vec" | "String" | "Box" | "HashMap" | "HashSet" | "OsString" | "PathBuf"
            )
        }
        [root, module, alias] if root == "std" || root == "alloc" => false,
        _ => false,
    };
    Some((owner, compact))
}

fn rewrite_constructor(call: &ExprCall, arena: &Ident) -> Option<Expr> {
    let Expr::Path(path) = call.func.as_ref() else {
        return None;
    };
    let (owner, compact) = constructor_owner(&path.path)?;
    if !compact {
        return None;
    }
    let method = path.path.segments.last()?.ident.to_string();
    let new_method = match (owner, method.as_str(), call.args.len()) {
        ("Vec", "new", 0) => "new_in",
        ("Vec", "with_capacity", 1) => "with_capacity_in",
        ("String", "new", 0) => "new_in",
        ("String", "from", 1) => "from_str_in",
        ("Box", "new", 1) => "new_in",
        ("OsString", "new", 0) | ("PathBuf", "new", 0) => "new_in",
        ("OsString", "from", 1) => "from",
        ("OsString", "from_os_str", 1) => "from_os_str",
        ("OsString", "from_os_string", 1) => "from_os_string",
        ("PathBuf", "from", 1) => "from",
        ("PathBuf", "from_path", 1) => "from_path",
        ("HashMap" | "HashSet", "with_capacity", 1) => "with_capacity",
        ("HashMap" | "HashSet", "with_capacity_and_hasher", 2) => "with_capacity_and_hasher",
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

fn rewrite_method(call: &mut ExprMethodCall, kind: &LocalKind, arena: &Ident) {
    let method = call.method.to_string();
    let replacement = match (kind, method.as_str()) {
        (LocalKind::Vec, "push") => Some("push_in"),
        (LocalKind::Vec, "pop") => Some("pop_in"),
        (LocalKind::Vec, "reserve") => Some("reserve_in"),
        (LocalKind::Vec, "shrink_to_fit") => Some("shrink_to_fit_in"),
        (LocalKind::Vec, "get" | "get_mut" | "as_slice" | "as_mut_slice" | "iter") => None,
        (LocalKind::String, "push_str") => Some("push_str_in"),
        (LocalKind::String, "push_char") => Some("push_char_in"),
        (LocalKind::String, "truncate") => Some("truncate_in"),
        (LocalKind::String, "shrink_to_fit") => Some("shrink_to_fit_in"),
        (LocalKind::String, "as_str" | "as_bytes") => None,
        (LocalKind::Box, "get" | "get_mut") => None,
        _ => return,
    };
    if let Some(name) = replacement {
        call.method = Ident::new(name, call.method.span());
    }
    let needs_arena = replacement.is_some()
        || matches!(
            (kind, method.as_str()),
            (
                LocalKind::Vec,
                "get" | "get_mut" | "as_slice" | "as_mut_slice" | "iter"
            ) | (LocalKind::String, "as_str" | "as_bytes")
                | (LocalKind::Box, "get" | "get_mut")
        );
    if needs_arena && !has_explicit_arena_arg(call, arena) {
        call.args.push(syn::parse_quote!(#arena));
    }
}

fn has_explicit_arena_arg(call: &ExprMethodCall, arena: &Ident) -> bool {
    call.args.last().is_some_and(|argument| {
        matches!(argument, Expr::Path(path) if path.qself.is_none() && path.path.is_ident(arena))
    })
}

fn method_needs_compact_resolution(method: &str) -> bool {
    matches!(
        method,
        "push"
            | "pop"
            | "reserve"
            | "shrink_to_fit"
            | "push_str"
            | "push_char"
            | "truncate"
            | "get"
            | "get_mut"
            | "as_slice"
            | "as_mut_slice"
            | "iter"
            | "as_str"
            | "as_bytes"
    )
}

fn method_mutates(kind: &LocalKind, method: &str) -> bool {
    match kind {
        LocalKind::Vec => matches!(
            method,
            "push"
                | "push_in"
                | "pop"
                | "pop_in"
                | "reserve"
                | "reserve_in"
                | "truncate"
                | "clear"
                | "get_mut"
                | "as_mut_slice"
                | "shrink_to_fit"
                | "shrink_to_fit_in"
        ),
        LocalKind::String => matches!(
            method,
            "push"
                | "push_char"
                | "push_char_in"
                | "push_str"
                | "push_str_in"
                | "truncate"
                | "truncate_in"
                | "clear"
                | "shrink_to_fit"
                | "shrink_to_fit_in"
        ),
        LocalKind::Box => matches!(method, "get_mut"),
        _ => false,
    }
}

fn is_compound_assignment(operator: &syn::BinOp) -> bool {
    matches!(
        operator,
        syn::BinOp::AddAssign(_)
            | syn::BinOp::SubAssign(_)
            | syn::BinOp::MulAssign(_)
            | syn::BinOp::DivAssign(_)
            | syn::BinOp::RemAssign(_)
            | syn::BinOp::BitXorAssign(_)
            | syn::BinOp::BitAndAssign(_)
            | syn::BinOp::BitOrAssign(_)
            | syn::BinOp::ShlAssign(_)
            | syn::BinOp::ShrAssign(_)
    )
}
