# Workstream Plan: Layout Macros and Regular-Rust Ergonomics

## Objective

Make compact arena programming look and feel as close as practical to ordinary Rust without a custom compiler.

Primary outputs:

- `#[compact]` generated layouts;
- constant-driven bit packing;
- boolean grouping;
- compact enum support;
- std-like facade crate;
- prelude;
- arena-scoped syntax assistance;
- explicit hot/cold and SoA generation.

## Starting interfaces

Read first after Runtime/Collections stabilizes:

```text
compact_core::Arena
compact_core::Offset32
compact_core::BitField
compact_core::bits_required
compact_collections::CompactBox
compact_collections::CompactVec
compact_collections::CompactString
```

Proposed additions:

```text
crates/compact_macros/Cargo.toml
crates/compact_macros/src/lib.rs
crates/compact_macros/src/compact.rs
crates/compact_macros/src/layout.rs
crates/compact_macros/src/arena.rs

crates/compact_std/Cargo.toml
crates/compact_std/src/lib.rs
crates/compact_std/src/prelude.rs

fixtures/regular_rust_style/
fixtures/macro_layouts/
```

## 1. `#[compact]` structs

Support explicitly annotated structs:

```rust
#[compact]
struct Job {
    #[max = 7]
    retries: u64,

    active: bool,

    #[max = 65535]
    worker_id: usize,
}
```

Generate:

- compact stored representation;
- logical getters/setters;
- checked construction;
- compile-time field layout metadata;
- native/logical conversion where useful.

Logical API should expose:

```text
retries -> u64
active -> bool
worker_id -> usize
```

even when physical storage is narrower.

## 2. Constant-driven bounds

Support const paths/expressions:

```rust
const MAX_RETRIES: u64 = 7;

#[compact(max = MAX_RETRIES)]
retries: u64
```

Generated layout should use const-evaluable helpers such as:

```rust
const RETRIES_BITS: u8 =
    compact_core::bits_required(MAX_RETRIES);
```

Preserve ordinary Rust constants for rustc optimization.

Do not duplicate constant values into runtime metadata unnecessarily.

## 3. Conservative narrowing

Only narrow when the complete domain is established by:

- explicit `max`;
- boolean;
- enum variant count/layout;
- bounded compact type;
- container invariant.

Unknown integer fields retain full logical storage width or an explicitly chosen compact primitive.

Never infer bounds from:

- observed values;
- examples;
- debug assertions;
- comments;
- incidental constructor usage.

## 4. Boolean grouping

Pack unrelated ordinary compact bool fields into shared scalar words where safe.

Example:

```rust
#[compact]
struct Flags {
    a: bool,
    b: bool,
    c: bool,
}
```

should use three physical bits, subject to layout grouping/alignment rules.

Generated setters must preserve neighboring bits.

## 5. Compact enums

For fieldless enums:

```rust
#[compact]
enum State {
    Idle,
    Running,
    Done,
}
```

use the minimum required discriminant bits.

For payload enums, support only layouts that can be represented safely and predictably.

Do not overcomplicate V2 with a universal arbitrary Rust enum ABI transformer.

Invalid bit patterns must not be exposed as safe logical enum values.

## 6. Compact struct field placement

Generated layout should group fields intelligently into `u8/u16/u32/u64` words.

Goals:

- minimize total bytes;
- minimize unnecessary padding;
- avoid excessive cross-word accesses;
- keep commonly grouped generated metadata local.

Do not use `#[repr(packed)]` as the main mechanism.

Generate explicit loads/masks/shifts.

## 7. Field byte descriptors

Generate compile-time descriptors for relevant packed fields where useful:

```text
word byte offset
bit offset
bit width
storage word
```

These descriptors should permit:

- debugging;
- serializers;
- direct byte inspection;
- generated native adapters.

They must be compile-time constants where possible.

## 8. `compact_std` facade

Create a normal library crate giving users a single ergonomic dependency.

Expose familiar names through:

```rust
use compact_std::prelude::*;
```

The prelude may export aliases/re-exports conceptually like:

```text
Vec    -> CompactVec
String -> CompactString
Box    -> CompactBox
```

Names should be idiomatic enough that converted code requires minimal edits.

Keep fully qualified compact names available to avoid ambiguity.

## 9. Constructor compatibility

Mirror standard APIs where practical:

```rust
Vec::new_in(arena)
Vec::with_capacity_in(...)
String::new_in(arena)
String::from_str_in(...)
Box::new_in(...)
```

Add ordinary-looking aliases only when arena context can be supplied safely.

Do not introduce hidden global context just to make `Vec::new()` work.

## 10. `arena!` macro

Provide a lexical macro/proc-macro facility that can reduce arena plumbing.

Example target:

```rust
compact_std::arena!(arena, {
    let mut values = Vec::new();
    values.push(1)?;
});
```

The macro may rewrite supported constructors to arena-aware forms.

Requirements:

- lexical only;
- no TLS;
- no global arena;
- no rewriting outside macro body;
- unsupported syntax must produce actionable diagnostics;
- generated code remains normal Rust after macro expansion;
- allocation failure semantics remain explicit.

Do not try to parse/reimplement all Rust semantics manually.

Use `syn`/`quote` or equivalent normal proc-macro tooling where appropriate.

## 11. Regular code preservation

Within an arena block, avoid rewriting unrelated operations.

These should remain ordinary Rust AST/code:

```text
if
match
for
while
arithmetic
method calls not related to compact allocation
pattern matching
generic algorithms
local native stack variables
```

Rewrite only storage/allocation constructs that need compact equivalents.

## 12. Native boundaries

Provide ergonomic conversion/borrow methods:

```rust
compact_string.as_str(arena)
compact_vec.as_slice(arena)
```

Generated compact structs should be able to expose native logical values without pretending physical packed memory has native struct ABI.

Owned native conversion may allocate/copy.

Borrowed compatible views should be zero-copy.

## 13. Hot/cold annotations

Support explicit opt-in:

```rust
#[compact]
struct Entry {
    #[hot]
    key: u32,

    #[cold]
    debug_label: String,
}
```

Only generate a split when the annotation clearly requests it.

One macro/workstream must own the mapping between logical object and hot/cold physical storage.

Generated accessors should hide the split.

No runtime profiling.

## 14. Struct-of-arrays

Support explicit collection-oriented generation:

```rust
#[compact(soa)]
struct Particle {
    x: f32,
    y: f32,
    active: bool,
}
```

Generate a collection with dense field arrays.

Logical iteration/access should remain recognizable.

Use packed bitset storage for boolean columns where appropriate.

Do not apply SoA to a standalone object automatically.

## 15. Diagnostics

This is critical for Codex-assisted migration.

Errors should explain:

- unsupported field type;
- bound not const-evaluable;
- maximum too large;
- invalid enum layout;
- unsupported reference ownership;
- unsupported arena constructor rewrite;
- native ABI boundary requirement.

Prefer messages that tell the executor exactly what replacement pattern to use.

## 16. Migration ergonomics

Create examples showing migration from ordinary Rust.

Before:

```rust
struct User {
    name: String,
    active: bool,
}

let mut users = Vec::new();
```

After should remain structurally close:

```rust
#[compact]
struct User {
    name: String,
    active: bool,
}

compact_std::arena!(arena, {
    let mut users = Vec::new();
});
```

The target is that a capable coding model can perform most conversions mechanically.

## Tests

Add compile-pass tests for:

- bounded integers;
- const bounds;
- boolean packing;
- enums;
- nested compact types;
- `compact_std::prelude::*`;
- `arena!`;
- ordinary loops/matches/iterators inside arena body;
- native `&str`/`&[T]` boundaries;
- hot/cold opt-in;
- SoA opt-in.

Add compile-fail tests for:

- runtime/non-const bound used where const required;
- out-of-range compile-time defaults;
- unsupported native pointer field;
- unsupported arena rewrite;
- invalid compact enum representation.

Runtime tests must verify:

- logical values round-trip;
- setters reject overflow;
- neighboring bits remain unchanged;
- generated representation sizes meet targets.

## Non-goals

Do not implement:

- rustc plugins;
- MIR analysis;
- arbitrary whole-crate rewriting;
- automatic data-flow bound discovery;
- automatic profiling-based layout;
- hidden thread-local arena;
- transparent FFI struct ABI substitution.

## Handoff

Report:

- supported syntax;
- unsupported syntax;
- generated representation examples;
- compile-time diagnostics;
- changed files;
- commit SHA;
- tests;
- any divergence from std-like method names.

The final integration owner must validate that the facade depends only on public runtime/container APIs.
