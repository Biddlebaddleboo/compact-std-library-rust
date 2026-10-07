# Workstream Plan: Robust `arena!` Binding Tracking

## Objective

Replace the current variable-name heuristic in:

`crates/compact_macros/src/arena.rs`

with lexical binding-aware analysis sufficient for ordinary Rust patterns supported by the compact facade.

The macro must continue to be a procedural macro over normal Rust syntax.

Do not turn this into a compiler, MIR plugin, or global type-inference system.

---

# Starting implementation

Inspect first:

`crates/compact_macros/src/arena.rs`

Exact symbols:

- `ArenaInput`
- `expand`
- `LocalKind`
- `ArenaRewrite`
- `VisitMut for ArenaRewrite`
- `ArenaRewrite::rewrite_vector_index`
- `classify_constructor`
- `rewrite_constructor`
- `rewrite_method`

Also inspect:

`crates/compact_std/src/lib.rs`

for facade aliases.

Fixtures:

- `fixtures/regular_rust_style/src/main.rs`
- macro compile-fail tests under `crates/compact_std/tests/ui`

---

# Verified current limitation

Tracking is currently approximately:

```rust
HashMap<String, LocalKind>
```

where `LocalKind` is `Vec` or `String`.

This loses Rust binding identity.

For example, the following require explicit correct treatment:

```rust
let mut a = Vec::new();
let mut b = a;
b.push(1)?;
```

```rust
let mut value = Vec::new();

{
    let value = 3_u32;
    // must not be treated as the outer compact vector
}

value.push(1)?;
```

```rust
let mut a = Vec::new();
a = make_compact_vec(arena)?;
a.push(1)?;
```

and destructuring:

```rust
let (mut a, mut b) = make_two_vectors(arena)?;
```

---

# Design principle

Track **bindings in lexical scopes**, not strings globally.

The macro does not need complete Rust type inference.

It needs a conservative abstract value analysis for the compact types it rewrites.

---

# Abstract value kinds

Generalize current `LocalKind`.

At minimum:

```text
Unknown
CompactVec
CompactString
CompactBox
KnownNonCompact
```

Add other compact types only when they actually need method rewriting.

`Unknown` must not be silently assumed compact.

An ambiguous receiver should remain ordinary Rust or produce an actionable diagnostic when rewriting would otherwise be required.

---

# Scope stack

Maintain an explicit lexical scope stack.

Each scope maps binding identity/name for that lexical level to an abstract kind.

Required behavior:

- entering block pushes scope;
- leaving block pops it;
- shadowing creates a new binding;
- lookup searches innermost to outermost;
- modifying an inner shadow must not mutate outer tracking.

Handle scopes introduced by:

- blocks;
- `if`/`else`;
- `match` arms;
- loops;
- closures where the macro chooses to analyze them.

Do not rewrite across a closure boundary unless capture behavior is explicitly supported and tested.

---

# Binding patterns

Implement recursive pattern binding for:

- identifier patterns;
- `mut`;
- tuple patterns;
- tuple-struct patterns when abstract kind decomposition is known;
- struct patterns when supported;
- parenthesized patterns;
- reference patterns only when semantically valid;
- wildcard ignored.

For patterns whose RHS shape cannot be determined, bind affected identifiers as `Unknown`.

Never guess.

---

# Move propagation

For:

```rust
let b = a;
```

if `a` is known compact and the expression is a plain move:

- bind `b` to the same compact kind.

Do not keep treating a moved-from non-`Copy` compact owner as independently usable merely because its old name remains in the table.

Track simple moved state where necessary.

Rust itself will reject illegal use-after-move, but the macro must not rewrite later syntax in a way that obscures diagnostics or changes which value is considered compact.

---

# Assignment

Handle:

```rust
a = expression;
```

Update `a`'s abstract kind from the RHS classification.

Examples:

```rust
a = Vec::new();
```

=> compact vector.

```rust
a = native_value;
```

=> known noncompact or unknown.

For compound assignment/index assignment, preserve receiver kind rather than treating it as rebinding.

---

# Constructor classification

Continue recognizing facade constructors:

- `Vec::new`
- `Vec::with_capacity`
- `String::new`
- `String::from`
- `Box::new`

and their explicit `_in` forms where useful.

Classification must happen after/alongside rewriting so the binding records the resulting compact type.

Do not classify fully qualified native constructors such as:

```rust
std::vec::Vec::new()
std::string::String::new()
```

as compact.

---

# Function/helper returns

A proc macro cannot infer arbitrary function return types from rustc.

Provide an explicit mechanism rather than guessing.

Preferred options, in order:

1. recognize return values from known compact facade constructors/functions;
2. allow explicit local type annotations to establish kind:

```rust
let mut values: Vec<'_, u32> = make_values(...)?;
```

3. optionally add a narrowly scoped annotation/helper for macro analysis if type syntax is insufficient.

Do not require annotations for direct constructors.

Do not parse external source files to discover function signatures.

---

# Branch joins

For assignment inside control flow:

```rust
let mut v = ...;

if condition {
    v = Vec::new();
} else {
    v = other;
}
```

compute a conservative join.

If all reachable branches yield the same compact kind:

- preserve it.

Otherwise:

- mark `Unknown`.

Do not choose whichever branch is visited first by the syntax walker.

Apply equivalent reasoning to `match`.

---

# Method rewriting

Rewrite only when receiver classification is known.

For compact vectors, preserve current mappings:

- `push` -> `push_in`
- `pop` -> `pop_in`
- `reserve` -> `reserve_in`
- `get`
- `get_mut`
- `as_slice`
- `as_mut_slice`
- `iter`

with correct arena insertion.

Extend for new runtime methods only after Workstream A freezes signatures.

For compact strings:

- `push_str`
- `push_char`
- `truncate`
- `as_str`
- `as_bytes`

and other explicitly supported methods.

Do not rewrite unrelated methods merely because names match.

---

# Index rewriting

Current:

```rust
values[index]
```

rewrites only for a known compact vector.

Preserve that rule.

Ensure:

- immutable index -> compact `get`;
- assignment LHS/mutable index -> compact `get_mut`;
- nested indexes work where supported;
- shadowed native vectors are not rewritten.

Add explicit out-of-bounds behavior tests.

---

# Macro invocations

Current code special-cases assertion macros and rejects native `vec!`.

Retain native `vec!` rejection unless V2.1 introduces a genuine compact `vec!` equivalent.

Do not recursively rewrite arbitrary unknown macro token streams.

Only rewrite inside known expression-bearing macros where parsing is reliable.

---

# Closures

Closures are a boundary requiring explicit policy.

Minimum acceptable V2.1 behavior:

- correctly handle compact variables captured from outer scope for read-only uses if rewriting can be proven safe;
- otherwise emit a clear diagnostic directing the user to explicit `_in` APIs inside the closure.

Do not silently mis-rewrite captures.

---

# Loops

Support:

```rust
for ...
while ...
loop ...
```

with lexical scopes.

Bindings created in loop patterns must not escape the loop.

Mutation of outer compact bindings should remain tracked when kind is unchanged.

If loop control flow can reassign a tracked binding to incompatible kinds, degrade it to `Unknown`.

---

# Diagnostics

Add targeted diagnostics for cases where macro convenience cannot safely determine the compact receiver.

Examples:

- ambiguous helper return;
- incompatible branch kinds;
- unsupported destructuring;
- closure case requiring explicit arena method;
- attempted native `vec!`.

Diagnostics should tell the user the explicit fallback, e.g.:

```text
arena! cannot prove this receiver is a compact Vec;
call CompactVec::push_in(..., arena) or add an explicit compact type annotation
```

Do not produce opaque proc-macro parse errors where a semantic diagnostic can be generated.

---

# Tests

Add a dedicated fixture:

```text
fixtures/arena_tracking/
```

Cover at minimum:

## Moves

```rust
let a = Vec::new();
let mut b = a;
b.push(1)?;
```

## Shadowing

```rust
let mut value = Vec::new();
{
    let value = std::vec::Vec::<u8>::new();
    assert!(value.is_empty());
}
value.push(1)?;
```

## Reassignment

Compact -> compact remains tracked.

Compact -> unknown/native stops compact rewriting.

## Destructuring

Supported tuple/pattern cases.

## Branches

Same-kind join remains compact.

Different-kind join produces explicit fallback/diagnostic.

## Closures

Declared supported capture behavior.

## Loops

Loop-local shadowing and outer mutation.

## Indexing

Moved/shadowed vector indexing uses correct target.

## Strings

Move and shadow tracking for `String`.

## Explicit `_in`

Code using explicit runtime APIs inside `arena!` must remain valid and not be double-rewritten.

---

# Compile-fail tests

Add UI tests for:

- ambiguous receiver where rewriting is requested;
- unsupported compact helper return without annotation;
- unsupported closure case;
- branch kind conflict if an arena-dependent method follows;
- malformed/unsupported destructuring where relevant.

Keep current tests for:

- native `vec!`;
- unsupported compact layouts;
- non-const bounds;
- payload enums;
- unsupported pointer fields.

---

# Performance

Macro expansion performance should remain linear or near-linear in AST size.

Avoid repeated whole-block rescans.

Do not build a compiler-grade CFG.

Use one structured traversal with lightweight scope/join state.

No runtime tracking code should be emitted solely for macro analysis.

All analysis happens at compile time.

---

# Non-goals

Do not implement:

- arbitrary Rust type inference;
- trait resolution;
- module resolution;
- external function signature discovery;
- borrow checker replacement;
- MIR analysis;
- automatic conversion of native collections;
- implicit global arena.

Rustc remains authoritative for actual type and borrow checking.

---

# Integration requirements

Before final integration, consume the finalized Workstream A signatures.

Update:

`fixtures/regular_rust_style/src/main.rs`

to demonstrate at least:

- compact vector move;
- shadowing;
- string move or helper-return annotation;
- ordinary indexing after move.

Do not weaken explicit error propagation.

---

# Required handoff

Report:

- scope/binding representation;
- RHS classification rules;
- branch-join rules;
- supported destructuring forms;
- closure policy;
- helper-return mechanism;
- changed files;
- commit SHA;
- compile-pass tests;
- compile-fail tests;
- fixture results;
- deviations and unresolved assumptions.
