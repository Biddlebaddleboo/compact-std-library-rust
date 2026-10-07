# V2.1 implementation report

The V2.1 runtime and `arena!` workstreams are implemented on top of upstream
`main` at `2eab4cc`.

## Runtime ownership and reuse

`ArenaState` lives at an aligned location inside the stable backing, so owner
tokens can reclaim allocations even if the stack `Arena` value moves. Its
32-byte allocator state tracks the backing, high-water cursor, sorted free
list, and next owner ID. New allocations use the tail; reuse is first-fit.
Released ranges are stored in their own bytes as eight-byte links, sorted and
coalesced on release. A released range that reaches the tail contracts the
cursor, including adjacent free ranges. Resize grows at the tail or into an
adjacent free range, and shrinks by returning a large enough suffix.

Each allocation has a 16-byte header for block extent, alignment prefix,
element capacity, and initialized length. The minimum usable backing is
60 bytes, enough for allocator state, one header, and one byte. `used_bytes()`
reports the high-water prefix and can contract when the tail is released;
`remaining_bytes()` includes both tail room and reusable ranges.

`ArenaAllocation<T>` is a private-field, non-`Clone` owner token containing the
stable state pointer, offset, and monotonic ID. Its `Drop` destroys the live
prefix and releases the block. The unique safe owner token prevents duplicate
release; the ID is used for slab/interner identity and is not duplicated in the
allocation header. This replaces a destructor registry and avoids per-element
native allocation.

Generic owning collections accept the unsafe `CompactValue` contract. It
requires slot-to-slot moves to preserve invariants, forbids address-dependent
or pinned values, and requires destructors to run while the backing is alive.
There is no blanket implementation for `Copy`; primitives, arrays, options,
results, tuples, generated compact handles, and compact owner wrappers are
implemented. Custom values need an explicit `unsafe impl`.

`CompactVec`, `CompactBox`, `CompactSmallVec`, `CompactSlab`, `CompactString`,
`CompactBitVec`, and `CompactInterner` now own reclaimable storage and preserve
destructor behavior. Vector growth allocates before moving, then transfers
initialized values and releases the old block; failed allocation leaves the
old sequence intact. Small-vector promotion allocates before moving its inline
prefix. Slab handles include allocation identity and slot generation.

| Value | Size |
| --- | ---: |
| `Offset32<T>` | 4 bytes |
| `CompactOption<T>` | 4 bytes |
| `ArenaAllocation<T>` / `CompactBox<T>` | 16 bytes |
| `CompactVec<T>` | 16 bytes |
| `CompactString` | 24 bytes |
| `CompactSlab<T>` | 24 bytes |
| `CompactSmallVec<T, 2>` | 24 bytes |

The handle growth carries the pointer and owner identity needed for drop and
reclamation. `CompactString` retains its twelve-byte inline payload.

## Packed fields

`read_bits` and `write_bits` use direct one-bit paths and byte-span scalar
paths for fields spanning up to eight bytes. A 64-bit field starting mid-byte
uses the correct nine-byte fallback. Bit numbering remains LSB-first and byte
order remains target-native.

One local release run (`cargo run --release -p compact_std --example
runtime_bench`, 2,000,000 operations per packed case) compared the optimized
helpers with a checked reference bit loop:

| Field | Read fast / loop | Write fast / loop |
| --- | ---: | ---: |
| Boolean | 4.73 / 5.03 ms | 10.72 / 7.95 ms |
| 3-bit | 5.37 / 10.36 ms | 11.40 / 12.89 ms |
| Cross-byte 9-bit | 18.47 / 28.35 ms | 29.00 / 29.46 ms |
| Aligned 16-bit | 18.48 / 48.58 ms | 29.17 / 50.50 ms |
| Aligned 32-bit | 12.73 / 95.95 ms | 27.49 / 98.50 ms |

In the same run, vector growth to 65,536 `u32`s took 576 microseconds,
used 262,160 bytes including allocator overhead, and finished at capacity
65,536. The sum of historical payload buffers under a leaking doubling
allocator would be 524,272 bytes. Traversal took 8.88 microseconds. Ten
thousand tail allocations took 187 microseconds; ten thousand allocate/drop/
reuse rounds took 147 microseconds. These are single-run local measurements,
not performance guarantees; the one-bit write result is slower than the
reference loop in this run.

## `arena!` binding analysis

The macro now maintains a stack of lexical `HashMap<String, LocalKind>` scopes
for `Unknown`, `Ambiguous`, known native values, moved values, compact vectors,
strings, boxes, and tuples. Name lookup searches inward to outward; each block,
match arm, loop, and closure gets its own scope, so shadowing does not change
an outer binding.

Direct facade constructors and their explicit `_in` forms establish compact
kinds. Fully qualified `std`/`alloc` collection constructors remain native.
Plain path moves, tuple moves, tuple destructuring, compact `push` arguments,
and `Box::new` arguments propagate ownership state. Explicit local type
annotations classify helper returns; arbitrary function signatures are not
inferred. Unknown or conflicting compact receivers receive a diagnostic with
an annotation or explicit `_in` fallback.

Branch joins retain a kind only when every branch agrees. A conflict involving
a compact kind becomes `Ambiguous`; other disagreement becomes `Unknown`.
Loop state joins the pre-loop and body states conservatively. Supported
destructuring includes identifiers, tuples and tuple structs, tuple `..`
patterns, parentheses, and type annotations. Struct fields, references, and
unsupported shapes are tracked as unknown; wildcards are ignored.

Read-only closures may use captured compact values when calls can be safely
rewritten. Mutating or `move` captures receive a targeted diagnostic directing
the caller to explicit `_in` methods. Native `vec!` remains rejected, and
unknown macro token streams are not rewritten.

## Changed areas and handoff

- Core allocator, offset validation, error/ABI minimum, packed helpers, and
  reclamation tests: `crates/compact_core`.
- Owning collections, destructor and reuse tests: `crates/compact_collections`.
- Generated `CompactValue` handles and SoA capacity constructor:
  `crates/compact_macros/src/compact.rs`.
- Lexical macro analysis and diagnostics: `crates/compact_macros/src/arena.rs`.
- Facade exports, runtime benchmark, README, regular-style fixture, dedicated
  `fixtures/arena_tracking`, and compile-fail UI cases.
- Runtime workstream commit: `d95485e`.
- Macro workstream commit: `97a24bb`.

Validation on the integrated tree:

- `cargo fmt --all -- --check` — passed.
- `cargo check --workspace` — passed.
- `cargo test --workspace` — passed: 37 unit/integration tests and 11 UI
  compile-fail cases.
- `cargo check -p compact_core --no-default-features` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo run --manifest-path fixtures/consumer/Cargo.toml` — passed.
- `cargo run --manifest-path fixtures/regular_rust_style/Cargo.toml` — passed.
- `cargo run --manifest-path fixtures/macro_layouts/Cargo.toml` — passed.
- `cargo run --manifest-path fixtures/arena_tracking/Cargo.toml` — passed.
- `cargo run --release -p compact_std --example runtime_bench` — passed and
  produced the measurements above.

The Miri component is unavailable on the installed
`stable-aarch64-unknown-linux-gnu` toolchain.

## Deviations and limits

- Owner identity remains in the unique token rather than increasing the
  16-byte allocation header. Safe code cannot clone or construct the token;
  unsafe code must uphold `CompactValue` and raw-offset contracts.
- Free-range lookup is first-fit linear in the number of free ranges. This
  keeps metadata in the backing and avoids native allocations; worst-case
  fragmentation lookup was not benchmarked.
- Read-only closure captures are supported; moving or mutating a captured
  compact owner is diagnosed and requires explicit API use.
- General helper-return inference and arbitrary struct-pattern field
  decomposition remain unsupported; annotations or explicit runtime calls
  resolve those cases.
