# V2.5 allocator concurrency contract

**Status:** chunk/TLS design gate only, reviewed against the pinned `b3ca878` baseline. The separate private owner-header validation fast path is accepted and integrated at `bdeccd9`; it does not implement chunk reservation, remote routing, or a new allocator policy. No chunk/TLS production change is authorized by this note.

## Recommendation

The next architecture to evaluate is **A with B’s local reuse and remote-release handling**: demand-driven, owner-affine chunks within the existing cage, with per-chunk synchronization and a bounded remote-pending path. Keep the process-wide allocator mutex for chunk refill, non-chunk allocation, global statistics, and reclamation. Do not add lock-free reclamation. Keep the global-mutex path as the fallback and comparison control.

| Contract | Evaluation |
| --- | --- |
| **A. Bounded TLS chunks** | Only candidate that can amortize B10’s global allocation lock. Bound active reserved slack and chunk metadata; fall back to the global allocator when a thread or global budget is full. Requires a sound offset-to-chunk route for remote drops. |
| **B. Local reuse and remote releases** | Pair with A. Reuse compatible local extents under the chunk’s synchronization. Remote drops remain pending and counted live until the chunk owner or reaper drains them. A queue needs a strict capacity and no-allocation overflow path. |
| **C. Shorter/sharded critical sections** | Keep as a measured control. R4 rejected request-layout precomputation and header initialization outside the global lock: both improved some B10 timings but regressed A2 or other acceptance cases. Revisit partitioning only after the V2.5 baseline identifies remaining lock contention. |
| **D. Alternative synchronization/reclamation** | Defer. Lock-free machinery is not a goal and needs a separate proof and approval after a mutex-based design demonstrates a need. |

The proposed first chunk version is owner-affine: a chunk is not handed to another mutator thread. A live allocation handle may move between threads, but its chunk home stays fixed. A terminating owner hands its chunk to a reaper only after closing the owner path. The Phase 1 model now rejects active chunk-owner transfer and tests a pinned remote release across retirement; it remains sequential and is not a concurrency proof.

## Ownership and states

Keep the `CageAllocation<T>` four-byte offset and the 16-byte `AllocationHeader` unchanged. A chunk record is side metadata containing a monotonic chunk generation, cage interval, home thread, phase, counters, and synchronization. TLS holds a safe handle to the active chunk; no raw TLS pointer may outlive a scope. `CageAllocation` itself does not gain a chunk ID.

The authoritative owner is:

- **Global free:** the existing global allocator’s free structures and cursor.
- **Reserved:** the chunk record and its home owner. This includes unused tail slack and flushed extents reusable by that chunk.
- **Live:** the unique `CageAllocation<T>` handle and its initialized header. The holder can differ from the chunk home when `T: Send`.
- **Pending release:** the local collector or remote-pending record. It remains live for accounting and blocks reclamation.
- **Reclaimable:** the retiring chunk’s teardown record. It is unavailable to every allocator until the whole interval is published globally free.

Required transitions are `GlobalFree -> Reserved -> Live -> PendingRelease -> Reserved`, same-owner `PendingRelease -> Live` exact reuse, `Reserved -> Reclaimable -> GlobalFree`, and global `Live/PendingRelease -> GlobalFree` for legacy non-chunk extents. `Live` and `PendingRelease` count in `live_bytes`; reserved slack and reclaimable bytes are separate. The byte partition over the managed high-water prefix must satisfy:

```text
live_bytes = sum(Live) + sum(PendingRelease)
cursor - INITIAL_CURSOR = live_bytes + global_free_bytes
                           + reserved_slack_bytes + reclaimable_bytes
                           + in_cage_chunk_metadata_bytes
```

The current allocator counts the complete block length, including the header, prefix, and padding, in `live_bytes`; preserve that rule. Any metadata carved from the cage must be counted separately or charged explicitly to reserved bytes. External registry/queue memory must be reported separately.

Keep `used_bytes()` equal to live plus pending block bytes. Preserve `remaining_bytes()` as capacity minus live bytes unless an API review approves a new meaning; with chunks it is a non-live budget, not a promise that another thread can allocate a contiguous range. `AllocatorStats` must continue to expose coherent live/high-water/global-free information. Reserved slack and reaper-owned bytes need separate diagnostic accounting, but do not add fields to the public stats struct without API review. How to form an exact live-byte snapshot without serializing every owner operation remains open.

## Publication, remote free, and locking

The simplest auditable publication primitive is a mutex, not relaxed atomics. The chunk record’s mutex serializes its interval state, local reusable extents, and pending releases. Unlocking after the pending record is committed publishes it; a later owner/reaper lock acquires that state before reuse. `PendingRelease -> Reserved` is the point where live accounting decreases. Exact pending reuse is permitted only for the owning collector and a compatible extent.

The frozen owner has no chunk ID, so remote destruction needs an offset-to-chunk lookup. Proposed routing: first check whether the extent lies in the current TLS chunk; otherwise look up its interval in a mutex-protected registry, retain a stable chunk handle, then publish through the chunk mutex. If no active/retiring chunk owns the interval, use the legacy global release path. A real implementation must prevent a reclaimer from removing a registry entry while a remote lookup or release is in flight.

Use one global lock order: **chunk registry → global allocator → chunk state**. Local chunk operations take only chunk state. Remote lookup releases the registry lock before taking chunk state and never reacquires the registry lock while holding chunk state. Refill and reclaim do not call user code. Header initialization stays under the chunk state lock until a separate proof shows a committed-but-uninitialized extent cannot be observed, reused, or leaked on unwind.

If a remote queue is retained instead of synchronous chunk-mutex publication, its entries must be bounded by the maximum live extents in a chunk. `Drop` must not allocate queue storage. Queue saturation must fall back to a safe synchronous chunk-mutex operation; it must never publish a subextent globally while sibling allocations are live. A poisoned chunk is quarantined, not returned to the global allocator.

## Exit, reentrancy, panic, and failure

The home thread may allocate only while its chunk is `Active`. On exit, its TLS owner path closes under the registry, its local collector is flushed, and the chunk becomes `Retiring`. No new local allocations begin. Live handles held by other threads keep it pinned; their drops route to the reaper. `Retiring -> Reclaiming` requires zero live bytes, zero pending releases, an empty remote path, and no in-flight chunk operation. `Reclaiming -> Reclaimed` publishes the full interval only after every suballocation and slack range has passed the check. Active chunks are never transferred between mutators in this version.

Do not hold the global or chunk mutex while running a `T` destructor or user closure. Preserve `with_batched_releases` nesting: nested calls join the active collector, and scope guards restore TLS before flushing. If a panic interrupts teardown, leave the chunk registered and inaccessible in `Reclaiming`; the reaper retries or quarantines it. Reservation failure must leave cursor, free-list links, and counters unchanged. Exhaustion falls back to a new global reservation only within the configured budget; otherwise return `AllocationExhausted`. A failed/poisoned release cannot silently discard a pending descriptor.

The current model uses synthetic allocation IDs to reject stale frees. The real four-byte owner/header formats contain no allocation generation. Before implementation, either prove that safe unique-owner `Drop` is the only release source and define behavior for all unsafe/internal callers, or add generation/duplicate-free side metadata without changing frozen layouts. This is unresolved.

## Memory budget and measurements

No eager per-thread arenas. Let `N` be concurrently active chunk owners, `C_i` their reserved spans, `R` the active/retiring chunk count, `M` bytes of metadata per chunk, and `Q_i` remote queue capacity. Bound total reserve by both a per-thread cap and a global slack budget:

```text
reserved_slack <= min(global_slack_cap, sum(per_thread_cap_i))
metadata_bytes <= R * M + sum(Q_i * entry_size)
```

If either budget is exhausted, use the global allocator. Do not let the cap scale with cumulative thread creation. R4’s B10 workload used two worker threads, had a 256,040-byte high-water cursor and 3,444 KiB peak RSS; it did not establish 4/8-worker memory behavior. A hypothetical 32-byte record for each 512-byte chunk would cost 6.25% of B10’s high-water bytes at full occupancy, before queue storage. Thus chunk size and metadata representation are acceptance questions, not settled assumptions.

The pinned `b3ca878` B10 suite median was 3.277 ms compact versus 0.422 ms native (7.77x); its three-repeat hardware-counter capture measured 7.54x cycles and 4.17x instructions for compact. A compact perf sample attributed 27.91% self samples to the AArch64 acquire-CAS instruction; the full symbol report is in `PROFILE_V2_5_BASELINE.md`.

After the accepted owner-header fast path, the two-suite B10 compact median moved 23.94% and p95 10.33%, while a separate nine-pair zero-second run moved 10.56% slower and nine 0.5-second windows moved 10.18% faster. The 0.5-second compact counter capture was nearly flat (cycles −0.09%, instructions −2.08%). Treat B10 timing as unresolved two-vCPU scheduler noise; the global-mutex synchronization hotspot remains a chunk-design hypothesis, not proof of an accepted allocator redesign.

A pre-owner-header weak-scaling run kept 8,000 input records per worker and checked native/compact checksums at 1, 2, 4, and 8 workers. The host still had only two vCPUs, so the 4/8-worker rows measure oversubscription and scheduler contention rather than physical multicore scaling. Re-run the matrix after a candidate owner/allocator change before using it as a direct comparison.

| Workers | Native median/p95 ms per repetition | Compact median/p95 ms per repetition | Timing ratio | Compact/native cycles | Peak RSS native/compact KiB |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 0.300 / 0.451 | 0.616 / 0.766 | 2.06x | 2.09x | 2,952 / 3,084 |
| 2 | 0.558 / 0.728 | 2.736 / 3.416 | 4.90x | 6.91x | 3,332 / 3,348 |
| 4 | 0.948 / 1.799 | 5.375 / 5.768 | 5.67x | 5.67x | 3,612 / 3,868 |
| 8 | 1.882 / 2.224 | 11.397 / 11.814 | 6.06x | 6.62x | 5,080 / 4,952 |

Median compact retained cage bytes scaled from 128,016 B at one worker to 1,024,128 B at eight, matching the 8,000-record-per-worker workload. The captures show a material B10 lock cost from two workers onward; they do not validate behavior on a host with four or eight physical cores.

The plan’s memory gates remain: at most 2% retained-byte increase and 5% peak RSS increase against this baseline. Measure reserved slack, chunk records, queue storage, free fragmentation, cursor high-water, RSS after idle/thread exit, and all worker counts in any candidate comparison. No chunk implementation was benchmarked for this note.

## Model audit and proof required

`allocator_model.rs` is test-only and explicitly sequential. It has chunk/allocation identities, byte conservation, local/remote pending transitions, forbidden active chunk-owner transfer, a remote-release pin across owner retirement, interrupted teardown recovery, exhaustion, stale-ID rejection, and coalescing cases. It assumes each model method is atomic; it does not prove real mutex/atomic ordering, concurrent interleavings, TLS destructor order, error behavior of `ReleaseCollector::flush_with`, public-stat snapshot semantics, header lifetime, or memory cost. It models a zero-based raw-byte arena, not production headers/alignment or the current `Allocator` fields.

Before an unsafe chunk implementation, extend the model and test the selected contract with deterministic barriers: remote publication racing owner exit and reclaim; stale/duplicate release versus address reuse; nested collectors and destructor reentrancy; panic at every reclaim commit step; queue capacity/overflow; allocation failure and exhaustion; high-alignment and fragmented coalescing; and a live suballocation surviving owner exit. Check exact global and per-chunk accounting after every step. Then use a concurrency model checker (such as Loom for the queue/registry protocol), Miri, supported sanitizers, and stress tests. Prove no chunk enters `Reclaimable` or global free while any live or pending allocation, queue entry, or in-flight operation refers to it. Preserve checksum parity and run A2/A4/A5/B8/B10 plus B3/B5 at 1/2/4/8 workers where available only after the baseline.

Open decisions still blocking implementation: offset-to-chunk lookup under frozen layouts; exact live-byte snapshot with per-chunk state; queue representation and overflow; TLS destructor/reaper coordination; duplicate/ABA detection without a generation in the owner/header; and a chunk/metadata budget that satisfies the memory gates.
