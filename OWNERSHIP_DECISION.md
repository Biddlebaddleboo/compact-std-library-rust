# V2.5 ownership stream — resolved-view contract

Status: design-only decision on `ec68fb7` (`codex/v25-ownership`). No production
source, tests, or benchmarks were changed/run. Implementation still requires
allocator-interface sign-off and baseline ranking.

## Decision

Use a small generic allocation view as the backend primitive, with specialized
collection views layered over it where they carry collection invariants. Do not
replace the deque ring view with a catch-all collection view, and do not make
the hash map's control/entry protocol part of the backend type.

## Proposed interface

The existing private `ResolvedAllocation`/`ResolvedAllocationMut` already hold
the validated pointer and copied header. Expose narrow wrappers for the other
crate, with private fields and no `Clone`, `Copy`, pointer extraction, or
initialized-length setter:

```rust
#[doc(hidden)]
pub struct CageAllocationView<'a, T: CompactValue> { /* ptr, header, PhantomData<&'a CageAllocation<T>> */ }
#[doc(hidden)]
pub struct CageAllocationViewMut<'a, T: CompactValue> { /* ptr, header_ptr, header, PhantomData<&'a mut CageAllocation<T>> */ }

impl<T: CompactValue> CageAllocation<T> {
    #[doc(hidden)]
    pub fn resolved_view(&self) -> Result<CageAllocationView<'_, T>>;
    #[doc(hidden)]
    pub fn resolved_view_mut(&mut self) -> Result<CageAllocationViewMut<'_, T>>;
}

impl<'a, T: CompactValue> CageAllocationView<'a, T> {
    pub fn len(&self) -> usize;
    pub fn capacity(&self) -> usize;
    pub fn as_slice(&self) -> &'a [T];
    pub fn uninit_capacity(&self) -> &'a [MaybeUninit<T>];
}

impl<'a, T: CompactValue> CageAllocationViewMut<'a, T> {
    pub fn len(&self) -> usize;
    pub fn capacity(&self) -> usize;
    pub fn as_slice(&self) -> &[T];
    pub fn as_mut_slice(&mut self) -> &mut [T];
    pub fn into_mut_slice(self) -> &'a mut [T];
    pub fn uninit_capacity(&self) -> &[MaybeUninit<T>];
    pub fn uninit_capacity_mut(&mut self) -> &mut [MaybeUninit<T>];
    pub fn into_uninit_capacity_mut(self) -> &'a mut [MaybeUninit<T>];
}
```

Opening either view must call the same `state()`, `read_header`, and
`validate_typed_header::<T>` sequence as today's `resolved` methods. A mutable
view may write initialized elements or `MaybeUninit` capacity, but cannot change
the initialized prefix through a public setter. Length-changing operations stay
behind safe owner helpers (`push`, `pop`, `truncate`, `extend_*`) and their
existing guards. Constructors accept only an owner borrow, not a raw/rebuilt
offset; any future offset-based API must retain full validation. The view has no
custom `Drop`: dropping it only closes the borrow; it neither frees nor
relocates storage.

## Borrow and safety contract

- **Open/close:** `resolved_view(&self)` returns a handle tied to that shared
  borrow; `resolved_view_mut(&mut self)` ties it to the exclusive owner borrow.
  The view ends by normal lexical drop, or by consuming it into a slice. A
  caller must end the view before reserve, resize, rehash, owner replacement,
  or drop. No view stores a pointer in the owner.
- **No growth:** the handle exposes only the capacity validated at open. Rust's
  `&mut CageAllocation<T>` borrow prevents safe calls to `try_resize`, `Drop`,
  or collection reserve/rehash while the view is live. `CompactVecDequeView`
  keeps its existing fixed-capacity behavior: reserve first; a full-view push
  fails without changing the deque.
- **No alias:** `CageAllocation` is a unique, non-`Clone` offset owner. The
  mutable view is non-copyable and carries `PhantomData<&mut CageAllocation<T>>`;
  a shared view carries `PhantomData<&CageAllocation<T>>`. Hash-map code may
  open a read view for `control` and a mutable view for `entries` because they
  are disjoint fields backed by separately allocated, non-overlapping extents.
  Never create two mutable views from one owner. Keep the raw-pointer-backed
  view types `!Send`/`!Sync`; do not add unsafe trait impls. The owner cannot be
  transferred while borrowed, and the view itself stays on its opening thread.
  Public `resolve_unchecked` remains unsafe and is not a fast-path substitute.
- **No stale pointer:** the view is borrowed from the owner whose offset it
  resolved. `CAGE` is a process `OnceLock`; `CageState.memory` is a stable base,
  and allocation resize is in place or fails. Collection growth that cannot
  resize in place allocates a replacement and moves values. The borrow excludes
  freeing/replacing the viewed extent until the view ends.
- **Unwind and drop:** opening a view performs no mutation. A slice-only view
  needs no unwind repair. Any future view operation that initializes a slot
  must publish the initialized prefix before user code/destructors can unwind,
  or retain an `AppendInitGuard`-style guard. Truncation must lower length
  before dropping an element. Preserve `TruncateGuard`/`AppendInitGuard`
  ordering; a guard must not reopen an owner while a live view still borrows it.
  The deque's existing `Drop` writes `head`/`len` back on return and unwind.
  Hash `retain`/`clear`/drop must continue clearing control and updating counts
  before a value destructor can panic.

## Verified consumers

- `cage.rs:1074–1128` resolves for `len`, `capacity`, slices, and indexed
  access; `1130–1296` contains capacity-bounded mutation and append guards;
  `1297–1419` moves between owners or attempts in-place resize;
  `1421–1485` exposes uninitialized capacity and creates the validated views;
  `1523–1534` truncates then releases on drop. `read_header` validates common
  offset/header/block bounds (`1748–1775`); `validate_typed_header` checks the
  typed capacity extent (`1778–1799`).
- `vec.rs:15–18` keeps `CompactVec` at its four-byte owner. `reserve` (`53–78`)
  may resize or replace storage; `push` and `try_extend` (`80–103`, `247–288`)
  already batch writes through `extend_from_iter`. `as_mut_slice` and
  `IndexMut` (`117–128`, `511–517`) open a fresh view per access; callers can
  already batch indexed edits by taking one mutable slice. `retain` (`149–218`)
  has distinct no-drop and destructor-safe unwind paths that must not be
  weakened.
- `deque.rs:37–42` stores owner/head/len; the 12-byte size is asserted in
  `tests/deque_view.rs:191` and `tests/cage_collections.rs:28–29`.
  Per-operation front/back/push/pop uses `uninit_capacity_mut` (`91–320`). The
  existing specialized `CompactVecDequeView` (`453–498`, `501–670`) resolves
  storage once, blocks growth, and writes head/len back in `Drop`.
- `hash_map.rs:260–270` owns separate control and `MaybeUninit<(K,V)>` tables.
  The probe/mutation phase of `insert/get/get_mut/remove_entry` (`308–413`)
  opens each table once; capacity checks may rehash before `insert` opens them.
  Probing uses ordinary slices. `clear/retain` (`422–457`,
  `537–561`), iterators (`500–523`), and `rehash` (`731–779`) likewise resolve
  table slices at operation boundaries. A future map-local table view may pair
  the two generic views, but the backend view should not know probe/control
  semantics.

## Recommendation and open risks

The generic base is justified as one reusable validation/borrow mechanism; keep
deque and hash behavior specialized. First candidate should be batch vector
mutation, then only add consumers selected by baseline ranking. The hidden
cross-crate types/methods are still public Rust API and need allocator-owner
approval. Their stack size, monomorphization, code size, and AArch64/x86-64
codegen are unmeasured. The owner layouts can remain unchanged (four-byte
`CompactVec`, twelve-byte deque), but any retained pointer/metadata in a
collection owner is out of contract and requires a separate quantified memory
and ABI review. No baseline ranking or approval is recorded here, so no
benchmark or implementation is authorized by this note.

## Orchestrator decision after V2.5 baseline

Do not expose the proposed generic view in V2.5. The collections crate already
has borrow-scoped `CageAllocation` slice access, `CompactVec::as_mut_slice`, and
the specialized `CompactVecDeque::with_view`; the measured batch gains come
from using those existing scopes, not from adding another public type. A new
cross-crate API has no measured incremental benefit yet, while it adds public
surface, monomorphization, and code-size questions. Keep the generic handle as
a future option if profiling finds a cross-collection consumer that cannot
use the current methods. Preserve this note's lifetime and unwind obligations
for any later proposal.
