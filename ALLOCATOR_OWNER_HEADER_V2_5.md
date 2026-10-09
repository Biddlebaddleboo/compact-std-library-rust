# V2.5 private-owner header validation prototype

**Status:** accepted for production integration after source review, all-16 A/B measurements, memory accounting, and the Miri workflow. This is a private owner-path fast path only; no allocator chunk/TLS code, owner-layout change, or public API was introduced. The pinned baseline remains `b3ca878`.

## Hypothesis

`validate_typed_header::<T>` can be omitted when a private `CageAllocation<T>` reads its header, while retaining `read_header`'s generic offset, header, block, cage-bound, and initialized-prefix checks. Keep typed validation on the unsafe external `Offset32<T>` resolver and `validate_owned` diagnostic.

The owner path currently performs both `read_header` and `validate_typed_header` from `header`, `resolved`, `resolved_mut`, and `try_resize`. The typed check multiplies `size_of::<T>() * capacity` and verifies the payload end against the block end. The proposed helper `read_owner_header<T>` retains `read_header` and omits only that second check.

## Constructor and header-writer audit

`CageAllocation` fields and `NonZeroOffset` are private. Repository search found no struct literal, `from_offset`, `From<Offset32<T>>`, clone, or copy implementation. The only owner constructor is `CageAllocation::allocate`, reached through `CompactRuntime::alloc_owned_slice` and `alloc_owned_value`.

| Path | Header effect | Payload-fit argument |
| --- | --- | --- |
| Fresh allocation in `CageAllocation::allocate` | `initialize_allocation_header` after `allocate_block` | `allocate` checks `size_of::<T>() * capacity`; `block_layout`/`allocate_block` reserve the payload plus 16-byte header, prefix, alignment, and 8-byte rounding; header records this capacity and range before the allocator lock is released |
| Exact pending reuse in `CageAllocation::allocate` | Same writer after `take_pending_reuse` removes an exact-length compatible extent | Reuse recomputes the same aligned layout for the same requested bytes; it is accepted only when computed block length exactly equals the pending extent |
| `try_resize` shrink/grow | Direct header write updates only `block_len` and `capacity` | New length is `align_up(prefix + header_size + max(size_of::<T>() * requested, 1), 8)`; shrink releases only the tail under the allocator lock, grow commits only after reserving contiguous bytes; header changes before unlocking |
| `ResolvedAllocationMut::set_initialized` | Updates only `initialized` | Callers bound the value by capacity; no payload-layout field changes |
| `AppendInitGuard::drop` | Updates only `initialized` during normal return/unwind | `written <= max <= capacity - start` is established before writes |

`Drop` reads the header through `release_extent` and full `read_header`; it does not call the typed validator because type information is erased. Its receiver is the unique owner and its only release trigger is the private `ReleaseGuard`. `CageAllocation`'s `CompactValue` implementation moves the descriptor; it does not duplicate it. Safe code cannot reconstruct a `CageAllocation` from the public typed offset.

## Prototype change

`read_owner_header<T>` takes `&CageAllocation<T>` (not a bare offset), extracts the private offset itself, calls full `read_header`, and carries the proof comment. Private owner accessors (`header`, `resolved`, `resolved_mut`, `try_resize`) use it. `read_typed_header<T>` performs full `read_header` plus `validate_typed_header`; it remains in `resolve_unchecked<T>` and `validate_owned`. `resolve_bytes_unchecked` has no element type, so it keeps its full generic `read_header` and byte-capacity check. The public owner docs now state the private invariant explicitly.

The safety argument is that only the checked allocation constructor establishes `T`'s capacity/block relationship, only checked `try_resize` changes it, and the other header writers alter only initialized length. The type is unique and non-cloneable. A raw offset has no such proof and keeps typed validation.

## Added and validated tests

- `allocator_issued_headers_satisfy_typed_payload_bounds`: cover zero, small, and larger capacities for `u8`, `u64`, and a 64-byte-aligned payload; compare owner and full typed reads.
- `resize_capacity_formula_preserves_typed_payload_bounds`: check the exact resize length formula for shrinking and growing candidates of an over-aligned payload.
- `typed_raw_offset_reader_rejects_payload_beyond_block`: show a structurally valid synthetic header is rejected by the typed external-offset reader. The malformed header is not reachable from any allocator-issued owner.
- The backend integration test exercises a real 64-byte-aligned owner through fresh allocation, growth, shrink to zero, growth again, pending exact reuse, and post-reuse access.

## Decision and limits

The source audit and validation support the payload-fit invariant for owners produced and maintained by the current private API. `read_owner_header` must remain restricted to those owners; externally reconstructed offsets continue through typed validation. Central review found no soundness defect, full Miri and focused/full-suite A/B validation passed, and the fast path is accepted for production. The independent metrics and exact validation commands are in `ALLOCATOR_OWNER_HEADER_V2_5_RESULTS.md`.

This acceptance does not close the separate unsafe chunk/TLS allocator gate. The owner-affine chunk design remains blocked on registry pin/reclaim ordering, bounded remote-release storage, TLS/reaper recovery, coherent stats, and a measured metadata/slack cap.
