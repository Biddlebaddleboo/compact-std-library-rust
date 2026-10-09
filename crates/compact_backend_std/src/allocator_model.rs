//! State-transition contract for allocator-owned cage bytes.
//!
//! This module is the design gate required by `PLAN_ALLOCATOR_CONCURRENCY.md`
//! ("Mandatory state-machine design gate") before any thread-local chunk or
//! reservation scheme may be implemented. It is documentation plus a small
//! executable model. It adds no runtime allocator behaviour, no `unsafe`, and
//! no fields to any retained or frozen layout; it is compiled only under
//! `cfg(test)` so it cannot affect production codegen.
//!
//! # Why this exists
//!
//! The production allocator serializes every mutation behind one process-wide
//! mutex. Deterministic measurement on the two-vCPU Neoverse host shows B10's
//! `parallel_allocate_drop_churn` performs exactly one lock acquisition per
//! allocation and one per release (16,000 acquisitions for 8,000
//! allocate/drop pairs, zero pending reuse, zero size-class hits, 8,000 cursor
//! fallbacks). The only changes that remove either acquisition are the gated
//! thread-local reservation/caching schemes. Those schemes are unsafe to build
//! without an explicit ownership contract, so this module records that
//! contract, the legal transitions, the invariants, and the questions a chunk
//! implementation must answer.
//!
//! # Byte states
//!
//! Every managed byte in the high-water prefix
//! `INITIAL_CURSOR..cursor` is in exactly one of the states in [`ByteState`].
//! The unallocated tail beyond `cursor` is outside this partition.
//! "Authority" names where the truth for a managed byte lives.
//!
//! | State | Meaning | Authority | Counted in |
//! | --- | --- | --- | --- |
//! | `Reserved` | Handed to one thread for future bump allocation; no header yet | the owning thread's chunk record | `reserved_bytes` (new, separate) |
//! | `Live` | Covered by exactly one `AllocationHeader` | the unique `CageAllocation<T>` handle; its holder may change threads when `T: Send` | `live_bytes` |
//! | `PendingRelease` | Handle dropped; held by the current thread's `ReleaseCollector` | the collector that received the release, which may differ from the former handle holder | `live_bytes` (unchanged) |
//! | `Free` | On the general free list or in a size-class cache | the global `Allocator` | `free_bytes` |
//! | `Reclaimable` | Unused slack in a reserved chunk at teardown | the tearing-down thread until it is published | `reclaimable_bytes` separately; then `free_bytes` after publication |
//!
//! # Legal transitions
//!
//! See [`transition_is_legal`] for the executable table. Production paths use
//! `Free -> Live`, `Live -> PendingRelease`, `PendingRelease -> Live` (same
//! collector only), `Live -> Free`, and `PendingRelease -> Free`. A live
//! handle may move between threads without changing its bytes; a remote drop
//! records the receiving collector as the `PendingRelease` owner. The chunk
//! design adds `Free -> Reserved`, `Reserved -> Live` (same thread),
//! `Reserved -> Reclaimable`, and `Reclaimable -> Free`. A `Reserved` owner
//! transfer is a separate whole-chunk operation after pending frees are
//! drained; the byte-only predicate below cannot validate that precondition.
//! Notably illegal:
//! `Free -> PendingRelease`, cross-thread `PendingRelease -> Live`,
//! cross-thread `Reserved -> Live`, `Reclaimable -> Live`, and
//! `Reserved -> Free` (a reserved chunk must pass through `Reclaimable` so
//! its live/pending check cannot be skipped).
//!
//! # Invariants a chunk implementation must preserve
//!
//! 1. Every byte has exactly one state and, when owned, exactly one owner.
//! 2. `live_bytes == sum(Live) + sum(PendingRelease)`.
//! 3. `live_bytes + free_bytes + reserved_bytes + reclaimable_bytes == cursor - INITIAL_CURSOR`.
//! 4. No byte is simultaneously `Free` and (`Live` | `PendingRelease` | `Reserved`).
//! 5. A chunk may become `Reclaimable` only once it holds zero `Live` and zero
//!    `PendingRelease` bytes. No chunk byte is ever globally reusable while any
//!    suballocation of that chunk is live.
//! 6. An allocation always returns bytes disjoint from every `Live`,
//!    `PendingRelease`, and `Reserved` byte.
//!
//! The small transition helpers below are supplemented by
//! [`hypothetical_chunk_model`], a deterministic sequential reference
//! protocol. That model accounts for ranges and chunk identities and tests
//! failure paths. It is hypothetical: remote publication is atomic at a model
//! method boundary, the reaper is an abstract owner, and the model does not
//! establish actual memory ordering, thread-local destructor behavior, or
//! production allocator safety. The production tests in `cage.rs` independently
//! exercise only the current mutex allocator; they do not call
//! [`check_no_overlap`] or model reserved chunks.
//!
//! # Open questions the chunk design must answer (do not implement without these)
//!
//! [`CHUNK_DESIGN_GAPS`] lists implementation questions the hypothetical
//! protocol cannot settle. They still need concrete answers and central review
//! before any unsafe chunk code lands.

/// The single state of one managed cage byte in the high-water prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ByteState {
    /// Reserved to one thread for future bump allocation; no header yet.
    Reserved { owner: ThreadKey },
    /// Covered by exactly one live `AllocationHeader`; this thread holds the
    /// unique `CageAllocation<T>` handle and can change if that handle moves
    /// across threads when `T: Send`.
    Live { owner: ThreadKey },
    /// Handle dropped, still held by this thread's `ReleaseCollector`; this
    /// can be a remote free from the thread that previously held `Live`.
    PendingRelease { owner: ThreadKey },
    /// Reachable from the general free list or a size-class cache.
    Free,
    /// Unused slack in a reserved chunk at teardown.
    Reclaimable,
}

/// Model-only thread identity. Production code has no such field.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ThreadKey(pub(crate) u8);

/// A half-open `[start, start + len)` range of cage bytes with one state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Block {
    pub(crate) start: u32,
    pub(crate) len: u32,
    pub(crate) state: ByteState,
}

impl Block {
    pub(crate) fn end(&self) -> u32 {
        self.start + self.len
    }

    pub(crate) fn overlaps(&self, other: &Block) -> bool {
        self.start < other.end() && other.start < self.end()
    }
}

/// Whether `from -> to` is a transition the contract permits.
///
/// Identity is legal so callers can express "unchanged" without special cases.
pub(crate) fn transition_is_legal(from: ByteState, to: ByteState) -> bool {
    use ByteState::{Free, Live, PendingRelease, Reclaimable, Reserved};
    if from == to {
        return true;
    }
    match (from, to) {
        // Production paths today. A live handle can move to another thread;
        // its drop may then enter that thread's collector (a remote free).
        (Free, Live { .. })
        | (Live { .. }, Live { .. })
        | (Live { .. }, PendingRelease { .. })
        | (Live { .. }, Free)
        | (PendingRelease { .. }, Free) => true,
        // Pending exact reuse and reservation activation are thread-local.
        // Cross-thread reuse of pending bytes must first publish them as Free.
        // Reserved ownership transfer is modeled only by a whole-chunk method
        // that drains pending entries before changing any reserved owners.
        (
            PendingRelease {
                owner: release_owner,
            },
            Live { owner: live_owner },
        ) if release_owner == live_owner => true,
        // Added by the gated chunk design.
        (Free, Reserved { .. }) | (Reserved { .. }, Reclaimable) | (Reclaimable, Free) => true,
        (
            Reserved {
                owner: reservation_owner,
            },
            Live { owner: live_owner },
        ) if reservation_owner == live_owner => true,
        _ => false,
    }
}

/// Return true when no two blocks overlap.
///
/// This is the executable form of invariant 4/6: a byte cannot be in two
/// states, so an allocation or release that produces overlapping ranges has
/// reused a live or pending byte.
pub(crate) fn check_no_overlap(blocks: &[Block]) -> bool {
    for (index, left) in blocks.iter().enumerate() {
        for right in &blocks[index + 1..] {
            if left.overlaps(right) {
                return false;
            }
        }
    }
    true
}

/// Reachable states a chunk teardown must prove empty before `Reclaimable`.
pub(crate) const LIVE_STATES: [ByteState; 2] = [
    ByteState::Live {
        owner: ThreadKey(0),
    },
    ByteState::PendingRelease {
        owner: ThreadKey(0),
    },
];

/// Implementation questions that the sequential hypothetical model cannot
/// settle. Each needs a concrete answer before any unsafe chunk implementation.
pub(crate) const CHUNK_DESIGN_GAPS: [&str; 6] = [
    "actual remote-queue publication ordering and the synchronization primitive that makes it safe",
    "thread-local destructor and reaper integration with in-flight operations on real threads",
    "nested collectors, destructor reentrancy, and errors while flushing actual release batches",
    "panic, allocation failure, and abort behavior while real chunk metadata is being reclaimed",
    "mapping reserved/reclaimable accounting into public stats without changing frozen layouts",
    "concurrent stress, Miri/model-checker evidence, and central review for a concrete unsafe implementation",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(id: u8) -> ThreadKey {
        ThreadKey(id)
    }

    fn block(start: u32, len: u32, state: ByteState) -> Block {
        Block { start, len, state }
    }

    #[test]
    fn production_transitions_are_legal() {
        for (from, to) in [
            (ByteState::Free, ByteState::Live { owner: owner(1) }),
            (
                ByteState::Live { owner: owner(1) },
                ByteState::PendingRelease { owner: owner(1) },
            ),
            (
                ByteState::PendingRelease { owner: owner(1) },
                ByteState::Live { owner: owner(1) },
            ),
            (ByteState::Live { owner: owner(1) }, ByteState::Free),
            (
                ByteState::PendingRelease { owner: owner(1) },
                ByteState::Free,
            ),
        ] {
            assert!(
                transition_is_legal(from, to),
                "{from:?} -> {to:?} should be legal"
            );
        }
    }

    #[test]
    fn remote_drop_and_live_handle_transfer_are_representable() {
        assert!(transition_is_legal(
            ByteState::Live { owner: owner(1) },
            ByteState::Live { owner: owner(2) }
        ));
        assert!(transition_is_legal(
            ByteState::Live { owner: owner(1) },
            ByteState::PendingRelease { owner: owner(2) }
        ));
    }

    #[test]
    fn pending_reuse_and_reserved_activation_stay_with_their_thread() {
        assert!(transition_is_legal(
            ByteState::PendingRelease { owner: owner(1) },
            ByteState::Live { owner: owner(1) }
        ));
        assert!(!transition_is_legal(
            ByteState::PendingRelease { owner: owner(1) },
            ByteState::Live { owner: owner(2) }
        ));
        assert!(transition_is_legal(
            ByteState::Reserved { owner: owner(1) },
            ByteState::Live { owner: owner(1) }
        ));
        assert!(!transition_is_legal(
            ByteState::Reserved { owner: owner(1) },
            ByteState::Live { owner: owner(2) }
        ));
    }

    #[test]
    fn chunk_transitions_are_legal_but_must_pass_through_reclaimable() {
        assert!(transition_is_legal(
            ByteState::Free,
            ByteState::Reserved { owner: owner(2) }
        ));
        assert!(transition_is_legal(
            ByteState::Reserved { owner: owner(2) },
            ByteState::Live { owner: owner(2) }
        ));
        assert!(transition_is_legal(
            ByteState::Reserved { owner: owner(2) },
            ByteState::Reclaimable
        ));
        assert!(transition_is_legal(ByteState::Reclaimable, ByteState::Free));
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        for (from, to) in [
            // A freed byte cannot appear in a batch without a live owner.
            (
                ByteState::Free,
                ByteState::PendingRelease { owner: owner(1) },
            ),
            // A live byte cannot become reservation slack; it must be released.
            (
                ByteState::Live { owner: owner(1) },
                ByteState::Reserved { owner: owner(1) },
            ),
            // Slack must be re-reserved or reused through a live header.
            (ByteState::Reclaimable, ByteState::Live { owner: owner(1) }),
            // Reservation slack cannot bypass the live/pending check.
            (ByteState::Reserved { owner: owner(1) }, ByteState::Free),
        ] {
            assert!(
                !transition_is_legal(from, to),
                "{from:?} -> {to:?} must be illegal"
            );
        }
    }

    #[test]
    fn overlap_checker_rejects_double_owned_bytes() {
        let live = block(32, 16, ByteState::Live { owner: owner(1) });
        let free = block(40, 16, ByteState::Free);
        assert!(!check_no_overlap(&[live, free]));

        let disjoint = block(64, 16, ByteState::Free);
        assert!(check_no_overlap(&[live, disjoint]));
    }

    #[test]
    fn overlap_checker_rejects_a_duplicate_allocation() {
        let first = block(32, 24, ByteState::Live { owner: owner(1) });
        let second = block(32, 24, ByteState::Live { owner: owner(2) });
        assert!(
            !check_no_overlap(&[first, second]),
            "two owners of the same bytes must be rejected"
        );
    }

    #[test]
    fn live_and_pending_states_are_the_ones_a_chunk_teardown_must_clear() {
        assert_eq!(LIVE_STATES.len(), 2);
        for state in LIVE_STATES {
            assert!(matches!(
                state,
                ByteState::Live { .. } | ByteState::PendingRelease { .. }
            ));
        }
    }

    #[test]
    fn every_chunk_design_gap_is_documented() {
        assert_eq!(CHUNK_DESIGN_GAPS.len(), 6);
        assert!(CHUNK_DESIGN_GAPS.iter().all(|gap| !gap.is_empty()));
    }
}

/// Sequential, hypothetical chunk protocol used only as a reference model.
///
/// This is deliberately not connected to `cage.rs`. A successful method call
/// is the abstract linearization point for its state transition; in particular,
/// remote publication is assumed to be atomic and visible to a later flush.
/// Its arena is zero-based and uses raw byte lengths, so it omits the production
/// initial cursor, headers, alignment, and allocator metadata. No atomic
/// ordering, TLS destructor, or production safety claim follows from these
/// tests.
#[cfg(test)]
mod hypothetical_chunk_model {
    use super::ThreadKey;
    use std::collections::{BTreeMap, BTreeSet};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// Abstract global reaper that accepts reservations from exiting threads.
    const REAPER: ThreadKey = ThreadKey(u8::MAX);

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct ChunkId(u32);

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct AllocationId(u32);

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ChunkPhase {
        Active,
        Retiring,
        Reclaiming,
        Reclaimed,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum PendingRoute {
        LocalCollector,
        RemoteQueue,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ExtentState {
        GlobalFree,
        Reserved {
            owner: ThreadKey,
        },
        Live {
            holder: ThreadKey,
            allocation: AllocationId,
        },
        PendingRelease {
            owner: ThreadKey,
            allocation: AllocationId,
            route: PendingRoute,
        },
        Reclaimable,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Extent {
        start: u32,
        len: u32,
        chunk: Option<ChunkId>,
        state: ExtentState,
    }

    impl Extent {
        fn end(self) -> u32 {
            self.start + self.len
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Chunk {
        id: ChunkId,
        start: u32,
        len: u32,
        owner: ThreadKey,
        phase: ChunkPhase,
    }

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    struct Accounting {
        live_bytes: u32,
        reserved_bytes: u32,
        free_bytes: u32,
        reclaimable_bytes: u32,
    }

    impl Accounting {
        fn total(self) -> u32 {
            self.live_bytes + self.reserved_bytes + self.free_bytes + self.reclaimable_bytes
        }

        fn bucket_mut(&mut self, bucket: Bucket) -> &mut u32 {
            match bucket {
                Bucket::Live => &mut self.live_bytes,
                Bucket::Reserved => &mut self.reserved_bytes,
                Bucket::Free => &mut self.free_bytes,
                Bucket::Reclaimable => &mut self.reclaimable_bytes,
            }
        }

        fn add(&mut self, bucket: Bucket, bytes: u32) {
            let value = self.bucket_mut(bucket);
            *value = value.checked_add(bytes).expect("model accounting overflow");
        }

        fn subtract(&mut self, bucket: Bucket, bytes: u32) {
            let value = self.bucket_mut(bucket);
            *value = value
                .checked_sub(bytes)
                .expect("model accounting underflow");
        }

        fn move_bytes(&mut self, from: ExtentState, to: ExtentState, bytes: u32) {
            let from = bucket(from);
            let to = bucket(to);
            if from != to {
                self.subtract(from, bytes);
                self.add(to, bytes);
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Bucket {
        Live,
        Reserved,
        Free,
        Reclaimable,
    }

    fn bucket(state: ExtentState) -> Bucket {
        match state {
            ExtentState::Live { .. } | ExtentState::PendingRelease { .. } => Bucket::Live,
            ExtentState::Reserved { .. } => Bucket::Reserved,
            ExtentState::GlobalFree => Bucket::Free,
            ExtentState::Reclaimable => Bucket::Reclaimable,
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ModelError {
        InvalidSize,
        AllocationExhausted,
        FragmentedReservation,
        UnknownChunk,
        UnknownOrStaleAllocation,
        WrongChunkOwner,
        WrongAllocationHolder,
        WrongQueueOwner,
        ChunkNotActive,
        ChunkBusy,
        WrongPhase,
        AlreadyReclaimed,
        ThreadExited,
        SizeMismatch,
        IdExhausted,
        InvariantViolation,
    }

    struct HypotheticalAllocator {
        capacity: u32,
        cursor: u32,
        next_chunk: u32,
        next_allocation: u32,
        extents: Vec<Extent>,
        chunks: BTreeMap<ChunkId, Chunk>,
        exited_threads: BTreeSet<ThreadKey>,
        accounting: Accounting,
    }

    impl HypotheticalAllocator {
        fn new(capacity: u32) -> Self {
            Self {
                capacity,
                cursor: 0,
                next_chunk: 0,
                next_allocation: 0,
                extents: Vec::new(),
                chunks: BTreeMap::new(),
                exited_threads: BTreeSet::new(),
                accounting: Accounting::default(),
            }
        }

        fn reserve_chunk(&mut self, owner: ThreadKey, len: u32) -> Result<ChunkId, ModelError> {
            self.ensure_thread_active(owner)?;
            if len == 0 {
                return Err(ModelError::InvalidSize);
            }

            self.coalesce_global_free();
            let reusable = self
                .extents
                .iter()
                .position(|extent| extent.state == ExtentState::GlobalFree && extent.len >= len);
            let (start, bump_end) = if let Some(index) = reusable {
                (self.extents[index].start, None)
            } else {
                let end = self
                    .cursor
                    .checked_add(len)
                    .filter(|end| *end <= self.capacity)
                    .ok_or(ModelError::AllocationExhausted)?;
                (self.cursor, Some(end))
            };

            let id = ChunkId(self.next_chunk);
            let next_chunk = self
                .next_chunk
                .checked_add(1)
                .ok_or(ModelError::IdExhausted)?;
            self.next_chunk = next_chunk;
            if let Some(end) = bump_end {
                self.cursor = end;
            }

            if let Some(index) = reusable {
                let free = self.extents.remove(index);
                debug_assert_eq!(free.start, start);
                debug_assert_eq!(free.state, ExtentState::GlobalFree);
                self.accounting.move_bytes(
                    ExtentState::GlobalFree,
                    ExtentState::Reserved { owner },
                    len,
                );
                if free.len > len {
                    self.extents.push(Extent {
                        start: start + len,
                        len: free.len - len,
                        chunk: None,
                        state: ExtentState::GlobalFree,
                    });
                }
            } else {
                self.accounting.add(Bucket::Reserved, len);
            }
            self.extents.push(Extent {
                start,
                len,
                chunk: Some(id),
                state: ExtentState::Reserved { owner },
            });
            self.chunks.insert(
                id,
                Chunk {
                    id,
                    start,
                    len,
                    owner,
                    phase: ChunkPhase::Active,
                },
            );
            self.sort_extents();
            self.assert_valid();
            Ok(id)
        }

        fn allocate_from_chunk(
            &mut self,
            chunk_id: ChunkId,
            owner: ThreadKey,
            len: u32,
        ) -> Result<AllocationId, ModelError> {
            self.ensure_thread_active(owner)?;
            if len == 0 {
                return Err(ModelError::InvalidSize);
            }
            let chunk = self.chunk(chunk_id)?;
            if chunk.phase != ChunkPhase::Active {
                return Err(ModelError::ChunkNotActive);
            }
            if chunk.owner != owner {
                return Err(ModelError::WrongChunkOwner);
            }
            let index = self
                .extents
                .iter()
                .position(|extent| {
                    extent.chunk == Some(chunk_id)
                        && extent.state == (ExtentState::Reserved { owner })
                        && extent.len >= len
                })
                .ok_or(ModelError::FragmentedReservation)?;
            let old = self.extents[index];
            let allocation = self.fresh_allocation_id()?;
            self.extents.remove(index);
            self.accounting.move_bytes(
                old.state,
                ExtentState::Live {
                    holder: owner,
                    allocation,
                },
                len,
            );
            self.extents.push(Extent {
                start: old.start,
                len,
                chunk: Some(chunk_id),
                state: ExtentState::Live {
                    holder: owner,
                    allocation,
                },
            });
            if old.len > len {
                self.extents.push(Extent {
                    start: old.start + len,
                    len: old.len - len,
                    chunk: Some(chunk_id),
                    state: ExtentState::Reserved { owner },
                });
            }
            self.sort_extents();
            self.assert_valid();
            Ok(allocation)
        }

        fn transfer_live_handle(
            &mut self,
            allocation: AllocationId,
            from: ThreadKey,
            to: ThreadKey,
        ) -> Result<(), ModelError> {
            self.ensure_thread_active(from)?;
            if to == REAPER {
                return Err(ModelError::WrongAllocationHolder);
            }
            self.ensure_thread_active(to)?;
            let extent = self
                .extent_for_allocation_mut(allocation)
                .ok_or(ModelError::UnknownOrStaleAllocation)?;
            match extent.state {
                ExtentState::Live {
                    holder,
                    allocation: id,
                } if holder == from && id == allocation => {
                    extent.state = ExtentState::Live {
                        holder: to,
                        allocation,
                    };
                }
                ExtentState::Live { .. } => return Err(ModelError::WrongAllocationHolder),
                _ => return Err(ModelError::UnknownOrStaleAllocation),
            }
            self.assert_valid();
            Ok(())
        }

        /// Dropping on a non-home thread publishes the extent to the chunk
        /// owner's remote queue. It remains pending and counted live until a
        /// matching flush runs.
        fn release_allocation(
            &mut self,
            allocation: AllocationId,
            dropper: ThreadKey,
        ) -> Result<(), ModelError> {
            self.ensure_thread_active(dropper)?;
            let index = self
                .extent_for_allocation(allocation)
                .ok_or(ModelError::UnknownOrStaleAllocation)?;
            let extent = self.extents[index];
            let holder = match extent.state {
                ExtentState::Live { holder, .. } => holder,
                _ => return Err(ModelError::UnknownOrStaleAllocation),
            };
            if holder != dropper {
                return Err(ModelError::WrongAllocationHolder);
            }
            let chunk_id = extent.chunk.ok_or(ModelError::InvariantViolation)?;
            let chunk = self.chunk(chunk_id)?;
            let local = chunk.phase == ChunkPhase::Active && chunk.owner == dropper;
            let route = if local {
                PendingRoute::LocalCollector
            } else {
                PendingRoute::RemoteQueue
            };
            self.extents[index].state = ExtentState::PendingRelease {
                owner: chunk.owner,
                allocation,
                route,
            };
            // Live and pending bytes share the public live accounting bucket.
            self.assert_valid();
            Ok(())
        }

        /// Exact pending reuse is local to the collector that owns the entry.
        /// A new allocation ID prevents an old handle from freeing reused bytes.
        fn reuse_pending_exact(
            &mut self,
            old_allocation: AllocationId,
            collector: ThreadKey,
            requested_len: u32,
        ) -> Result<AllocationId, ModelError> {
            self.ensure_thread_active(collector)?;
            let index = self
                .extent_for_allocation(old_allocation)
                .ok_or(ModelError::UnknownOrStaleAllocation)?;
            let extent = self.extents[index];
            match extent.state {
                ExtentState::PendingRelease {
                    owner,
                    route: PendingRoute::LocalCollector,
                    ..
                } if owner == collector => {}
                ExtentState::PendingRelease { .. } => return Err(ModelError::WrongQueueOwner),
                _ => return Err(ModelError::UnknownOrStaleAllocation),
            }
            if extent.len != requested_len {
                return Err(ModelError::SizeMismatch);
            }
            let chunk_id = extent.chunk.ok_or(ModelError::InvariantViolation)?;
            let chunk = self.chunk(chunk_id)?;
            if chunk.phase != ChunkPhase::Active || chunk.owner != collector {
                return Err(ModelError::WrongQueueOwner);
            }
            let new_allocation = self.fresh_allocation_id()?;
            self.extents[index].state = ExtentState::Live {
                holder: collector,
                allocation: new_allocation,
            };
            // Pending -> live preserves the byte count in the live bucket.
            self.assert_valid();
            Ok(new_allocation)
        }

        fn flush_local(&mut self, collector: ThreadKey) -> Result<usize, ModelError> {
            self.ensure_thread_active(collector)?;
            let indices = self
                .extents
                .iter()
                .enumerate()
                .filter_map(|(index, extent)| match extent.state {
                    ExtentState::PendingRelease {
                        owner,
                        route: PendingRoute::LocalCollector,
                        ..
                    } if owner == collector => Some(index),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let mut chunks = BTreeSet::new();
            for index in &indices {
                let chunk_id = self.extents[*index]
                    .chunk
                    .ok_or(ModelError::InvariantViolation)?;
                let chunk = self.chunk(chunk_id)?;
                if chunk.owner != collector || chunk.phase != ChunkPhase::Active {
                    return Err(ModelError::WrongQueueOwner);
                }
                chunks.insert(chunk_id);
                let old = self.extents[*index].state;
                self.accounting.move_bytes(
                    old,
                    ExtentState::Reserved { owner: collector },
                    self.extents[*index].len,
                );
                self.extents[*index].state = ExtentState::Reserved { owner: collector };
            }
            for chunk_id in chunks {
                self.coalesce_reserved(chunk_id);
            }
            self.assert_valid();
            Ok(indices.len())
        }

        fn flush_remote(
            &mut self,
            chunk_id: ChunkId,
            queue_owner: ThreadKey,
        ) -> Result<usize, ModelError> {
            self.ensure_queue_owner(queue_owner)?;
            let chunk = self.chunk(chunk_id)?;
            if chunk.owner != queue_owner {
                return Err(ModelError::WrongQueueOwner);
            }
            if !matches!(chunk.phase, ChunkPhase::Active | ChunkPhase::Retiring) {
                return Err(ModelError::ChunkNotActive);
            }
            let indices = self
                .extents
                .iter()
                .enumerate()
                .filter_map(|(index, extent)| match extent.state {
                    ExtentState::PendingRelease {
                        owner,
                        route: PendingRoute::RemoteQueue,
                        ..
                    } if extent.chunk == Some(chunk_id) && owner == queue_owner => Some(index),
                    _ => None,
                })
                .collect::<Vec<_>>();
            for index in &indices {
                let old = self.extents[*index].state;
                self.accounting.move_bytes(
                    old,
                    ExtentState::Reserved { owner: queue_owner },
                    self.extents[*index].len,
                );
                self.extents[*index].state = ExtentState::Reserved { owner: queue_owner };
            }
            self.coalesce_reserved(chunk_id);
            self.assert_valid();
            Ok(indices.len())
        }

        fn transfer_chunk(
            &mut self,
            chunk_id: ChunkId,
            from: ThreadKey,
            to: ThreadKey,
        ) -> Result<(), ModelError> {
            self.ensure_thread_active(from)?;
            if to == REAPER {
                return Err(ModelError::WrongChunkOwner);
            }
            self.ensure_thread_active(to)?;
            let chunk = self.chunk(chunk_id)?;
            if chunk.phase != ChunkPhase::Active {
                return Err(ModelError::ChunkNotActive);
            }
            if chunk.owner != from {
                return Err(ModelError::WrongChunkOwner);
            }
            if self.has_pending(chunk_id) {
                return Err(ModelError::ChunkBusy);
            }
            self.chunks.get_mut(&chunk_id).unwrap().owner = to;
            for extent in &mut self.extents {
                if extent.chunk == Some(chunk_id)
                    && matches!(extent.state, ExtentState::Reserved { .. })
                {
                    extent.state = ExtentState::Reserved { owner: to };
                }
            }
            self.coalesce_reserved(chunk_id);
            self.assert_valid();
            Ok(())
        }

        /// A thread exit drops handles it still holds, flushes its collectors,
        /// transfers its chunks to the abstract reaper, and reclaims only empty
        /// chunks. Live handles already moved to other threads keep chunks pinned.
        fn thread_exit(&mut self, thread: ThreadKey) -> Result<(), ModelError> {
            self.ensure_thread_active(thread)?;
            let held = self
                .extents
                .iter()
                .filter_map(|extent| match extent.state {
                    ExtentState::Live { holder, allocation } if holder == thread => {
                        Some(allocation)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            for allocation in held {
                self.release_allocation(allocation, thread)?;
            }
            self.flush_local(thread)?;

            let owned_chunks = self
                .chunks
                .values()
                .filter(|chunk| {
                    chunk.owner == thread
                        && matches!(chunk.phase, ChunkPhase::Active | ChunkPhase::Reclaiming)
                })
                .map(|chunk| chunk.id)
                .collect::<Vec<_>>();
            for chunk_id in owned_chunks {
                let phase = self.chunk(chunk_id)?.phase;
                if phase == ChunkPhase::Active {
                    self.flush_remote(chunk_id, thread)?;
                }
                {
                    let chunk = self.chunks.get_mut(&chunk_id).unwrap();
                    chunk.owner = REAPER;
                    if phase == ChunkPhase::Active {
                        chunk.phase = ChunkPhase::Retiring;
                    }
                }
                for extent in &mut self.extents {
                    if extent.chunk == Some(chunk_id)
                        && matches!(extent.state, ExtentState::Reserved { .. })
                    {
                        extent.state = ExtentState::Reserved { owner: REAPER };
                    }
                }
                self.coalesce_reserved(chunk_id);
                if !self.has_live_or_pending(chunk_id) {
                    self.teardown_chunk(chunk_id)?;
                }
                self.assert_valid();
            }
            self.exited_threads.insert(thread);
            self.assert_valid();
            Ok(())
        }

        fn begin_teardown(&mut self, chunk_id: ChunkId) -> Result<(), ModelError> {
            let chunk = self.chunk(chunk_id)?;
            if chunk.phase == ChunkPhase::Reclaimed {
                return Err(ModelError::AlreadyReclaimed);
            }
            if self.has_live_or_pending(chunk_id) {
                return Err(ModelError::ChunkBusy);
            }
            if !matches!(
                chunk.phase,
                ChunkPhase::Active | ChunkPhase::Retiring | ChunkPhase::Reclaiming
            ) {
                return Err(ModelError::WrongPhase);
            }
            self.chunks.get_mut(&chunk_id).unwrap().phase = ChunkPhase::Reclaiming;
            self.assert_valid();
            Ok(())
        }

        fn mark_chunk_reclaimable(&mut self, chunk_id: ChunkId) -> Result<(), ModelError> {
            let chunk = self.chunk(chunk_id)?;
            if chunk.phase != ChunkPhase::Reclaiming {
                return Err(ModelError::WrongPhase);
            }
            if self.has_live_or_pending(chunk_id) {
                return Err(ModelError::ChunkBusy);
            }
            let indices = self
                .extents
                .iter()
                .enumerate()
                .filter_map(|(index, extent)| (extent.chunk == Some(chunk_id)).then_some(index))
                .collect::<Vec<_>>();
            let all_reserved = indices
                .iter()
                .all(|index| matches!(self.extents[*index].state, ExtentState::Reserved { .. }));
            let all_reclaimable = indices
                .iter()
                .all(|index| self.extents[*index].state == ExtentState::Reclaimable);
            if indices.is_empty() || (!all_reserved && !all_reclaimable) {
                return Err(ModelError::WrongPhase);
            }
            if all_reserved {
                for index in indices {
                    let old = self.extents[index].state;
                    self.accounting.move_bytes(
                        old,
                        ExtentState::Reclaimable,
                        self.extents[index].len,
                    );
                    self.extents[index].state = ExtentState::Reclaimable;
                }
            }
            self.assert_valid();
            Ok(())
        }

        fn publish_reclaimable_chunk(&mut self, chunk_id: ChunkId) -> Result<(), ModelError> {
            let chunk = self.chunk(chunk_id)?;
            if chunk.phase != ChunkPhase::Reclaiming {
                return Err(ModelError::WrongPhase);
            }
            if self.has_live_or_pending(chunk_id) {
                return Err(ModelError::ChunkBusy);
            }
            let indices = self
                .extents
                .iter()
                .enumerate()
                .filter_map(|(index, extent)| (extent.chunk == Some(chunk_id)).then_some(index))
                .collect::<Vec<_>>();
            if indices.is_empty()
                || indices
                    .iter()
                    .any(|index| self.extents[*index].state != ExtentState::Reclaimable)
            {
                return Err(ModelError::WrongPhase);
            }
            for index in indices {
                let len = self.extents[index].len;
                self.accounting
                    .move_bytes(ExtentState::Reclaimable, ExtentState::GlobalFree, len);
                self.extents[index].chunk = None;
                self.extents[index].state = ExtentState::GlobalFree;
            }
            self.chunks.get_mut(&chunk_id).unwrap().phase = ChunkPhase::Reclaimed;
            self.coalesce_global_free();
            self.contract_free_tail();
            self.assert_valid();
            Ok(())
        }

        fn teardown_chunk_with_hook(
            &mut self,
            chunk_id: ChunkId,
            after_begin: impl FnOnce(),
        ) -> Result<(), ModelError> {
            self.begin_teardown(chunk_id)?;
            // The hook stands for an unwind point. No bytes are marked
            // reclaimable or published before it returns.
            after_begin();
            self.mark_chunk_reclaimable(chunk_id)?;
            self.publish_reclaimable_chunk(chunk_id)
        }

        fn teardown_chunk(&mut self, chunk_id: ChunkId) -> Result<(), ModelError> {
            self.teardown_chunk_with_hook(chunk_id, || {})
        }

        fn has_live_or_pending(&self, chunk_id: ChunkId) -> bool {
            self.extents.iter().any(|extent| {
                extent.chunk == Some(chunk_id)
                    && matches!(
                        extent.state,
                        ExtentState::Live { .. } | ExtentState::PendingRelease { .. }
                    )
            })
        }

        fn has_pending(&self, chunk_id: ChunkId) -> bool {
            self.extents.iter().any(|extent| {
                extent.chunk == Some(chunk_id)
                    && matches!(extent.state, ExtentState::PendingRelease { .. })
            })
        }

        fn accounting(&self) -> Accounting {
            self.accounting
        }

        fn chunk_accounting(&self, chunk_id: ChunkId) -> Result<Accounting, ModelError> {
            self.chunk(chunk_id)?;
            let mut accounting = Accounting::default();
            for extent in self
                .extents
                .iter()
                .filter(|extent| extent.chunk == Some(chunk_id))
            {
                accounting.add(bucket(extent.state), extent.len);
            }
            Ok(accounting)
        }

        fn chunk(&self, id: ChunkId) -> Result<Chunk, ModelError> {
            self.chunks
                .get(&id)
                .copied()
                .ok_or(ModelError::UnknownChunk)
        }

        fn extent_for_allocation(&self, id: AllocationId) -> Option<usize> {
            self.extents.iter().position(|extent| match extent.state {
                ExtentState::Live { allocation, .. }
                | ExtentState::PendingRelease { allocation, .. } => allocation == id,
                _ => false,
            })
        }

        fn extent_for_allocation_mut(&mut self, id: AllocationId) -> Option<&mut Extent> {
            let index = self.extent_for_allocation(id)?;
            self.extents.get_mut(index)
        }

        fn fresh_allocation_id(&mut self) -> Result<AllocationId, ModelError> {
            let id = AllocationId(self.next_allocation);
            self.next_allocation = self
                .next_allocation
                .checked_add(1)
                .ok_or(ModelError::IdExhausted)?;
            Ok(id)
        }

        fn ensure_thread_active(&self, thread: ThreadKey) -> Result<(), ModelError> {
            if thread == REAPER || self.exited_threads.contains(&thread) {
                Err(ModelError::ThreadExited)
            } else {
                Ok(())
            }
        }

        fn ensure_queue_owner(&self, thread: ThreadKey) -> Result<(), ModelError> {
            if thread != REAPER && self.exited_threads.contains(&thread) {
                Err(ModelError::ThreadExited)
            } else {
                Ok(())
            }
        }

        fn coalesce_reserved(&mut self, chunk_id: ChunkId) {
            self.sort_extents();
            let mut merged: Vec<Extent> = Vec::with_capacity(self.extents.len());
            for extent in self.extents.drain(..) {
                if let Some(previous) = merged.last_mut() {
                    if previous.end() == extent.start
                        && previous.chunk == Some(chunk_id)
                        && extent.chunk == Some(chunk_id)
                        && matches!(previous.state, ExtentState::Reserved { .. })
                        && previous.state == extent.state
                    {
                        previous.len = previous
                            .len
                            .checked_add(extent.len)
                            .expect("model extent overflow");
                        continue;
                    }
                }
                merged.push(extent);
            }
            self.extents = merged;
        }

        fn coalesce_global_free(&mut self) {
            self.sort_extents();
            let mut merged: Vec<Extent> = Vec::with_capacity(self.extents.len());
            for extent in self.extents.drain(..) {
                if let Some(previous) = merged.last_mut() {
                    if previous.end() == extent.start
                        && previous.chunk.is_none()
                        && extent.chunk.is_none()
                        && previous.state == ExtentState::GlobalFree
                        && extent.state == ExtentState::GlobalFree
                    {
                        previous.len = previous
                            .len
                            .checked_add(extent.len)
                            .expect("model extent overflow");
                        continue;
                    }
                }
                merged.push(extent);
            }
            self.extents = merged;
        }

        fn contract_free_tail(&mut self) {
            while let Some(last) = self.extents.last().copied() {
                if last.state != ExtentState::GlobalFree || last.end() != self.cursor {
                    break;
                }
                self.extents.pop();
                self.cursor = last.start;
                self.accounting.subtract(Bucket::Free, last.len);
            }
        }

        fn sort_extents(&mut self) {
            self.extents.sort_unstable_by_key(|extent| extent.start);
        }

        fn ledger_accounting(&self) -> Accounting {
            let mut computed = Accounting::default();
            for extent in &self.extents {
                computed.add(bucket(extent.state), extent.len);
            }
            computed
        }

        fn validate(&self) -> Result<(), ModelError> {
            if self.cursor > self.capacity || self.accounting != self.ledger_accounting() {
                return Err(ModelError::InvariantViolation);
            }
            if self.accounting.total() != self.cursor {
                return Err(ModelError::InvariantViolation);
            }

            let mut next = 0_u32;
            let mut allocation_ids = BTreeSet::new();
            for (index, extent) in self.extents.iter().enumerate() {
                let end = extent
                    .start
                    .checked_add(extent.len)
                    .ok_or(ModelError::InvariantViolation)?;
                if extent.len == 0 || extent.start != next || end > self.cursor {
                    return Err(ModelError::InvariantViolation);
                }
                next = end;
                if index > 0
                    && self.extents[index - 1].state == ExtentState::GlobalFree
                    && extent.state == ExtentState::GlobalFree
                {
                    return Err(ModelError::InvariantViolation);
                }

                match extent.state {
                    ExtentState::GlobalFree => {
                        if extent.chunk.is_some() {
                            return Err(ModelError::InvariantViolation);
                        }
                    }
                    state => {
                        let chunk_id = extent.chunk.ok_or(ModelError::InvariantViolation)?;
                        let chunk = self
                            .chunks
                            .get(&chunk_id)
                            .ok_or(ModelError::InvariantViolation)?;
                        if chunk.phase == ChunkPhase::Reclaimed {
                            return Err(ModelError::InvariantViolation);
                        }
                        match state {
                            ExtentState::Reserved { owner } => {
                                if owner != chunk.owner {
                                    return Err(ModelError::InvariantViolation);
                                }
                            }
                            ExtentState::Live { holder, allocation } => {
                                self.ensure_thread_active(holder)?;
                                if !allocation_ids.insert(allocation) {
                                    return Err(ModelError::InvariantViolation);
                                }
                            }
                            ExtentState::PendingRelease {
                                owner,
                                allocation,
                                route,
                            } => {
                                if owner != chunk.owner || !allocation_ids.insert(allocation) {
                                    return Err(ModelError::InvariantViolation);
                                }
                                match route {
                                    PendingRoute::LocalCollector
                                        if chunk.phase == ChunkPhase::Active => {}
                                    PendingRoute::RemoteQueue
                                        if matches!(
                                            chunk.phase,
                                            ChunkPhase::Active | ChunkPhase::Retiring
                                        ) => {}
                                    _ => return Err(ModelError::InvariantViolation),
                                }
                            }
                            ExtentState::Reclaimable => {
                                if chunk.phase != ChunkPhase::Reclaiming {
                                    return Err(ModelError::InvariantViolation);
                                }
                            }
                            ExtentState::GlobalFree => unreachable!(),
                        }
                    }
                }
            }
            if next != self.cursor {
                return Err(ModelError::InvariantViolation);
            }

            let mut active_ranges = Vec::new();
            for (id, chunk) in &self.chunks {
                if chunk.id != *id || chunk.len == 0 {
                    return Err(ModelError::InvariantViolation);
                }
                if chunk.phase == ChunkPhase::Reclaimed {
                    if self.extents.iter().any(|extent| extent.chunk == Some(*id)) {
                        return Err(ModelError::InvariantViolation);
                    }
                    continue;
                }
                self.ensure_queue_owner(chunk.owner)?;
                let chunk_end = chunk
                    .start
                    .checked_add(chunk.len)
                    .ok_or(ModelError::InvariantViolation)?;
                let mut position = chunk.start;
                let mut has_reserved = false;
                let mut has_reclaimable = false;
                for extent in self
                    .extents
                    .iter()
                    .filter(|extent| extent.chunk == Some(*id))
                {
                    if extent.start != position || extent.end() > chunk_end {
                        return Err(ModelError::InvariantViolation);
                    }
                    position = extent.end();
                    has_reserved |= matches!(extent.state, ExtentState::Reserved { .. });
                    has_reclaimable |= extent.state == ExtentState::Reclaimable;
                }
                if position != chunk_end {
                    return Err(ModelError::InvariantViolation);
                }
                if self.chunk_accounting(*id)?.total() != chunk.len {
                    return Err(ModelError::InvariantViolation);
                }
                if chunk.phase == ChunkPhase::Reclaiming && has_reserved && has_reclaimable {
                    return Err(ModelError::InvariantViolation);
                }
                if self.has_live_or_pending(*id) && has_reclaimable {
                    return Err(ModelError::InvariantViolation);
                }
                active_ranges.push((chunk.start, chunk_end));
            }
            active_ranges.sort_unstable();
            for adjacent in active_ranges.windows(2) {
                if adjacent[0].1 > adjacent[1].0 {
                    return Err(ModelError::InvariantViolation);
                }
            }
            Ok(())
        }

        fn assert_valid(&self) {
            assert_eq!(self.validate(), Ok(()), "hypothetical model invariant");
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn thread(id: u8) -> ThreadKey {
            ThreadKey(id)
        }

        #[test]
        fn remote_free_stays_live_until_publication_is_flushed() {
            let mut model = HypotheticalAllocator::new(32);
            let chunk = model.reserve_chunk(thread(1), 16).unwrap();
            let allocation = model.allocate_from_chunk(chunk, thread(1), 8).unwrap();
            model
                .transfer_live_handle(allocation, thread(1), thread(2))
                .unwrap();

            model
                .release_allocation(allocation, thread(2))
                .expect("remote drop publishes to the chunk owner's queue");
            assert_eq!(
                model.accounting(),
                Accounting {
                    live_bytes: 8,
                    reserved_bytes: 8,
                    free_bytes: 0,
                    reclaimable_bytes: 0,
                }
            );
            assert_eq!(
                model.chunk_accounting(chunk),
                Ok(Accounting {
                    live_bytes: 8,
                    reserved_bytes: 8,
                    free_bytes: 0,
                    reclaimable_bytes: 0,
                })
            );
            assert_eq!(model.teardown_chunk(chunk), Err(ModelError::ChunkBusy));
            let pending = model
                .extents
                .iter()
                .find(|extent| match extent.state {
                    ExtentState::PendingRelease { allocation: id, .. } => id == allocation,
                    _ => false,
                })
                .unwrap();
            assert_eq!(
                pending.state,
                ExtentState::PendingRelease {
                    owner: thread(1),
                    allocation,
                    route: PendingRoute::RemoteQueue,
                }
            );
            assert_eq!(
                model.flush_remote(chunk, thread(2)),
                Err(ModelError::WrongQueueOwner)
            );
            assert_eq!(model.flush_remote(chunk, thread(1)), Ok(1));
            assert_eq!(model.accounting().live_bytes, 0);
            assert_eq!(model.accounting().reserved_bytes, 16);
            assert_eq!(
                model.chunk_accounting(chunk),
                Ok(Accounting {
                    live_bytes: 0,
                    reserved_bytes: 16,
                    free_bytes: 0,
                    reclaimable_bytes: 0,
                })
            );

            let replacement = model.allocate_from_chunk(chunk, thread(1), 8).unwrap();
            assert_ne!(replacement, allocation);
            model.assert_valid();
        }

        #[test]
        fn pending_exact_reuse_gets_a_fresh_identity_and_rejects_stale_free() {
            let mut model = HypotheticalAllocator::new(32);
            let chunk = model.reserve_chunk(thread(3), 16).unwrap();
            let old = model.allocate_from_chunk(chunk, thread(3), 8).unwrap();
            let other = model.allocate_from_chunk(chunk, thread(3), 8).unwrap();
            assert_ne!(old, other);
            let old_start = model
                .extents
                .iter()
                .find(|extent| {
                    matches!(
                        extent.state,
                        ExtentState::Live { allocation, .. } if allocation == old
                    )
                })
                .unwrap()
                .start;
            let other_start = model
                .extents
                .iter()
                .find(|extent| {
                    matches!(
                        extent.state,
                        ExtentState::Live { allocation, .. } if allocation == other
                    )
                })
                .unwrap()
                .start;
            assert_ne!(
                old_start, other_start,
                "distinct IDs own disjoint byte ranges"
            );

            model.release_allocation(old, thread(3)).unwrap();
            let before = model.accounting();
            assert_eq!(
                model.release_allocation(old, thread(3)),
                Err(ModelError::UnknownOrStaleAllocation),
                "a pending release cannot be freed twice"
            );
            assert_eq!(model.accounting(), before);

            let new = model.reuse_pending_exact(old, thread(3), 8).unwrap();
            assert_ne!(new, old);
            assert_eq!(model.accounting(), before);
            let new_start = model
                .extents
                .iter()
                .find(|extent| {
                    matches!(
                        extent.state,
                        ExtentState::Live { allocation, .. } if allocation == new
                    )
                })
                .unwrap()
                .start;
            assert_eq!(new_start, old_start);
            assert_eq!(
                model.release_allocation(old, thread(3)),
                Err(ModelError::UnknownOrStaleAllocation)
            );
            assert_eq!(model.accounting(), before);
            model.assert_valid();
        }

        #[test]
        fn chunk_transfer_requires_draining_remote_publications() {
            let mut model = HypotheticalAllocator::new(32);
            let chunk = model.reserve_chunk(thread(1), 16).unwrap();
            let remotely_freed = model.allocate_from_chunk(chunk, thread(1), 4).unwrap();
            let still_live = model.allocate_from_chunk(chunk, thread(1), 4).unwrap();
            model
                .transfer_live_handle(remotely_freed, thread(1), thread(2))
                .unwrap();
            model.release_allocation(remotely_freed, thread(2)).unwrap();
            assert_eq!(
                model.transfer_chunk(chunk, thread(1), thread(2)),
                Err(ModelError::ChunkBusy)
            );
            model.flush_remote(chunk, thread(1)).unwrap();
            assert_eq!(model.accounting().live_bytes, 4);
            model.transfer_chunk(chunk, thread(1), thread(2)).unwrap();
            assert_eq!(model.accounting().live_bytes, 4);
            let transferred = model.allocate_from_chunk(chunk, thread(2), 4).unwrap();
            assert_ne!(remotely_freed, transferred);
            assert_ne!(still_live, transferred);
            assert_eq!(model.chunks[&chunk].owner, thread(2));

            // A live allocation remains valid across chunk-owner transfer. Its
            // later drop is routed to the new owner and counted until flushed.
            model.release_allocation(still_live, thread(1)).unwrap();
            assert_eq!(model.accounting().live_bytes, 8);
            model.flush_remote(chunk, thread(2)).unwrap();
            assert_eq!(model.accounting().live_bytes, 4);
            assert_eq!(model.accounting().reserved_bytes, 12);
            model.assert_valid();
        }

        #[test]
        fn exiting_owner_retires_chunk_until_remote_live_handle_is_released() {
            let mut model = HypotheticalAllocator::new(32);
            let chunk = model.reserve_chunk(thread(1), 16).unwrap();
            let allocation = model.allocate_from_chunk(chunk, thread(1), 8).unwrap();
            model
                .transfer_live_handle(allocation, thread(1), thread(2))
                .unwrap();

            model.thread_exit(thread(1)).unwrap();
            assert_eq!(model.chunks[&chunk].owner, REAPER);
            assert_eq!(model.chunks[&chunk].phase, ChunkPhase::Retiring);
            assert_eq!(model.accounting().live_bytes, 8);
            assert_eq!(model.accounting().reserved_bytes, 8);
            assert_eq!(model.chunk_accounting(chunk).unwrap().live_bytes, 8);
            assert_eq!(
                model.allocate_from_chunk(chunk, thread(3), 1),
                Err(ModelError::ChunkNotActive)
            );
            assert_eq!(
                model.teardown_chunk(chunk),
                Err(ModelError::ChunkBusy),
                "thread exit must not publish a chunk containing a live suballocation"
            );

            model.release_allocation(allocation, thread(2)).unwrap();
            model.flush_remote(chunk, REAPER).unwrap();
            model.teardown_chunk(chunk).unwrap();
            assert_eq!(model.chunks[&chunk].phase, ChunkPhase::Reclaimed);
            assert_eq!(model.accounting(), Accounting::default());
            model.assert_valid();
        }

        #[test]
        fn panic_during_teardown_leaves_chunk_owned_and_recoverable() {
            let mut model = HypotheticalAllocator::new(32);
            let chunk = model.reserve_chunk(thread(4), 16).unwrap();
            let _guard = model.reserve_chunk(thread(5), 8).unwrap();

            let interrupted = catch_unwind(AssertUnwindSafe(|| {
                let _ = model.teardown_chunk_with_hook(chunk, || panic!("injected teardown panic"));
            }));
            assert!(interrupted.is_err());
            assert_eq!(model.chunks[&chunk].phase, ChunkPhase::Reclaiming);
            assert_eq!(model.accounting().reserved_bytes, 24);
            assert_eq!(model.accounting().reclaimable_bytes, 0);
            assert_eq!(model.accounting().free_bytes, 0);
            model.assert_valid();

            model.teardown_chunk(chunk).unwrap();
            assert_eq!(model.accounting().reclaimable_bytes, 0);
            assert_eq!(model.accounting().free_bytes, 16);
            model.assert_valid();
        }

        #[test]
        fn thread_exit_reaps_chunk_left_reclaiming_after_teardown_unwinds() {
            let mut model = HypotheticalAllocator::new(24);
            let chunk = model.reserve_chunk(thread(4), 16).unwrap();

            let interrupted = catch_unwind(AssertUnwindSafe(|| {
                let _ = model.teardown_chunk_with_hook(chunk, || panic!("injected teardown panic"));
            }));
            assert!(interrupted.is_err());
            assert_eq!(model.chunks[&chunk].phase, ChunkPhase::Reclaiming);

            model.thread_exit(thread(4)).unwrap();
            assert!(model.exited_threads.contains(&thread(4)));
            assert_eq!(model.chunks[&chunk].owner, REAPER);
            assert_eq!(model.chunks[&chunk].phase, ChunkPhase::Reclaimed);
            assert_eq!(model.accounting(), Accounting::default());
            model.assert_valid();
        }

        #[test]
        fn live_or_pending_suballocation_blocks_whole_chunk_reclamation() {
            let mut model = HypotheticalAllocator::new(32);
            let chunk = model.reserve_chunk(thread(6), 16).unwrap();
            let _guard = model.reserve_chunk(thread(7), 8).unwrap();
            let allocation = model.allocate_from_chunk(chunk, thread(6), 8).unwrap();

            assert_eq!(model.teardown_chunk(chunk), Err(ModelError::ChunkBusy));
            assert_eq!(model.accounting().reclaimable_bytes, 0);
            model.release_allocation(allocation, thread(6)).unwrap();
            assert_eq!(model.teardown_chunk(chunk), Err(ModelError::ChunkBusy));
            assert_eq!(
                model.accounting().live_bytes,
                8,
                "pending still counts live"
            );
            assert_eq!(model.accounting().reclaimable_bytes, 0);

            model.flush_local(thread(6)).unwrap();
            model.begin_teardown(chunk).unwrap();
            model.mark_chunk_reclaimable(chunk).unwrap();
            assert_eq!(model.accounting().reserved_bytes, 8);
            assert_eq!(model.accounting().reclaimable_bytes, 16);
            assert_eq!(model.accounting().free_bytes, 0);
            model.publish_reclaimable_chunk(chunk).unwrap();
            assert_eq!(model.accounting().reclaimable_bytes, 0);
            assert_eq!(model.accounting().free_bytes, 16);
            model.assert_valid();
        }

        #[test]
        fn exhausted_arena_recovers_by_reclaiming_and_reserving_again() {
            let mut model = HypotheticalAllocator::new(24);
            let first = model.reserve_chunk(thread(1), 12).unwrap();
            let second = model.reserve_chunk(thread(2), 12).unwrap();
            let first_live = model.allocate_from_chunk(first, thread(1), 12).unwrap();
            let _second_live = model.allocate_from_chunk(second, thread(2), 12).unwrap();
            let before = model.accounting();
            assert_eq!(
                model.reserve_chunk(thread(3), 1),
                Err(ModelError::AllocationExhausted)
            );
            assert_eq!(model.accounting(), before);

            model.release_allocation(first_live, thread(1)).unwrap();
            model.flush_local(thread(1)).unwrap();
            model.teardown_chunk(first).unwrap();
            let replenished = model.reserve_chunk(thread(3), 12).unwrap();
            assert_eq!(model.chunks[&replenished].start, 0);
            assert_eq!(model.accounting().free_bytes, 0);
            assert_eq!(model.accounting().live_bytes, 12);
            assert_eq!(model.accounting().reserved_bytes, 12);
            model.assert_valid();
        }

        #[test]
        fn adjacent_reclaimed_reservations_coalesce_for_larger_replenishment() {
            let mut model = HypotheticalAllocator::new(64);
            let first = model.reserve_chunk(thread(1), 8).unwrap();
            let second = model.reserve_chunk(thread(2), 8).unwrap();
            let _guard = model.reserve_chunk(thread(3), 8).unwrap();
            model.teardown_chunk(first).unwrap();
            assert_eq!(model.accounting().free_bytes, 8);
            model.teardown_chunk(second).unwrap();
            assert_eq!(model.accounting().free_bytes, 16);
            assert_eq!(
                model
                    .extents
                    .iter()
                    .filter(|extent| extent.state == ExtentState::GlobalFree)
                    .count(),
                1,
                "adjacent free bytes from separate reservation IDs coalesce"
            );

            let larger = model.reserve_chunk(thread(4), 12).unwrap();
            assert_eq!(model.chunks[&larger].start, 0);
            assert_eq!(model.accounting().reserved_bytes, 20);
            assert_eq!(model.accounting().free_bytes, 4);
            assert_eq!(model.accounting().total(), model.cursor);
            model.assert_valid();
        }
    }
}
