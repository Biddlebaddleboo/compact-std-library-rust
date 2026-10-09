# V2.5 allocator phase 1: owner-affine chunk safety model

**Status:** phase 1 proposal for central review. No production `cage.rs` changes. The test-only model now rejects mutator-to-mutator chunk transfer and exposes the remote lookup/commit interleaving with owner retirement. This does not close the unsafe implementation gate.

## Candidate state and interface

Keep `CageAllocation<T>` at four bytes and `AllocationHeader` at 16 bytes. Put all chunk identity and synchronization in side metadata:

```text
ChunkRecord {
    generation,
    interval: [start, end),
    home: ThreadKey,
    phase: Active | Retiring | Reclaiming | Reclaimed | Poisoned,
    live_bytes, reserved_bytes, remote_pending_bytes,
    in_flight_remote,
    state: Mutex<ChunkState>,
}
```

`ThreadKey` is a stable runtime identity, not a reused OS thread ID. The registry is an interval index from chunk start to a stable `Arc<ChunkRecord>`. TLS contains a safe `Arc` handle to the active chunk; it contains no raw pointer into the TLS stack and is dropped after the owner path closes. The existing global allocator remains responsible for reservation, fallback, and final publication.

The operations are:

1. `reserve_chunk(home, size)`: acquire registry, then global allocator; reserve one aligned interval within per-thread and global slack caps; insert the record before returning its TLS handle. Failure leaves cursor, free lists, and counters unchanged.
2. `allocate_local(handle, layout)`: lock only chunk state; require `Active` and matching `home`; split a reserved extent, initialize the allocation header, then publish `Live` before unlocking. If the chunk has no compatible range, drop its lock before global fallback/refill.
3. `release_owner(offset)`: the private `Drop` path derives the block interval from the live header, finds and clones the stable registry handle under the registry mutex, releases the registry mutex, then locks the chunk. Commit `Live -> LocalPending` or `RemotePending` without running user code. The live count does not decrease while a release is pending. A release failure keeps the range live and quarantines the chunk; it must not erase the only release descriptor.
4. `drain_pending(record)`: under chunk state, validate each descriptor and move pending bytes into same-chunk reusable space. Remote pending bytes stay counted live until this commit.
5. `retire_owner(handle)`: clear the TLS active handle and flush its local collector; under the registry mutex change `Active -> Retiring`. No mutator may take over the chunk. Existing `Live` owners on other threads can still release through the record.
6. `reclaim(record)`: acquire locks in registry -> global allocator -> chunk-state order. Require `Retiring`, zero live and pending bytes, and zero in-flight remote pins. Change to `Reclaiming`, convert the entire interval including reserved slack to reclaimable, publish the entire interval globally free, then remove the registry entry and mark `Reclaimed`. Any failure before global publication leaves the record registered and quarantined/retryable.

Local operations take only chunk state. A remote lookup briefly takes registry state to clone the stable handle and increment `in_flight_remote`, then releases it before waiting on chunk state. A remote operation decrements the pin only after commit or cancellation. It never reacquires registry/global while holding chunk state. Registry removal and global publication are a single ordered reaper transaction. No user destructor, closure, or callback runs while allocator locks are held.

The exact counter equation over `INITIAL_CURSOR..cursor` is:

```text
live_bytes = Live + LocalPending + RemotePending
cursor - INITIAL_CURSOR = global_free + live_bytes + reserved_slack + reclaimable
```

External registry/queue allocations are reported separately. Cage-carved metadata is explicitly charged to a reservation or a separate metadata bucket. `used_bytes()` remains `live_bytes`, including pending extents; `remaining_bytes()` remains capacity minus live bytes, not a contiguous-allocation promise. `AllocatorStats` layout stays unchanged. A diagnostic snapshot must acquire registry, global state, and all chunk locks in stable interval order so its live/free/high-water values describe one coherent instant; adding public stat fields needs separate API review.

## Proof boundary

The model provides synthetic `AllocationId`s, so stale and duplicate releases are rejected after reuse. Production has no generation in the frozen owner/header. The proposed production proof therefore depends on an audit establishing that the private `release(offset)` path is reachable only from the unique `CageAllocation<T>::Drop` guard, that the owner is non-cloneable, and that unsafe callers cannot invoke release or manufacture a second owner under the documented safety contract. If that audit fails, side metadata must add per-allocation generations/ownership tracking before chunk reuse is allowed.

The model pin is acquired before owner retirement and stays outstanding while the corresponding extent is still `Live`. Reclaim rejects either a live/pending extent or an explicit pin. This demonstrates the required schedule abstractly; it does not prove Rust mutex, registry, `Arc`, atomic, or TLS ordering. The selected linearization points are: reservation at registry insertion after global reservation; allocation at header initialization plus `Reserved -> Live` under chunk state; release at `Live -> Pending`; reuse at `Pending -> Live`; retirement at registry-protected `Active -> Retiring`; reclamation at full-range global publication under the three ordered locks.

## Remaining gates before phase 2

| Gate | Required evidence | State |
| --- | --- | --- |
| Offset routing and owner provenance | Audit every private release caller, header read/write, and safe/unsafe owner construction; document malformed offsets and lookup miss behavior | Open |
| Registry pin/removal race | Deterministic schedule plus Loom-style model for lookup pin vs retire/reclaim; prove no stale registry interval or use-after-reuse | Model schedule added; synchronization proof open |
| Allocation commit and rollback | Prove headers cannot be observed before initialization; inject reservation, metadata allocation, and counter failures without changing free lists/cursor | Open |
| Remote pending representation | Bounded, no-allocation `Drop` path; queue/list capacity, overflow behavior, and descriptor preservation on poisoned locks | Open |
| Owner exit and reentrancy | Verify TLS destructor order, collector nesting, panic/drop paths, thread churn, handles moved to another thread, and reaper retries | Open |
| Lock order | Ensure every multi-lock path follows registry -> global -> chunk; prove no fallback/refill takes global while retaining chunk state | Proposed order; code audit open |
| Accounting/statistics | Exact per-chunk/global byte conservation and coherent `used_bytes`, `remaining_bytes`, stats snapshots during concurrent updates | Model equation defined; API/runtime proof open |
| ABA/duplicate release | Complete the unique-owner audit or add generation-bearing side metadata; test address reuse after release | Synthetic IDs only; production proof open |
| Memory bound | Select chunk size and caps; include record, index, queue, retained slack, fragmentation, idle RSS, and thread churn under the +2%/+5% gates | Open |
| Concurrency evidence | Deterministic barriers, model checker, randomized stress, supported sanitizers, and Miri-safe portions after implementation | Open |

Expected implementation tests include: remote pin vs owner exit and reclaim at every interleaving; duplicate/stale free followed by exact address reuse; local and remote queue exhaustion; failed reservation/release and poisoned mutex; header publication under unwind; high alignment and fragmented chunks; nested collectors and destructor reentrancy; an allocation held across home-thread exit; no overlapping live ranges; exact accounting after every step; and thread churn with idle-RSS recovery. Then run Miri on supported paths and compare the full required workload matrix. The host's two physical CPUs can establish one/two-worker behavior only; four/eight workers are oversubscription checks.

**Decision:** phase 2 is not safe to start. The model narrows the contract but does not close the production provenance, bounded pending descriptor, registry synchronization, lock-poison, stats, or memory-budget gates. Central architecture review and evidence for each open gate are required before `cage.rs` gains chunk code.
