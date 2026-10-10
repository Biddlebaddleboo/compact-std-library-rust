//! Process-wide cage and four-byte allocation owners.

use crate::deterministic_memory::{
    LocalCacheState, RecycledExtent, ReleaseExtent, LOCAL_CACHE_CAPACITY,
    MAX_ACTIVE_LOCAL_CACHE_OWNERS,
};
use compact_core::{
    checked_align_up, CompactValue, Error, Offset32, Result, MAX_CAGE_BYTES, MIN_CAGE_BYTES,
};
use core::marker::PhantomData;
use core::mem::{align_of, size_of, MaybeUninit};
use core::ptr::NonNull;
use core::slice;
use std::alloc::{alloc, dealloc, Layout};
use std::cell::{Cell, RefCell};
use std::ops::{Deref, DerefMut};
#[cfg(feature = "allocator-telemetry")]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};
#[cfg(feature = "allocator-telemetry")]
use std::time::Instant;

const INITIAL_CURSOR: u32 = 8;
const FREE_NODE_BYTES: u32 = size_of::<FreeNode>() as u32;
const BLOCK_SIZE_BUCKETS: usize = 129;
const RELEASE_BATCH_CAPACITY: usize = 64;
const SIZE_CLASSES: [u32; 4] = [32, 40, 112, 528];
const SIZE_CLASS_COUNT: usize = SIZE_CLASSES.len();
const SIZE_CLASS_CACHE_CAPACITY: u32 = 32;
const MAX_SIZE_CLASS_EXTENTS: usize = SIZE_CLASSES.len() * SIZE_CLASS_CACHE_CAPACITY as usize;
const MAX_MERGE_EXTENTS: usize = RELEASE_BATCH_CAPACITY + MAX_SIZE_CLASS_EXTENTS;
const MAX_PENDING_RELEASE_DRAIN: usize = RELEASE_BATCH_CAPACITY;
const LOCAL_CACHE_BYTE_BUDGET: usize = 4 * 1024;
const BENCHMARK_POLICY_A: bool = cfg!(feature = "benchmark-allocator-a")
    && !cfg!(feature = "benchmark-allocator-b")
    && !cfg!(feature = "benchmark-allocator-c");
const BENCHMARK_POLICY_C: bool = cfg!(feature = "benchmark-allocator-c")
    || (cfg!(feature = "benchmark-allocator-a") && cfg!(feature = "benchmark-allocator-b"));
const ENABLE_PENDING_REUSE: bool = !BENCHMARK_POLICY_A;
const ENABLE_SIZE_CLASS_CACHE: bool = BENCHMARK_POLICY_A || BENCHMARK_POLICY_C;

#[repr(C)]
#[derive(Clone, Copy)]
struct AllocationHeader {
    block_len: u32,
    prefix: u32,
    capacity: u32,
    initialized: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FreeNode {
    next: u32,
    len: u32,
}

struct ReleaseCollector {
    extents: [ReleaseExtent; RELEASE_BATCH_CAPACITY],
    len: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingLookup {
    Disabled,
    NoCollector,
    NoExactBlock { alignment_incompatible: bool },
    Recycled,
}

#[cfg(feature = "allocator-telemetry")]
struct PendingReuseTelemetry {
    hits: AtomicU64,
    misses: AtomicU64,
    no_active_collector: AtomicU64,
    no_exact_block: AtomicU64,
    alignment_incompatible: AtomicU64,
    scan_candidates: AtomicU64,
    scan_depth_histogram: [AtomicU64; RELEASE_BATCH_CAPACITY + 1],
    candidate_size_histogram: [AtomicU64; BLOCK_SIZE_BUCKETS],
}

#[cfg(feature = "allocator-telemetry")]
static PENDING_REUSE_TELEMETRY: PendingReuseTelemetry = PendingReuseTelemetry {
    hits: AtomicU64::new(0),
    misses: AtomicU64::new(0),
    no_active_collector: AtomicU64::new(0),
    no_exact_block: AtomicU64::new(0),
    alignment_incompatible: AtomicU64::new(0),
    scan_candidates: AtomicU64::new(0),
    scan_depth_histogram: [const { AtomicU64::new(0) }; RELEASE_BATCH_CAPACITY + 1],
    candidate_size_histogram: [const { AtomicU64::new(0) }; BLOCK_SIZE_BUCKETS],
};

#[cfg(feature = "allocator-telemetry")]
struct AllocatorPhaseTelemetry {
    pending_lookup_ns: AtomicU64,
    layout_ns: AtomicU64,
    lock_wait_ns: AtomicU64,
    free_list_search_ns: AtomicU64,
    bump_allocation_ns: AtomicU64,
    header_initialization_ns: AtomicU64,
}

#[cfg(feature = "allocator-telemetry")]
static ALLOCATOR_PHASE_TELEMETRY: AllocatorPhaseTelemetry = AllocatorPhaseTelemetry {
    pending_lookup_ns: AtomicU64::new(0),
    layout_ns: AtomicU64::new(0),
    lock_wait_ns: AtomicU64::new(0),
    free_list_search_ns: AtomicU64::new(0),
    bump_allocation_ns: AtomicU64::new(0),
    header_initialization_ns: AtomicU64::new(0),
};

#[cfg(feature = "allocator-telemetry")]
struct PhaseTimer {
    started: Instant,
    total_ns: &'static AtomicU64,
}

#[cfg(feature = "allocator-telemetry")]
impl PhaseTimer {
    fn start(total_ns: &'static AtomicU64) -> Self {
        Self {
            started: Instant::now(),
            total_ns,
        }
    }
}

#[cfg(feature = "allocator-telemetry")]
impl Drop for PhaseTimer {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        self.total_ns.fetch_add(elapsed, Ordering::Relaxed);
    }
}

#[cfg(feature = "allocator-telemetry")]
static PENDING_ALLOCATION_SIZE_HISTOGRAM: [AtomicU64; BLOCK_SIZE_BUCKETS] =
    [const { AtomicU64::new(0) }; BLOCK_SIZE_BUCKETS];

impl ReleaseCollector {
    fn new() -> Self {
        Self {
            extents: [ReleaseExtent::default(); RELEASE_BATCH_CAPACITY],
            len: 0,
        }
    }

    fn push(&mut self, extent: ReleaseExtent) {
        if self.len == RELEASE_BATCH_CAPACITY {
            self.flush();
        }
        if self.len < RELEASE_BATCH_CAPACITY {
            self.extents[self.len] = extent;
            self.len += 1;
        } else {
            // A failed batch must not erase the descriptor already in this
            // bounded collector or lose the new release. The shared release
            // path has an in-cage pending-list fallback for this case.
            let mut one = [extent];
            let _ = release_many(&mut one);
        }
    }

    fn take_compatible(
        &mut self,
        base: *mut u8,
        bytes: usize,
        alignment: usize,
    ) -> (PendingLookup, Option<RecycledExtent>) {
        let minimum_len = bytes
            .checked_add(size_of::<AllocationHeader>())
            .and_then(|raw| raw.checked_add(7))
            .and_then(|raw| u32::try_from(raw & !7).ok());
        let mut alignment_incompatible = false;
        #[cfg(feature = "allocator-telemetry")]
        let mut scanned = 0;
        for index in (0..self.len).rev() {
            let extent = self.extents[index];
            #[cfg(feature = "allocator-telemetry")]
            {
                scanned += 1;
                record_pending_candidate_size(extent.len);
            }
            let Ok((data_offset, prefix, block_len)) =
                block_layout(base, extent.start, bytes, alignment)
            else {
                continue;
            };
            if block_len == extent.len {
                let Some(data_offset) = core::num::NonZeroU32::new(data_offset) else {
                    continue;
                };
                // Preserve insertion order so the last remaining entry is
                // still the most recently released candidate.
                self.extents.copy_within(index + 1..self.len, index);
                self.len -= 1;
                self.extents[self.len] = ReleaseExtent::default();
                #[cfg(feature = "allocator-telemetry")]
                record_pending_scan_depth(scanned);
                return (
                    PendingLookup::Recycled,
                    Some(RecycledExtent {
                        data_offset,
                        prefix,
                        block_len,
                    }),
                );
            }
            if minimum_len == Some(extent.len) && block_len > extent.len {
                alignment_incompatible = true;
            }
        }
        #[cfg(feature = "allocator-telemetry")]
        record_pending_scan_depth(scanned);
        (
            PendingLookup::NoExactBlock {
                alignment_incompatible,
            },
            None,
        )
    }

    fn flush(&mut self) {
        self.flush_with(release_many);
    }

    fn flush_with(&mut self, mut release: impl FnMut(&mut [ReleaseExtent]) -> Result<()>) {
        if self.len == 0 {
            return;
        }
        if release(&mut self.extents[..self.len]).is_ok() {
            self.clear();
            return;
        }

        // A bad member must not prevent the remaining valid descriptors from
        // being returned during destructor unwinding. Retain every descriptor
        // whose individual retry also fails so the caller can preserve it.
        let original_len = self.len;
        let mut failed_len = 0;
        for index in 0..original_len {
            let extent = self.extents[index];
            let mut one = [extent];
            if release(&mut one).is_err() {
                self.extents[failed_len] = extent;
                failed_len += 1;
            }
        }
        self.extents[failed_len..original_len].fill(ReleaseExtent::default());
        self.len = failed_len;
    }

    fn clear(&mut self) {
        self.extents[..self.len].fill(ReleaseExtent::default());
        self.len = 0;
    }
}

thread_local! {
    static ACTIVE_RELEASE_COLLECTOR: Cell<*mut ReleaseCollector> = const {
        Cell::new(core::ptr::null_mut())
    };
    static LOCAL_REUSE_CACHE: LocalReuseCacheSlot = const { LocalReuseCacheSlot::new() };
}

struct LocalReuseCacheSlot {
    cache: RefCell<LocalCacheState>,
    registered: Cell<bool>,
}

impl LocalReuseCacheSlot {
    const fn new() -> Self {
        Self {
            cache: RefCell::new(LocalCacheState::new()),
            registered: Cell::new(false),
        }
    }
}

impl Drop for LocalReuseCacheSlot {
    fn drop(&mut self) {
        if !self.registered.replace(false) {
            return;
        }
        if let Ok(state) = state() {
            flush_local_cache_state(state, self.cache.get_mut());
            state
                .active_local_cache_owners
                .fetch_sub(1, Ordering::AcqRel);
        }
    }
}

// Ordinary allocations can avoid touching TLS when no thread is batching
// releases. A thread's collector increments this before its operation runs
// and decrements only after clearing its TLS slot, so its own allocation
// cannot observe zero while its collector is active.
static ACTIVE_RELEASE_COLLECTOR_COUNT: AtomicUsize = AtomicUsize::new(0);

struct ReleaseBatchScope {
    collector: *mut ReleaseCollector,
    previous: *mut ReleaseCollector,
}

impl Drop for ReleaseBatchScope {
    fn drop(&mut self) {
        ACTIVE_RELEASE_COLLECTOR.with(|active| active.set(self.previous));
        if ENABLE_PENDING_REUSE {
            ACTIVE_RELEASE_COLLECTOR_COUNT.fetch_sub(1, Ordering::Relaxed);
        }
        // SAFETY: the collector is stack-local to `with_batched_releases` and
        // this scope guard is dropped before that local leaves scope.
        unsafe { (*self.collector).flush() };
    }
}

const _: [(); 16] = [(); size_of::<AllocationHeader>()];
const _: [(); 8] = [(); size_of::<FreeNode>()];

struct Allocator {
    cursor: u32,
    live_bytes: u32,
    free_head: u32,
    size_class_heads: [u32; SIZE_CLASSES.len()],
    size_class_counts: [u32; SIZE_CLASSES.len()],
    has_size_class_cache: bool,
    #[cfg(feature = "allocator-telemetry")]
    lock_acquisitions: u64,
    #[cfg(feature = "allocator-telemetry")]
    free_list_nodes_visited: u64,
    #[cfg(feature = "allocator-telemetry")]
    allocation_size_histogram: [u64; BLOCK_SIZE_BUCKETS],
    #[cfg(feature = "allocator-telemetry")]
    release_batches: u64,
    #[cfg(feature = "allocator-telemetry")]
    released_extents: u64,
    #[cfg(feature = "allocator-telemetry")]
    max_release_batch: u32,
    #[cfg(feature = "allocator-telemetry")]
    size_class_hits: u64,
    #[cfg(feature = "allocator-telemetry")]
    size_class_misses: u64,
    #[cfg(feature = "allocator-telemetry")]
    global_class_hits: [u64; SIZE_CLASS_COUNT],
    #[cfg(feature = "allocator-telemetry")]
    global_class_misses: [u64; SIZE_CLASS_COUNT],
    #[cfg(feature = "allocator-telemetry")]
    global_class_empty: [u64; SIZE_CLASS_COUNT],
    #[cfg(feature = "allocator-telemetry")]
    global_class_alignment_incompatible: [u64; SIZE_CLASS_COUNT],
    #[cfg(feature = "allocator-telemetry")]
    requested_size_no_class: u64,
    #[cfg(feature = "allocator-telemetry")]
    general_list_fallbacks: u64,
    #[cfg(feature = "allocator-telemetry")]
    cursor_fallbacks: u64,
    #[cfg(feature = "allocator-telemetry")]
    released_exact_size_extents: u64,
    #[cfg(feature = "allocator-telemetry")]
    exact_size_extents_cached: u64,
    #[cfg(feature = "allocator-telemetry")]
    exact_size_extents_coalesced_before_cache: u64,
}

struct CageState {
    capacity: usize,
    memory: NonNull<u8>,
    allocator: Mutex<Allocator>,
    local_reuse_activated: AtomicBool,
    active_local_cache_owners: AtomicUsize,
    local_cache_bytes: AtomicUsize,
    local_cache_budget: usize,
    pending_releases: Mutex<PendingReleaseQueue>,
    pending_release_nonempty: AtomicBool,
    allocator_faulted: AtomicBool,
}

/// Intrusive in-cage releases waiting for a bounded foreground drain.
///
/// The queue mutex prevents an offset head from suffering ABA if a released
/// extent is drained, reused, and later released at the same address while a
/// producer is trying to publish another node.
#[derive(Default)]
struct PendingReleaseQueue {
    head: u32,
}

/// Holds the global allocator lock for one internal operation. Callers must
/// finish all user code and destructors before opening a transaction.
struct AllocatorTransaction<'a> {
    state: &'a CageState,
    allocator: MutexGuard<'a, Allocator>,
}

impl Deref for AllocatorTransaction<'_> {
    type Target = Allocator;

    fn deref(&self) -> &Self::Target {
        &self.allocator
    }
}

impl DerefMut for AllocatorTransaction<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.allocator
    }
}

impl AllocatorTransaction<'_> {
    fn release_many(&mut self, extents: &mut [ReleaseExtent]) -> Result<()> {
        release_many_locked(
            self.state,
            &mut self.allocator,
            extents,
            ENABLE_SIZE_CLASS_CACHE,
        )
    }
}

// SAFETY: `memory` uniquely owns a stable raw allocation. Allocator scalars and
// free-list writes are serialized by `allocator`; each live range's data and
// header are mutated through its exclusive owner or during an allocator
// critical section. Shared views follow `CompactValue`'s safety contract and
// normal Rust borrowing. `CageAllocation<T>` is only Send/Sync when `T` has
// the corresponding native thread-safety traits.
unsafe impl Send for CageState {}
unsafe impl Sync for CageState {}

impl CageState {
    fn base(&self) -> *mut u8 {
        self.memory.as_ptr()
    }
}

impl Drop for CageState {
    fn drop(&mut self) {
        let layout =
            Layout::from_size_align(self.capacity, 8).expect("valid cage allocation layout");
        // SAFETY: `memory` was allocated with this exact layout in `init`.
        unsafe { dealloc(self.memory.as_ptr(), layout) };
    }
}

static CAGE: OnceLock<CageState> = OnceLock::new();

/// Configuration for the one process-wide compact cage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CageConfig {
    /// Requested cage capacity in bytes.
    pub capacity: usize,
}

/// Read-only snapshot of the process cage allocator.
///
/// This diagnostic surface is hidden from the normal API documentation and
/// does not alter allocator state or retained owner layouts.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocatorStats {
    /// Bytes occupied by live allocations and authoritative pending releases.
    pub live_bytes: u32,
    /// Current high-water cursor measured from the start of the cage.
    pub high_water_cursor: u32,
    /// Total bytes in reusable free blocks.
    pub free_bytes: u32,
    /// Number of reusable free blocks.
    pub free_blocks: u32,
    /// Size of the largest reusable free block.
    pub largest_free_block: u32,
    /// Number of allocator mutex acquisitions since runtime initialization.
    pub lock_acquisitions: u64,
    /// Number of ordered free-list nodes visited by allocation/release paths.
    pub free_list_nodes_visited: u64,
    /// Number of release batches applied.
    pub release_batches: u64,
    /// Number of extents processed by release batches.
    pub released_extents: u64,
    /// Largest release batch applied.
    pub max_release_batch: u32,
    /// Number of small size-class allocations served from a cached block.
    pub size_class_hits: u64,
    /// Number of eligible small allocations that missed size-class caches.
    pub size_class_misses: u64,
    /// Pending-release exact-reuse hits before acquiring the allocator lock.
    pub pending_reuse_hits: u64,
    /// Allocations that could not reuse an extent in the active pending batch.
    pub pending_reuse_misses: u64,
    /// Pending-reuse misses that had no active thread-local collector.
    pub pending_reuse_no_active_collector: u64,
    /// Pending-reuse misses with no exact compatible block length.
    pub pending_reuse_no_exact_block: u64,
    /// Pending-reuse misses where alignment padding made an exact-size block too small.
    pub pending_reuse_alignment_incompatible: u64,
    #[cfg(feature = "allocator-telemetry")]
    /// Candidate extents examined by pending exact-size lookups.
    pub pending_reuse_scan_candidates: u64,
    #[cfg(feature = "allocator-telemetry")]
    /// Pending lookup depths, indexed from 0 through the collector capacity.
    pub pending_reuse_scan_depth_histogram: [u64; RELEASE_BATCH_CAPACITY + 1],
    #[cfg(feature = "allocator-telemetry")]
    /// Examined pending extent sizes in 8-byte buckets; the last bucket is 1024 B+.
    pub pending_reuse_candidate_size_histogram: [u64; BLOCK_SIZE_BUCKETS],
    #[cfg(feature = "allocator-telemetry")]
    /// Cumulative time spent checking the active release collector, in nanoseconds.
    pub pending_lookup_phase_ns: u64,
    #[cfg(feature = "allocator-telemetry")]
    /// Cumulative time spent computing allocation layouts, in nanoseconds.
    pub layout_phase_ns: u64,
    #[cfg(feature = "allocator-telemetry")]
    /// Cumulative time waiting to acquire the allocator mutex, in nanoseconds.
    pub lock_wait_phase_ns: u64,
    #[cfg(feature = "allocator-telemetry")]
    /// Cumulative time searching global reusable free ranges, in nanoseconds.
    pub free_list_search_phase_ns: u64,
    #[cfg(feature = "allocator-telemetry")]
    /// Cumulative time reserving ranges at the bump cursor, in nanoseconds.
    pub bump_allocation_phase_ns: u64,
    #[cfg(feature = "allocator-telemetry")]
    /// Cumulative time writing fresh allocation headers, in nanoseconds.
    pub header_initialization_phase_ns: u64,
    /// Successful global-cache allocations by exact block size.
    pub global_class_hits: [u64; SIZE_CLASS_COUNT],
    /// Global-cache lookups not served by each class, by exact block size.
    pub global_class_misses: [u64; SIZE_CLASS_COUNT],
    /// Global-cache lookups where each relevant class had no cached blocks.
    pub global_class_empty: [u64; SIZE_CLASS_COUNT],
    /// Global-class candidate blocks rejected because alignment padding would not fit.
    pub global_class_alignment_incompatible: [u64; SIZE_CLASS_COUNT],
    /// Requests whose natural block length is not one of the configured classes.
    pub requested_size_no_class: u64,
    /// Allocations served by the general free-list fallback.
    pub general_list_fallbacks: u64,
    /// Allocations served by the bump cursor after reusable ranges missed.
    pub cursor_fallbacks: u64,
    /// Released extents whose individual block length matched a configured class.
    pub released_exact_size_extents: u64,
    /// Exact-size free runs inserted into a global class cache.
    pub exact_size_extents_cached: u64,
    /// Exact-size released extents joined a larger run before cache insertion.
    pub exact_size_extents_coalesced_before_cache: u64,
    /// Allocation counts by 8-byte block-size bucket; bucket 128 is 1024 B+.
    pub allocation_size_histogram: [u64; BLOCK_SIZE_BUCKETS],
    /// Cached free block counts for the measured exact-size classes.
    pub size_class_free_blocks: [u32; SIZE_CLASSES.len()],
    /// Cached free bytes for the measured exact-size classes.
    pub size_class_free_bytes: [u32; SIZE_CLASSES.len()],
}

impl Default for AllocatorStats {
    fn default() -> Self {
        Self {
            live_bytes: 0,
            high_water_cursor: 0,
            free_bytes: 0,
            free_blocks: 0,
            largest_free_block: 0,
            lock_acquisitions: 0,
            free_list_nodes_visited: 0,
            release_batches: 0,
            released_extents: 0,
            max_release_batch: 0,
            size_class_hits: 0,
            size_class_misses: 0,
            pending_reuse_hits: 0,
            pending_reuse_misses: 0,
            pending_reuse_no_active_collector: 0,
            pending_reuse_no_exact_block: 0,
            pending_reuse_alignment_incompatible: 0,
            #[cfg(feature = "allocator-telemetry")]
            pending_reuse_scan_candidates: 0,
            #[cfg(feature = "allocator-telemetry")]
            pending_reuse_scan_depth_histogram: [0; RELEASE_BATCH_CAPACITY + 1],
            #[cfg(feature = "allocator-telemetry")]
            pending_reuse_candidate_size_histogram: [0; BLOCK_SIZE_BUCKETS],
            #[cfg(feature = "allocator-telemetry")]
            pending_lookup_phase_ns: 0,
            #[cfg(feature = "allocator-telemetry")]
            layout_phase_ns: 0,
            #[cfg(feature = "allocator-telemetry")]
            lock_wait_phase_ns: 0,
            #[cfg(feature = "allocator-telemetry")]
            free_list_search_phase_ns: 0,
            #[cfg(feature = "allocator-telemetry")]
            bump_allocation_phase_ns: 0,
            #[cfg(feature = "allocator-telemetry")]
            header_initialization_phase_ns: 0,
            global_class_hits: [0; SIZE_CLASS_COUNT],
            global_class_misses: [0; SIZE_CLASS_COUNT],
            global_class_empty: [0; SIZE_CLASS_COUNT],
            global_class_alignment_incompatible: [0; SIZE_CLASS_COUNT],
            requested_size_no_class: 0,
            general_list_fallbacks: 0,
            cursor_fallbacks: 0,
            released_exact_size_extents: 0,
            exact_size_extents_cached: 0,
            exact_size_extents_coalesced_before_cache: 0,
            allocation_size_histogram: [0; BLOCK_SIZE_BUCKETS],
            size_class_free_blocks: [0; SIZE_CLASSES.len()],
            size_class_free_bytes: [0; SIZE_CLASSES.len()],
        }
    }
}

impl CageConfig {
    /// Create a configuration with the requested capacity.
    pub const fn new(capacity: usize) -> Self {
        Self { capacity }
    }
}

/// Process-wide runtime access and initialization.
pub struct CompactRuntime;

impl CompactRuntime {
    /// Initialize the process cage exactly once.
    pub fn init(config: CageConfig) -> Result<()> {
        if CAGE.get().is_some() {
            return Err(Error::RuntimeAlreadyInitialized);
        }
        if config.capacity < MIN_CAGE_BYTES {
            return Err(Error::InvalidCapacity);
        }
        if config.capacity as u64 > MAX_CAGE_BYTES || config.capacity > u32::MAX as usize {
            return Err(Error::CageTooLarge);
        }
        let layout =
            Layout::from_size_align(config.capacity, 8).map_err(|_| Error::InvalidCapacity)?;
        // SAFETY: `layout` is nonzero and valid after the checks above.
        let memory = NonNull::new(unsafe { alloc(layout) }).ok_or(Error::AllocationFailed)?;
        let state = CageState {
            capacity: config.capacity,
            memory,
            allocator: Mutex::new(Allocator {
                cursor: INITIAL_CURSOR,
                live_bytes: 0,
                free_head: 0,
                size_class_heads: [0; SIZE_CLASSES.len()],
                size_class_counts: [0; SIZE_CLASSES.len()],
                has_size_class_cache: false,
                #[cfg(feature = "allocator-telemetry")]
                lock_acquisitions: 0,
                #[cfg(feature = "allocator-telemetry")]
                free_list_nodes_visited: 0,
                #[cfg(feature = "allocator-telemetry")]
                allocation_size_histogram: [0; BLOCK_SIZE_BUCKETS],
                #[cfg(feature = "allocator-telemetry")]
                release_batches: 0,
                #[cfg(feature = "allocator-telemetry")]
                released_extents: 0,
                #[cfg(feature = "allocator-telemetry")]
                max_release_batch: 0,
                #[cfg(feature = "allocator-telemetry")]
                size_class_hits: 0,
                #[cfg(feature = "allocator-telemetry")]
                size_class_misses: 0,
                #[cfg(feature = "allocator-telemetry")]
                global_class_hits: [0; SIZE_CLASS_COUNT],
                #[cfg(feature = "allocator-telemetry")]
                global_class_misses: [0; SIZE_CLASS_COUNT],
                #[cfg(feature = "allocator-telemetry")]
                global_class_empty: [0; SIZE_CLASS_COUNT],
                #[cfg(feature = "allocator-telemetry")]
                global_class_alignment_incompatible: [0; SIZE_CLASS_COUNT],
                #[cfg(feature = "allocator-telemetry")]
                requested_size_no_class: 0,
                #[cfg(feature = "allocator-telemetry")]
                general_list_fallbacks: 0,
                #[cfg(feature = "allocator-telemetry")]
                cursor_fallbacks: 0,
                #[cfg(feature = "allocator-telemetry")]
                released_exact_size_extents: 0,
                #[cfg(feature = "allocator-telemetry")]
                exact_size_extents_cached: 0,
                #[cfg(feature = "allocator-telemetry")]
                exact_size_extents_coalesced_before_cache: 0,
            }),
            local_reuse_activated: AtomicBool::new(false),
            active_local_cache_owners: AtomicUsize::new(0),
            local_cache_bytes: AtomicUsize::new(0),
            local_cache_budget: (config.capacity / 50).min(LOCAL_CACHE_BYTE_BUDGET),
            pending_releases: Mutex::new(PendingReleaseQueue::default()),
            pending_release_nonempty: AtomicBool::new(false),
            allocator_faulted: AtomicBool::new(false),
        };
        CAGE.set(state)
            .map_err(|_| Error::RuntimeAlreadyInitialized)
    }

    /// Return whether the process cage has been initialized.
    pub fn is_initialized() -> bool {
        CAGE.get().is_some()
    }

    /// Return the configured cage capacity.
    pub fn capacity() -> Result<usize> {
        Ok(state()?.capacity)
    }

    /// Return live plus pending bytes, including allocation headers and padding.
    pub fn used_bytes() -> Result<usize> {
        let state = state()?;
        flush_current_local_cache(state);
        Ok(lock(state)?.live_bytes as usize)
    }

    /// Return capacity not currently occupied by live allocation blocks.
    pub fn remaining_bytes() -> Result<usize> {
        let state = state()?;
        flush_current_local_cache(state);
        Ok(state
            .capacity
            .saturating_sub(lock(state)?.live_bytes as usize))
    }

    /// Snapshot live and globally reusable bytes after flushing this thread's
    /// bounded cache and draining one pending-release batch.
    #[doc(hidden)]
    pub fn allocator_stats() -> Result<AllocatorStats> {
        let state = state()?;
        flush_current_local_cache(state);
        let allocator = lock(state)?;
        let mut stats = AllocatorStats {
            live_bytes: allocator.live_bytes,
            high_water_cursor: allocator.cursor,
            size_class_free_blocks: allocator.size_class_counts,
            #[cfg(feature = "allocator-telemetry")]
            lock_acquisitions: allocator.lock_acquisitions,
            #[cfg(feature = "allocator-telemetry")]
            free_list_nodes_visited: allocator.free_list_nodes_visited,
            #[cfg(feature = "allocator-telemetry")]
            release_batches: allocator.release_batches,
            #[cfg(feature = "allocator-telemetry")]
            released_extents: allocator.released_extents,
            #[cfg(feature = "allocator-telemetry")]
            max_release_batch: allocator.max_release_batch,
            #[cfg(feature = "allocator-telemetry")]
            size_class_hits: allocator.size_class_hits,
            #[cfg(feature = "allocator-telemetry")]
            size_class_misses: allocator.size_class_misses,
            #[cfg(feature = "allocator-telemetry")]
            pending_reuse_hits: PENDING_REUSE_TELEMETRY.hits.load(Ordering::Relaxed),
            #[cfg(feature = "allocator-telemetry")]
            pending_reuse_misses: PENDING_REUSE_TELEMETRY.misses.load(Ordering::Relaxed),
            #[cfg(feature = "allocator-telemetry")]
            pending_reuse_no_active_collector: PENDING_REUSE_TELEMETRY
                .no_active_collector
                .load(Ordering::Relaxed),
            #[cfg(feature = "allocator-telemetry")]
            pending_reuse_no_exact_block: PENDING_REUSE_TELEMETRY
                .no_exact_block
                .load(Ordering::Relaxed),
            #[cfg(feature = "allocator-telemetry")]
            pending_reuse_alignment_incompatible: PENDING_REUSE_TELEMETRY
                .alignment_incompatible
                .load(Ordering::Relaxed),
            #[cfg(feature = "allocator-telemetry")]
            pending_reuse_scan_candidates: PENDING_REUSE_TELEMETRY
                .scan_candidates
                .load(Ordering::Relaxed),
            #[cfg(feature = "allocator-telemetry")]
            pending_reuse_scan_depth_histogram: core::array::from_fn(|index| {
                PENDING_REUSE_TELEMETRY.scan_depth_histogram[index].load(Ordering::Relaxed)
            }),
            #[cfg(feature = "allocator-telemetry")]
            pending_reuse_candidate_size_histogram: core::array::from_fn(|index| {
                PENDING_REUSE_TELEMETRY.candidate_size_histogram[index].load(Ordering::Relaxed)
            }),
            #[cfg(feature = "allocator-telemetry")]
            pending_lookup_phase_ns: ALLOCATOR_PHASE_TELEMETRY
                .pending_lookup_ns
                .load(Ordering::Relaxed),
            #[cfg(feature = "allocator-telemetry")]
            layout_phase_ns: ALLOCATOR_PHASE_TELEMETRY.layout_ns.load(Ordering::Relaxed),
            #[cfg(feature = "allocator-telemetry")]
            lock_wait_phase_ns: ALLOCATOR_PHASE_TELEMETRY
                .lock_wait_ns
                .load(Ordering::Relaxed),
            #[cfg(feature = "allocator-telemetry")]
            free_list_search_phase_ns: ALLOCATOR_PHASE_TELEMETRY
                .free_list_search_ns
                .load(Ordering::Relaxed),
            #[cfg(feature = "allocator-telemetry")]
            bump_allocation_phase_ns: ALLOCATOR_PHASE_TELEMETRY
                .bump_allocation_ns
                .load(Ordering::Relaxed),
            #[cfg(feature = "allocator-telemetry")]
            header_initialization_phase_ns: ALLOCATOR_PHASE_TELEMETRY
                .header_initialization_ns
                .load(Ordering::Relaxed),
            #[cfg(feature = "allocator-telemetry")]
            global_class_hits: allocator.global_class_hits,
            #[cfg(feature = "allocator-telemetry")]
            global_class_misses: allocator.global_class_misses,
            #[cfg(feature = "allocator-telemetry")]
            global_class_empty: allocator.global_class_empty,
            #[cfg(feature = "allocator-telemetry")]
            global_class_alignment_incompatible: allocator.global_class_alignment_incompatible,
            #[cfg(feature = "allocator-telemetry")]
            requested_size_no_class: allocator.requested_size_no_class,
            #[cfg(feature = "allocator-telemetry")]
            general_list_fallbacks: allocator.general_list_fallbacks,
            #[cfg(feature = "allocator-telemetry")]
            cursor_fallbacks: allocator.cursor_fallbacks,
            #[cfg(feature = "allocator-telemetry")]
            released_exact_size_extents: allocator.released_exact_size_extents,
            #[cfg(feature = "allocator-telemetry")]
            exact_size_extents_cached: allocator.exact_size_extents_cached,
            #[cfg(feature = "allocator-telemetry")]
            exact_size_extents_coalesced_before_cache: allocator
                .exact_size_extents_coalesced_before_cache,
            #[cfg(feature = "allocator-telemetry")]
            allocation_size_histogram: core::array::from_fn(|index| {
                allocator.allocation_size_histogram[index].saturating_add(
                    PENDING_ALLOCATION_SIZE_HISTOGRAM[index].load(Ordering::Relaxed),
                )
            }),
            ..AllocatorStats::default()
        };
        let mut current = allocator.free_head;
        while current != 0 {
            // SAFETY: the free list is protected by the allocator lock and each
            // link is maintained as an in-cage range by allocator operations.
            let node = unsafe { read_free_node(state, current)? };
            stats.free_bytes = stats
                .free_bytes
                .checked_add(node.len)
                .ok_or(Error::InvalidOffset)?;
            stats.free_blocks = stats
                .free_blocks
                .checked_add(1)
                .ok_or(Error::InvalidOffset)?;
            stats.largest_free_block = stats.largest_free_block.max(node.len);
            current = node.next;
        }
        for class_index in 0..SIZE_CLASSES.len() {
            let mut class_current = allocator.size_class_heads[class_index];
            let mut counted = 0_u32;
            while class_current != 0 {
                // SAFETY: class heads contain only allocator-owned free blocks.
                let node = unsafe { read_free_node(state, class_current)? };
                stats.free_bytes = stats
                    .free_bytes
                    .checked_add(node.len)
                    .ok_or(Error::InvalidOffset)?;
                stats.free_blocks = stats
                    .free_blocks
                    .checked_add(1)
                    .ok_or(Error::InvalidOffset)?;
                stats.largest_free_block = stats.largest_free_block.max(node.len);
                stats.size_class_free_bytes[class_index] = stats.size_class_free_bytes[class_index]
                    .checked_add(node.len)
                    .ok_or(Error::InvalidOffset)?;
                counted = counted.checked_add(1).ok_or(Error::InvalidOffset)?;
                class_current = node.next;
            }
            if counted != allocator.size_class_counts[class_index] {
                return Err(Error::InvalidOffset);
            }
        }
        if allocator.has_size_class_cache
            != allocator.size_class_counts.iter().any(|count| *count != 0)
        {
            return Err(Error::InvalidOffset);
        }
        Ok(stats)
    }

    /// Allocate an uninitialized typed block with the requested element capacity.
    pub fn alloc_owned_slice<T: CompactValue>(capacity: usize) -> Result<CageAllocation<T>> {
        CageAllocation::allocate(capacity)
    }

    /// Allocate one initialized value.
    pub fn alloc_owned_value<T: CompactValue>(value: T) -> Result<CageAllocation<T>> {
        let mut allocation = Self::alloc_owned_slice::<T>(1)?;
        allocation.push(value)?;
        Ok(allocation)
    }

    /// Reserve one temporary bump region inside the process cage.
    pub fn scratch(capacity: usize) -> Result<crate::ScratchRegion> {
        crate::ScratchRegion::new(capacity)
    }

    /// Run a teardown or replacement operation and batch cage releases caused
    /// by its dropped values. The operation runs before any allocator lock is
    /// acquired; nested calls join the active thread-local batch.
    #[doc(hidden)]
    pub fn with_batched_releases<R>(operation: impl FnOnce() -> R) -> R {
        let previous = ACTIVE_RELEASE_COLLECTOR.with(Cell::get);
        if !previous.is_null() {
            return operation();
        }

        let mut collector = ReleaseCollector::new();
        let collector_pointer = &mut collector as *mut ReleaseCollector;
        let previous = ACTIVE_RELEASE_COLLECTOR.with(|active| active.replace(collector_pointer));
        if ENABLE_PENDING_REUSE {
            ACTIVE_RELEASE_COLLECTOR_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        let scope = ReleaseBatchScope {
            collector: collector_pointer,
            previous,
        };
        let result = operation();
        drop(scope);
        result
    }

    /// Check that an owner still names a live allocation in this process cage.
    pub fn validate_owned<T: CompactValue>(allocation: &CageAllocation<T>) -> Result<()> {
        let state = state()?;
        let header = read_typed_header::<T>(state, allocation.raw_offset())?;
        let allocator = lock(state)?;
        let start = allocation
            .raw_offset()
            .checked_sub(size_of::<AllocationHeader>() as u32)
            .and_then(|header_offset| header_offset.checked_sub(header.prefix))
            .ok_or(Error::InvalidOffset)?;
        let end = start
            .checked_add(header.block_len)
            .ok_or(Error::InvalidOffset)?;
        if end > allocator.cursor {
            return Err(Error::InvalidOffset);
        }
        let mut free = allocator.free_head;
        while free != 0 {
            let node = unsafe { read_free_node(state, free)? };
            let free_end = free.checked_add(node.len).ok_or(Error::InvalidOffset)?;
            if start < free_end && free < end {
                return Err(Error::InvalidOffset);
            }
            free = node.next;
        }
        for class_index in 0..SIZE_CLASSES.len() {
            let mut free = allocator.size_class_heads[class_index];
            let mut count = 0_u32;
            while free != 0 {
                let node = unsafe { read_free_node(state, free)? };
                let free_end = free.checked_add(node.len).ok_or(Error::InvalidOffset)?;
                if start < free_end && free < end {
                    return Err(Error::InvalidOffset);
                }
                count = count.checked_add(1).ok_or(Error::InvalidOffset)?;
                if count > SIZE_CLASS_CACHE_CAPACITY {
                    return Err(Error::InvalidOffset);
                }
                free = node.next;
            }
            if count != allocator.size_class_counts[class_index] {
                return Err(Error::InvalidOffset);
            }
        }
        if allocator.has_size_class_cache
            != allocator.size_class_counts.iter().any(|count| *count != 0)
        {
            return Err(Error::InvalidOffset);
        }
        Ok(())
    }

    /// Validate intrusive allocator ordering and live/free byte accounting.
    ///
    /// This diagnostic is intended for property tests, fuzzing, and audits.
    #[doc(hidden)]
    pub fn validate_allocator_state() -> Result<()> {
        let state = state()?;
        flush_current_local_cache(state);
        let allocator = lock(state)?;
        validate_allocator(state, &allocator)
    }

    /// Resolve an offset to a value.
    ///
    /// # Safety
    ///
    /// The offset must name a live initialized `T`, and an owner must keep
    /// that allocation alive and immutably borrowed for the returned lifetime.
    #[inline]
    pub unsafe fn resolve_unchecked<'a, T: CompactValue>(offset: Offset32<T>) -> Result<&'a T> {
        if offset.is_null() {
            return Err(Error::InvalidOffset);
        }
        let state = state()?;
        let header = read_typed_header::<T>(state, offset.as_u32())?;
        if header.initialized == 0 {
            return Err(Error::InitializationError);
        }
        // SAFETY: the caller upholds the lifetime and provenance contract.
        Ok(unsafe { &*ptr_from_offset::<T>(state, offset.as_u32()) })
    }

    /// Resolve an offset to a byte range.
    ///
    /// # Safety
    ///
    /// The range must be wholly inside a live byte allocation kept alive by
    /// an owner for the returned lifetime.
    #[inline]
    pub unsafe fn resolve_bytes_unchecked<'a>(offset: u32, len: usize) -> Result<&'a [u8]> {
        let state = state()?;
        let header = unsafe { read_header(state, offset) }?;
        if len > header.capacity as usize {
            return Err(Error::OutOfBounds);
        }
        // SAFETY: the caller guarantees liveness and initialization; the
        // allocation header bounds the requested byte range.
        Ok(unsafe { slice_from_offset(state, offset, len) })
    }
}

/// A unique compact allocation owner represented by one non-null `u32` offset.
///
/// The private invariant is that `offset` points to a live allocation created
/// by this allocator, and its header capacity and block length describe a
/// correctly aligned payload of exactly `T`. Only allocator construction and
/// resize paths may create or update this state. `offset()` exports a compact
/// descriptor, not a way to reconstruct an owner.
#[repr(transparent)]
#[must_use = "dropping this owner releases its cage allocation"]
pub struct CageAllocation<T: CompactValue> {
    offset: NonZeroOffset,
    marker: PhantomData<T>,
}

#[repr(transparent)]
#[derive(Clone, Copy)]
struct NonZeroOffset(core::num::NonZeroU32);

/// Temporary native view whose lifetime is tied to an owner borrow.
struct ResolvedAllocation<'a, T> {
    ptr: *mut T,
    header: AllocationHeader,
    marker: PhantomData<&'a [T]>,
}

impl<'a, T> ResolvedAllocation<'a, T> {
    fn as_slice(&self) -> &'a [T] {
        // SAFETY: the view was resolved from its live owner and its marker keeps
        // the owner borrowed for `'a`; the header records the initialized prefix.
        unsafe { slice::from_raw_parts(self.ptr, self.header.initialized as usize) }
    }
}

/// Temporary exclusive native view whose lifetime is tied to an owner borrow.
struct ResolvedAllocationMut<'a, T> {
    ptr: *mut T,
    header_ptr: *mut AllocationHeader,
    header: AllocationHeader,
    marker: PhantomData<&'a mut [T]>,
}

impl<'a, T> ResolvedAllocationMut<'a, T> {
    fn into_mut_slice(self) -> &'a mut [T] {
        // SAFETY: the view holds the unique owner borrow and the header records
        // exactly the initialized prefix.
        unsafe { slice::from_raw_parts_mut(self.ptr, self.header.initialized as usize) }
    }

    fn set_initialized(&mut self, initialized: u32) {
        debug_assert!(initialized <= self.header.capacity);
        self.header.initialized = initialized;
        // SAFETY: this view is exclusively borrowed from the live allocation.
        unsafe { (*self.header_ptr).initialized = initialized };
    }
}

struct AppendInitGuard<'a, T> {
    ptr: *mut T,
    header_ptr: *mut AllocationHeader,
    start: u32,
    written: u32,
    marker: PhantomData<&'a mut [T]>,
}

impl<'a, T> AppendInitGuard<'a, T> {
    fn new<'v>(view: &'v mut ResolvedAllocationMut<'_, T>) -> AppendInitGuard<'v, T> {
        AppendInitGuard {
            ptr: view.ptr,
            header_ptr: view.header_ptr,
            start: view.header.initialized,
            written: 0,
            marker: PhantomData,
        }
    }
}

impl<T> Drop for AppendInitGuard<'_, T> {
    fn drop(&mut self) {
        // Publish the initialized prefix even if the iterator or constructor
        // panicked. Any already-written values will then be dropped normally.
        // SAFETY: created while holding the owner's exclusive borrow.
        unsafe { (*self.header_ptr).initialized = self.start + self.written };
    }
}

impl<T: CompactValue> CageAllocation<T> {
    fn allocate(capacity: usize) -> Result<Self> {
        let capacity_u32 = u32::try_from(capacity).map_err(|_| Error::OffsetOverflow)?;
        let bytes = size_of::<T>()
            .checked_mul(capacity)
            .ok_or(Error::OffsetOverflow)?;
        let state = state()?;
        let needed = bytes.max(1);
        let alignment = align_of::<T>().max(4);
        #[cfg(feature = "allocator-telemetry")]
        let pending_lookup_phase = PhaseTimer::start(&ALLOCATOR_PHASE_TELEMETRY.pending_lookup_ns);
        let (lookup, recycled) = take_pending_reuse(state, needed, alignment);
        #[cfg(feature = "allocator-telemetry")]
        drop(pending_lookup_phase);
        if let Some(recycled) = recycled {
            debug_assert_eq!(lookup, PendingLookup::Recycled);
            record_pending_lookup(PendingLookup::Recycled, recycled.block_len);
            let header = AllocationHeader {
                block_len: recycled.block_len,
                prefix: recycled.prefix,
                capacity: capacity_u32,
                initialized: 0,
            };
            // SAFETY: the exact pending extent was removed from the
            // thread-local collector after all layout checks succeeded.
            initialize_allocation_header(state, recycled.data_offset.get(), header);
            return Ok(Self {
                offset: NonZeroOffset(recycled.data_offset),
                marker: PhantomData,
            });
        } else {
            debug_assert_ne!(lookup, PendingLookup::Recycled);
            record_pending_lookup(lookup, 0);
        }
        if let Some(recycled) = take_local_reuse(state, needed, alignment, capacity_u32) {
            record_pending_lookup(PendingLookup::Recycled, recycled.block_len);
            return Ok(Self {
                offset: NonZeroOffset(recycled.data_offset),
                marker: PhantomData,
            });
        }
        let mut allocator = lock_for_allocation(state)?;
        let (data_offset, prefix, block_len) =
            allocate_block(state, &mut allocator, needed, alignment)?;
        let offset = core::num::NonZeroU32::new(data_offset)
            .expect("cage allocations include a nonzero header and payload offset");
        let header = AllocationHeader {
            block_len,
            prefix,
            capacity: capacity_u32,
            initialized: 0,
        };
        // SAFETY: `allocate_block` reserves this aligned header and data range.
        initialize_allocation_header(state, offset.get(), header);
        Ok(Self {
            offset: NonZeroOffset(offset),
            marker: PhantomData,
        })
    }

    /// Return the initialized element count.
    #[inline]
    pub fn len(&self) -> usize {
        self.header().expect("live cage owner header").initialized as usize
    }
    /// Return the allocated element capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.header().expect("live cage owner header").capacity as usize
    }
    /// Return initialized length and capacity from one resolved header read.
    #[doc(hidden)]
    #[inline]
    pub fn len_capacity(&self) -> (usize, usize) {
        let header = self.header().expect("live cage owner header");
        (header.initialized as usize, header.capacity as usize)
    }
    /// Return whether the allocation has no initialized elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Return this allocation's typed offset.
    #[inline]
    pub fn offset(&self) -> Offset32<T> {
        // SAFETY: this owner always contains the allocator-issued live offset.
        unsafe { Offset32::from_raw_unchecked(self.raw_offset()) }
    }
    /// Borrow the initialized prefix.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        self.resolved().expect("live cage owner header").as_slice()
    }
    /// Mutably borrow the initialized prefix.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.resolved_mut()
            .expect("live cage owner header")
            .into_mut_slice()
    }
    /// Return an initialized element by index.
    #[inline]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.resolved()
            .expect("live cage owner header")
            .as_slice()
            .get(index)
    }
    /// Mutably borrow an initialized element by index.
    #[inline]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.resolved_mut()
            .expect("live cage owner header")
            .into_mut_slice()
            .get_mut(index)
    }
    /// Initialize the next element slot.
    #[inline]
    pub fn push(&mut self, value: T) -> Result<()> {
        let mut resolved = self.resolved_mut()?;
        if resolved.header.initialized >= resolved.header.capacity {
            return Err(Error::OutOfBounds);
        }
        let index = resolved.header.initialized as usize;
        // SAFETY: this is the next uninitialized slot within the owner.
        unsafe { resolved.ptr.add(index).write(value) };
        resolved.set_initialized(index as u32 + 1);
        Ok(())
    }
    /// Remove and return the final initialized element.
    pub fn pop(&mut self) -> Option<T> {
        let mut resolved = self.resolved_mut().expect("live cage owner header");
        let len = resolved.header.initialized as usize;
        if len == 0 {
            return None;
        }
        resolved.set_initialized((len - 1) as u32);
        // SAFETY: the element was initialized and removed from the drop prefix.
        Some(unsafe { resolved.ptr.add(len - 1).read() })
    }
    /// Drop initialized elements until `new_len` remain.
    pub fn truncate(&mut self, new_len: usize) {
        if core::mem::needs_drop::<T>() {
            CompactRuntime::with_batched_releases(|| self.truncate_inner(new_len));
        } else {
            self.truncate_inner(new_len);
        }
    }

    fn truncate_inner(&mut self, new_len: usize) {
        let mut guard = TruncateGuard {
            allocation: self,
            new_len,
            armed: true,
        };
        // SAFETY: the guard is created from this exclusive owner borrow.
        let allocation = unsafe { &mut *guard.allocation };
        let mut resolved = allocation.resolved_mut().expect("live cage owner header");
        if !core::mem::needs_drop::<T>() {
            let target_len = new_len.min(resolved.header.initialized as usize);
            resolved.set_initialized(target_len as u32);
            guard.armed = false;
            return;
        }
        while resolved.header.initialized as usize > new_len {
            let index = resolved.header.initialized as usize - 1;
            resolved.set_initialized(index as u32);
            // SAFETY: length was lowered first, so unwinding cannot double-drop this element.
            unsafe { resolved.ptr.add(index).drop_in_place() };
        }
        guard.armed = false;
    }
    /// Append a copyable slice to the initialized prefix.
    pub fn extend_copy(&mut self, values: &[T]) -> Result<()>
    where
        T: Copy,
    {
        let mut resolved = self.resolved_mut()?;
        let start = resolved.header.initialized as usize;
        let end = start
            .checked_add(values.len())
            .ok_or(Error::OffsetOverflow)?;
        if end > resolved.header.capacity as usize {
            return Err(Error::OutOfBounds);
        }
        if !values.is_empty() {
            // SAFETY: destination is uninitialized and the source is valid.
            unsafe {
                resolved
                    .ptr
                    .add(start)
                    .copy_from_nonoverlapping(values.as_ptr(), values.len())
            };
        }
        resolved.set_initialized(end as u32);
        Ok(())
    }

    /// Append values from an iterator into available capacity, resolving the
    /// allocation once. A panic publishes the exact initialized prefix.
    #[doc(hidden)]
    pub fn extend_from_iter(
        &mut self,
        iterator: &mut impl Iterator<Item = T>,
        max: usize,
    ) -> Result<usize> {
        let mut resolved = self.resolved_mut()?;
        let start = resolved.header.initialized as usize;
        let available = resolved.header.capacity as usize - start;
        if max > available {
            return Err(Error::OutOfBounds);
        }
        let mut guard = AppendInitGuard::new(&mut resolved);
        while (guard.written as usize) < max {
            let Some(value) = iterator.next() else {
                break;
            };
            // SAFETY: max was checked against available capacity and each slot
            // is written once before the guard publishes it as initialized.
            unsafe { guard.ptr.add(start + guard.written as usize).write(value) };
            guard.written += 1;
        }
        Ok(guard.written as usize)
    }

    /// Append values from a fallible producer into available capacity. Source
    /// errors are saved for the caller after the successfully produced prefix
    /// has been published.
    #[doc(hidden)]
    pub fn extend_from_fallible_fn<E>(
        &mut self,
        max: usize,
        mut next: impl FnMut() -> core::result::Result<Option<T>, E>,
        source_error: &mut Option<E>,
    ) -> Result<usize> {
        let mut resolved = self.resolved_mut()?;
        let start = resolved.header.initialized as usize;
        let available = resolved.header.capacity as usize - start;
        if max > available {
            return Err(Error::OutOfBounds);
        }
        let mut guard = AppendInitGuard::new(&mut resolved);
        while (guard.written as usize) < max {
            let value = match next() {
                Ok(Some(value)) => value,
                Ok(None) => break,
                Err(error) => {
                    *source_error = Some(error);
                    break;
                }
            };
            // SAFETY: max was checked against available capacity and each value
            // is written before the guard publishes it as initialized.
            unsafe { guard.ptr.add(start + guard.written as usize).write(value) };
            guard.written += 1;
        }
        Ok(guard.written as usize)
    }

    /// Construct and append `count` values, publishing an initialized prefix
    /// safely if the constructor panics.
    #[doc(hidden)]
    pub fn extend_from_fn(
        &mut self,
        count: usize,
        mut make_value: impl FnMut() -> T,
    ) -> Result<()> {
        let mut resolved = self.resolved_mut()?;
        let start = resolved.header.initialized as usize;
        let available = resolved.header.capacity as usize - start;
        if count > available {
            return Err(Error::OutOfBounds);
        }
        let mut guard = AppendInitGuard::new(&mut resolved);
        while (guard.written as usize) < count {
            let value = make_value();
            // SAFETY: count was checked against available capacity and each
            // produced value is written before the guard publishes it.
            unsafe { guard.ptr.add(start + guard.written as usize).write(value) };
            guard.written += 1;
        }
        Ok(())
    }
    /// Move the initialized prefix into an empty owner of the same type.
    pub fn move_into(&mut self, destination: &mut Self) -> Result<()> {
        let mut source = self.resolved_mut()?;
        let mut destination = destination.resolved_mut()?;
        if destination.header.initialized != 0 {
            return Err(Error::InitializationError);
        }
        let len = source.header.initialized as usize;
        if len > destination.header.capacity as usize {
            return Err(Error::OutOfBounds);
        }
        if len != 0 {
            // SAFETY: the owners are distinct, the source prefix is initialized,
            // and the destination range is uninitialized and large enough.
            unsafe { destination.ptr.copy_from_nonoverlapping(source.ptr, len) };
        }
        source.set_initialized(0);
        destination.set_initialized(len as u32);
        Ok(())
    }
    /// Move initialized values from an inline uninitialized slice.
    ///
    /// # Safety
    ///
    /// `source` must point to `len` initialized, aligned `T` values. The source
    /// slots must not overlap the destination allocation and become
    /// uninitialized; they must not be read or dropped afterward.
    pub unsafe fn move_from_uninit_slice(
        &mut self,
        source: *mut MaybeUninit<T>,
        len: usize,
    ) -> Result<()> {
        let mut resolved = self.resolved_mut()?;
        let start = resolved.header.initialized as usize;
        let end = start.checked_add(len).ok_or(Error::OffsetOverflow)?;
        if end > resolved.header.capacity as usize {
            return Err(Error::OutOfBounds);
        }
        for index in 0..len {
            // SAFETY: guaranteed by the caller; the destination is in capacity.
            let value = unsafe { source.add(index).cast::<T>().read() };
            unsafe { resolved.ptr.add(start + index).write(value) };
        }
        resolved.set_initialized(end as u32);
        Ok(())
    }
    /// Try to change capacity without relocating the allocation.
    pub fn try_resize(&mut self, capacity: usize) -> Result<bool> {
        let requested = u32::try_from(capacity).map_err(|_| Error::OffsetOverflow)?;
        let state = state()?;
        let offset = self.raw_offset();
        let header = read_owner_header::<T>(state, self)?;
        if requested < header.initialized {
            return Err(Error::InitializationError);
        }
        let bytes = size_of::<T>()
            .checked_mul(capacity)
            .ok_or(Error::OffsetOverflow)?
            .max(1);
        let mut allocator = lock_for_allocation(state)?;
        if self.try_resize_locked(state, &mut allocator, offset, header, requested, bytes)? {
            return Ok(true);
        }
        drop(allocator);
        if state.local_cache_bytes.load(Ordering::Acquire) == 0 {
            return Ok(false);
        }
        flush_current_local_cache(state);
        let mut allocator = lock_for_allocation(state)?;
        self.try_resize_locked(state, &mut allocator, offset, header, requested, bytes)
    }

    fn try_resize_locked(
        &self,
        state: &CageState,
        allocator: &mut Allocator,
        offset: u32,
        header: AllocationHeader,
        requested: u32,
        bytes: usize,
    ) -> Result<bool> {
        if allocator.size_class_counts.iter().any(|count| *count != 0) {
            // Resize uses the ordered free list to consume adjacent blocks.
            // Merge cached extents first so no class-owned neighbor is missed.
            merge_free_ranges_locked(state, allocator)?;
        }
        let start = self
            .raw_offset()
            .checked_sub(size_of::<AllocationHeader>() as u32)
            .and_then(|header_offset| header_offset.checked_sub(header.prefix))
            .ok_or(Error::InvalidOffset)?;
        let old_len = header.block_len;
        let new_len = u32::try_from(checked_align_up(
            (header.prefix as usize)
                .checked_add(size_of::<AllocationHeader>())
                .and_then(|n| n.checked_add(bytes))
                .ok_or(Error::OffsetOverflow)?,
            8,
        )?)
        .map_err(|_| Error::OffsetOverflow)?;
        if new_len <= old_len {
            if new_len < old_len {
                insert_free(state, allocator, start + new_len, old_len - new_len)?;
            }
            allocator.live_bytes = allocator.live_bytes - old_len + new_len;
            let mut changed = header;
            changed.block_len = new_len;
            changed.capacity = requested;
            // SAFETY: this owner uniquely represents the live allocation.
            unsafe { header_ptr(state, offset).write(changed) };
            return Ok(true);
        }
        let end = start.checked_add(old_len).ok_or(Error::OffsetOverflow)?;
        let extra = new_len - old_len;
        if end == allocator.cursor {
            if allocator
                .cursor
                .checked_add(extra)
                .ok_or(Error::OffsetOverflow)?
                > state.capacity as u32
            {
                return Ok(false);
            }
            allocator.cursor += extra;
        } else if let Some((free_len, _next)) = free_node_at(state, allocator, end)? {
            if free_len < extra {
                return Ok(false);
            }
            consume_free_prefix(state, allocator, end, extra)?;
        } else {
            return Ok(false);
        }
        allocator.live_bytes = allocator
            .live_bytes
            .checked_add(extra)
            .ok_or(Error::OffsetOverflow)?;
        let mut changed = header;
        changed.block_len = new_len;
        changed.capacity = requested;
        // SAFETY: this owner uniquely represents the live allocation.
        unsafe { header_ptr(state, offset).write(changed) };
        Ok(true)
    }
    /// Borrow the full capacity as potentially uninitialized slots.
    #[inline]
    pub fn uninit_capacity(&self) -> &[MaybeUninit<T>] {
        let resolved = self.resolved().expect("live cage owner header");
        // SAFETY: MaybeUninit permits reading every slot state; the owner borrow
        // keeps the allocation live for the returned slice.
        unsafe {
            slice::from_raw_parts(
                resolved.ptr.cast::<MaybeUninit<T>>(),
                resolved.header.capacity as usize,
            )
        }
    }
    /// Mutably borrow the full capacity as potentially uninitialized slots.
    #[inline]
    pub fn uninit_capacity_mut(&mut self) -> &mut [MaybeUninit<T>] {
        let resolved = self.resolved_mut().expect("live cage owner header");
        let ptr = resolved.ptr;
        let capacity = resolved.header.capacity as usize;
        // SAFETY: the unique owner is mutably borrowed and every capacity slot is writable.
        unsafe { slice::from_raw_parts_mut(ptr.cast::<MaybeUninit<T>>(), capacity) }
    }
    #[inline]
    fn raw_offset(&self) -> u32 {
        self.offset.0.get()
    }
    #[inline]
    fn header(&self) -> Result<AllocationHeader> {
        let state = state()?;
        read_owner_header::<T>(state, self)
    }
    #[inline]
    fn resolved(&self) -> Result<ResolvedAllocation<'_, T>> {
        let state = state()?;
        let offset = self.raw_offset();
        let header = read_owner_header::<T>(state, self)?;
        // SAFETY: the validated owner offset points to its aligned payload.
        let ptr = unsafe { ptr_from_offset::<T>(state, offset) };
        Ok(ResolvedAllocation {
            ptr,
            header,
            marker: PhantomData,
        })
    }
    #[inline]
    fn resolved_mut(&mut self) -> Result<ResolvedAllocationMut<'_, T>> {
        let state = state()?;
        let offset = self.raw_offset();
        let header = read_owner_header::<T>(state, self)?;
        // SAFETY: the validated owner offset points to its aligned payload/header.
        let ptr = unsafe { ptr_from_offset::<T>(state, offset) };
        let header_ptr = unsafe { header_ptr(state, offset) };
        Ok(ResolvedAllocationMut {
            ptr,
            header_ptr,
            header,
            marker: PhantomData,
        })
    }
}

impl CageAllocation<u64> {
    /// Borrow the initialized words as their exact byte representation.
    pub fn as_byte_slice(&self) -> &[u8] {
        let resolved = self.resolved().expect("live cage owner header");
        let len = (resolved.header.initialized as usize)
            .checked_mul(size_of::<u64>())
            .expect("cage byte length fits its allocation");
        // SAFETY: every initialized `u64` has all bytes initialized, and the
        // returned slice is tied to this immutable owner borrow.
        unsafe { slice::from_raw_parts(resolved.ptr.cast::<u8>(), len) }
    }
}

struct TruncateGuard<T: CompactValue> {
    allocation: *mut CageAllocation<T>,
    new_len: usize,
    armed: bool,
}
impl<T: CompactValue> Drop for TruncateGuard<T> {
    fn drop(&mut self) {
        if self.armed {
            // SAFETY: guard is created from an exclusive owner borrow and runs only during unwind.
            unsafe { (*self.allocation).truncate(self.new_len) };
        }
    }
}

struct ReleaseGuard(u32);
impl Drop for ReleaseGuard {
    fn drop(&mut self) {
        release(self.0);
    }
}

impl<T: CompactValue> Drop for CageAllocation<T> {
    fn drop(&mut self) {
        if core::mem::needs_drop::<T>() {
            CompactRuntime::with_batched_releases(|| {
                let _release = ReleaseGuard(self.raw_offset());
                self.truncate_inner(0);
            });
        } else {
            let _release = ReleaseGuard(self.raw_offset());
            self.truncate_inner(0);
        }
    }
}

unsafe impl<T: CompactValue> CompactValue for CageAllocation<T> {}

#[inline]
fn state() -> Result<&'static CageState> {
    CAGE.get().ok_or(Error::RuntimeNotInitialized)
}
fn lock(state: &CageState) -> Result<AllocatorTransaction<'_>> {
    #[cfg(feature = "allocator-telemetry")]
    let lock_wait_phase = PhaseTimer::start(&ALLOCATOR_PHASE_TELEMETRY.lock_wait_ns);
    let lock_result = state.allocator.lock();
    #[cfg(feature = "allocator-telemetry")]
    drop(lock_wait_phase);
    finish_lock(state, lock_result)
}

fn lock_for_allocation(state: &CageState) -> Result<AllocatorTransaction<'_>> {
    #[cfg(feature = "allocator-telemetry")]
    let lock_wait_phase = PhaseTimer::start(&ALLOCATOR_PHASE_TELEMETRY.lock_wait_ns);
    let lock_result = if state.local_reuse_activated.load(Ordering::Acquire) {
        state.allocator.lock()
    } else {
        match state.allocator.try_lock() {
            // The uncontended try-lock is the acquisition, so the disabled
            // cache path does not add a separate probe lock or touch TLS.
            Ok(allocator) => Ok(allocator),
            Err(TryLockError::WouldBlock) => {
                state.local_reuse_activated.store(true, Ordering::Release);
                state.allocator.lock()
            }
            Err(TryLockError::Poisoned(poisoned)) => Err(poisoned),
        }
    };
    #[cfg(feature = "allocator-telemetry")]
    drop(lock_wait_phase);
    finish_lock(state, lock_result)
}

fn finish_lock<'a>(
    state: &'a CageState,
    lock_result: std::sync::LockResult<MutexGuard<'a, Allocator>>,
) -> Result<AllocatorTransaction<'a>> {
    let mut allocator = lock_result.map_err(|_| Error::AllocatorPoisoned)?;
    if state.allocator_faulted.load(Ordering::Acquire) {
        return Err(Error::AllocatorPoisoned);
    }
    #[cfg(feature = "allocator-telemetry")]
    {
        allocator.lock_acquisitions = allocator.lock_acquisitions.saturating_add(1);
    }
    drain_pending_releases_locked(state, &mut allocator)?;
    Ok(AllocatorTransaction { state, allocator })
}

fn reserve_local_cache_owner(active_owners: &AtomicUsize) -> bool {
    let mut current = active_owners.load(Ordering::Acquire);
    loop {
        if current >= MAX_ACTIVE_LOCAL_CACHE_OWNERS {
            return false;
        }
        match active_owners.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
}

fn flush_local_cache_state(state: &CageState, cache: &mut LocalCacheState) {
    if cache.len == 0 {
        return;
    }
    let cached_bytes = cache.bytes();
    let mut extents = cache.extents;
    let len = cache.len.min(LOCAL_CACHE_CAPACITY);
    if release_many(&mut extents[..len]).is_err() {
        for extent in extents[..len].iter().copied() {
            enqueue_pending_release(state, extent);
        }
    }
    cache.clear();
    state
        .local_cache_bytes
        .fetch_sub(cached_bytes, Ordering::AcqRel);
}

fn flush_current_local_cache(state: &CageState) {
    let _ = LOCAL_REUSE_CACHE.try_with(|slot| {
        if let Ok(mut cache) = slot.cache.try_borrow_mut() {
            flush_local_cache_state(state, &mut cache);
        }
    });
}

/// Apply at most one fixed-size batch of published releases.
/// Unprocessed descriptors remain linked from the queue head.
fn drain_pending_releases_locked(state: &CageState, allocator: &mut Allocator) -> Result<()> {
    if state.allocator_faulted.load(Ordering::Acquire) {
        return Err(Error::AllocatorPoisoned);
    }
    if !state.pending_release_nonempty.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut queue = match state.pending_releases.lock() {
        Ok(queue) => queue,
        Err(poisoned) => {
            state.allocator_faulted.store(true, Ordering::Release);
            drop(poisoned.into_inner());
            return Err(Error::AllocatorPoisoned);
        }
    };
    let original_head = queue.head;
    if original_head == 0 {
        state
            .pending_release_nonempty
            .store(false, Ordering::Release);
        return Ok(());
    }

    let mut extents = [ReleaseExtent::default(); MAX_PENDING_RELEASE_DRAIN];
    let mut len = 0;
    let mut current = original_head;
    while current != 0 && len < extents.len() {
        // SAFETY: release descriptors are in-cage nodes; read_free_node checks
        // the offset and node header bounds before reading it.
        let node = match unsafe { read_free_node(state, current) } {
            Ok(node) => node,
            Err(error) => {
                state.allocator_faulted.store(true, Ordering::Release);
                return Err(error);
            }
        };
        let extent = ReleaseExtent {
            start: current,
            len: node.len,
        };
        if validate_free_extent(allocator, extent).is_err()
            || extents[..len]
                .iter()
                .any(|previous| previous.start == extent.start)
        {
            state.allocator_faulted.store(true, Ordering::Release);
            return Err(Error::AllocatorPoisoned);
        }
        extents[len] = extent;
        len += 1;
        current = node.next;
    }

    if let Err(error) = release_many_locked(
        state,
        allocator,
        &mut extents[..len],
        ENABLE_SIZE_CLASS_CACHE,
    ) {
        // The batch validator runs before its mutation phase. Restoring the
        // chain keeps all descriptors authoritative while the allocator is
        // failed closed. The queue head remains at its original descriptor.
        state.allocator_faulted.store(true, Ordering::Release);
        return Err(error);
    }

    queue.head = current;
    state
        .pending_release_nonempty
        .store(current != 0, Ordering::Release);
    Ok(())
}

fn enqueue_pending_release(state: &CageState, extent: ReleaseExtent) {
    if extent.len < FREE_NODE_BYTES
        || extent.len % 8 != 0
        || extent.start < INITIAL_CURSOR
        || extent
            .start
            .checked_add(extent.len)
            .map_or(true, |end| end as usize > state.capacity)
    {
        state.allocator_faulted.store(true, Ordering::Release);
        return;
    }

    let mut queue = match state.pending_releases.lock() {
        Ok(queue) => queue,
        Err(poisoned) => {
            state.allocator_faulted.store(true, Ordering::Release);
            poisoned.into_inner()
        }
    };
    // SAFETY: the released extent remains exclusively owned by this
    // descriptor until the consumer removes it under the queue mutex.
    unsafe {
        write_free_node(
            state,
            extent.start,
            FreeNode {
                next: queue.head,
                len: extent.len,
            },
        )
    };
    queue.head = extent.start;
    state
        .pending_release_nonempty
        .store(true, Ordering::Release);
}

fn initialize_allocation_header(state: &CageState, offset: u32, header: AllocationHeader) {
    #[cfg(feature = "allocator-telemetry")]
    let _header_initialization_phase =
        PhaseTimer::start(&ALLOCATOR_PHASE_TELEMETRY.header_initialization_ns);
    // SAFETY: callers pass the data offset for a newly reserved live extent.
    unsafe { header_ptr(state, offset).write(header) };
}

fn local_reuse_enabled(state: &CageState) -> bool {
    ENABLE_PENDING_REUSE
        && state.local_reuse_activated.load(Ordering::Acquire)
        && state.local_cache_budget != 0
        && !state.allocator_faulted.load(Ordering::Acquire)
}

fn local_reuse_eligible(bytes: usize, alignment: usize) -> bool {
    if alignment > 8 {
        return false;
    }
    let Ok(bytes) = u32::try_from(bytes) else {
        return false;
    };
    bytes
        .checked_add(size_of::<AllocationHeader>() as u32 + 7)
        .is_some_and(|len| size_class_index(len & !7).is_some())
}

fn with_local_reuse_cache<R>(
    state: &CageState,
    create: bool,
    operation: impl FnOnce(&mut LocalCacheState) -> R,
) -> Option<R> {
    LOCAL_REUSE_CACHE
        .try_with(|slot| {
            let mut cache = slot.cache.try_borrow_mut().ok()?;
            if !slot.registered.get() {
                if !create || !reserve_local_cache_owner(&state.active_local_cache_owners) {
                    return None;
                }
                slot.registered.set(true);
            }
            Some(operation(&mut cache))
        })
        .ok()
        .flatten()
}

fn take_local_reuse(
    state: &CageState,
    bytes: usize,
    alignment: usize,
    capacity: u32,
) -> Option<RecycledExtent> {
    if state.local_cache_bytes.load(Ordering::Acquire) == 0 {
        return None;
    }
    if !local_reuse_eligible(bytes, alignment) || !local_reuse_enabled(state) {
        return None;
    }
    let (_, _, wanted_len) = block_layout(state.base(), INITIAL_CURSOR, bytes, alignment).ok()?;
    with_local_reuse_cache(state, false, |cache| {
        cache.take_compatible(
            &state.local_cache_bytes,
            wanted_len,
            |extent| {
                let (data_offset, prefix, block_len) =
                    block_layout(state.base(), extent.start, bytes, alignment).ok()?;
                if block_len != extent.len {
                    return None;
                }
                Some(RecycledExtent {
                    data_offset: core::num::NonZeroU32::new(data_offset)?,
                    prefix,
                    block_len,
                })
            },
            |recycled| {
                let header = AllocationHeader {
                    block_len: recycled.block_len,
                    prefix: recycled.prefix,
                    capacity,
                    initialized: 0,
                };
                // SAFETY: the exact compatible extent is exclusively held by
                // this thread's mutable cache borrow until its header is
                // initialized and the descriptor is removed.
                initialize_allocation_header(state, recycled.data_offset.get(), header);
            },
        )
    })
    .flatten()
}

fn cache_released_extent(state: &CageState, extent: ReleaseExtent) -> bool {
    if size_class_index(extent.len).is_none() {
        return false;
    }
    if !local_reuse_enabled(state) {
        return false;
    }
    let Some(publish) = with_local_reuse_cache(state, true, |cache| {
        cache.push(
            extent,
            &state.local_cache_bytes,
            state.local_cache_budget,
            |candidate| size_class_index(candidate.len).is_some(),
        )
    }) else {
        return false;
    };
    for extent in publish.into_iter().flatten() {
        let mut one = [extent];
        let _ = release_many(&mut one);
    }
    true
}

fn take_pending_reuse(
    state: &CageState,
    bytes: usize,
    alignment: usize,
) -> (PendingLookup, Option<RecycledExtent>) {
    if !ENABLE_PENDING_REUSE {
        let _ = (state, bytes, alignment);
        (PendingLookup::Disabled, None)
    } else if ACTIVE_RELEASE_COLLECTOR_COUNT.load(Ordering::Relaxed) == 0 {
        (PendingLookup::NoCollector, None)
    } else {
        ACTIVE_RELEASE_COLLECTOR.with(|active| {
            let collector = active.get();
            if collector.is_null() {
                return (PendingLookup::NoCollector, None);
            }
            // SAFETY: this pointer is installed only while its stack-local
            // collector is alive on this thread, and this closure runs on the
            // same thread before any collector flush can occur.
            unsafe { (*collector).take_compatible(state.base(), bytes, alignment) }
        })
    }
}

fn record_pending_lookup(lookup: PendingLookup, block_len: u32) {
    #[cfg(feature = "allocator-telemetry")]
    match lookup {
        PendingLookup::Disabled => {}
        PendingLookup::NoCollector => {
            PENDING_REUSE_TELEMETRY
                .misses
                .fetch_add(1, Ordering::Relaxed);
            PENDING_REUSE_TELEMETRY
                .no_active_collector
                .fetch_add(1, Ordering::Relaxed);
        }
        PendingLookup::NoExactBlock {
            alignment_incompatible,
        } => {
            PENDING_REUSE_TELEMETRY
                .misses
                .fetch_add(1, Ordering::Relaxed);
            PENDING_REUSE_TELEMETRY
                .no_exact_block
                .fetch_add(1, Ordering::Relaxed);
            if alignment_incompatible {
                PENDING_REUSE_TELEMETRY
                    .alignment_incompatible
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        PendingLookup::Recycled => {
            PENDING_REUSE_TELEMETRY.hits.fetch_add(1, Ordering::Relaxed);
            let bucket = ((block_len as usize) / 8).min(BLOCK_SIZE_BUCKETS - 1);
            PENDING_ALLOCATION_SIZE_HISTOGRAM[bucket].fetch_add(1, Ordering::Relaxed);
        }
    }
    #[cfg(not(feature = "allocator-telemetry"))]
    let _ = (lookup, block_len);
}

#[cfg(feature = "allocator-telemetry")]
fn record_pending_scan_depth(scanned: usize) {
    let depth = scanned.min(RELEASE_BATCH_CAPACITY);
    PENDING_REUSE_TELEMETRY
        .scan_candidates
        .fetch_add(depth as u64, Ordering::Relaxed);
    PENDING_REUSE_TELEMETRY.scan_depth_histogram[depth].fetch_add(1, Ordering::Relaxed);
}

#[cfg(feature = "allocator-telemetry")]
fn record_pending_candidate_size(block_len: u32) {
    let bucket = ((block_len as usize) / 8).min(BLOCK_SIZE_BUCKETS - 1);
    PENDING_REUSE_TELEMETRY.candidate_size_histogram[bucket].fetch_add(1, Ordering::Relaxed);
}

fn block_layout(
    base: *mut u8,
    start: u32,
    bytes: usize,
    alignment: usize,
) -> Result<(u32, u32, u32)> {
    #[cfg(feature = "allocator-telemetry")]
    let _layout_phase = PhaseTimer::start(&ALLOCATOR_PHASE_TELEMETRY.layout_ns);
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(Error::AlignmentError);
    }
    if alignment <= 8 {
        let block_start_address = (base as usize)
            .checked_add(start as usize)
            .ok_or(Error::OffsetOverflow)?;
        if block_start_address % 8 == 0 {
            let data = start
                .checked_add(size_of::<AllocationHeader>() as u32)
                .ok_or(Error::OffsetOverflow)?;
            let raw = size_of::<AllocationHeader>()
                .checked_add(bytes)
                .ok_or(Error::OffsetOverflow)?;
            let len = checked_align_up(raw, 8)?;
            return Ok((
                data,
                0,
                u32::try_from(len).map_err(|_| Error::OffsetOverflow)?,
            ));
        }
    }
    block_layout_general(base, start, bytes, alignment)
}

fn block_layout_general(
    base: *mut u8,
    start: u32,
    bytes: usize,
    alignment: usize,
) -> Result<(u32, u32, u32)> {
    let after_header = (base as usize)
        .checked_add(start as usize)
        .and_then(|n| n.checked_add(size_of::<AllocationHeader>()))
        .ok_or(Error::OffsetOverflow)?;
    let data_address =
        checked_align_up(after_header, alignment.max(align_of::<AllocationHeader>()))?;
    let data = data_address
        .checked_sub(base as usize)
        .ok_or(Error::OffsetOverflow)?;
    let header_offset = data
        .checked_sub(size_of::<AllocationHeader>())
        .ok_or(Error::OffsetOverflow)?;
    let prefix = header_offset
        .checked_sub(start as usize)
        .ok_or(Error::OffsetOverflow)?;
    let raw = prefix
        .checked_add(size_of::<AllocationHeader>())
        .and_then(|n| n.checked_add(bytes))
        .ok_or(Error::OffsetOverflow)?;
    let len = checked_align_up(raw, 8)?;
    Ok((
        u32::try_from(data).map_err(|_| Error::OffsetOverflow)?,
        u32::try_from(prefix).map_err(|_| Error::OffsetOverflow)?,
        u32::try_from(len).map_err(|_| Error::OffsetOverflow)?,
    ))
}

/// Canonical conversion from a cage-relative offset to a temporary native pointer.
///
/// # Safety
///
/// `offset` must be within the cage allocation. The caller must ensure the
/// resulting pointer is used only while the owning cage allocation remains live.
#[inline]
unsafe fn ptr_from_offset<T>(state: &CageState, offset: u32) -> *mut T {
    // SAFETY: the caller proves the offset is within the cage allocation.
    unsafe { state.base().add(offset as usize).cast::<T>() }
}

/// Canonical conversion from a cage-relative byte range to a temporary slice.
///
/// # Safety
///
/// The caller proves that `offset..offset + len` lies in one live allocation,
/// that every byte is initialized, and that the returned borrow is tied to its owner.
unsafe fn slice_from_offset<'a>(state: &CageState, offset: u32, len: usize) -> &'a [u8] {
    // SAFETY: delegated to the caller; pointer formation is centralized above.
    unsafe { slice::from_raw_parts(ptr_from_offset::<u8>(state, offset), len) }
}

/// Resolve the common header immediately before an allocation's data offset.
///
/// # Safety
///
/// `offset` must be an allocator-issued, live owner offset or validated by the
/// caller's unsafe contract.
#[inline]
unsafe fn header_ptr(state: &CageState, offset: u32) -> *mut AllocationHeader {
    let header_offset = offset
        .checked_sub(size_of::<AllocationHeader>() as u32)
        .expect("live allocation has a preceding header");
    // SAFETY: guaranteed by this function's caller.
    unsafe { ptr_from_offset(state, header_offset) }
}

#[inline]
unsafe fn read_header(state: &CageState, offset: u32) -> Result<AllocationHeader> {
    let header_offset = offset
        .checked_sub(size_of::<AllocationHeader>() as u32)
        .ok_or(Error::InvalidOffset)?;
    if offset as usize > state.capacity {
        return Err(Error::InvalidOffset);
    }
    // SAFETY: caller supplies an allocator-issued offset or upholds the unsafe contract.
    let header = unsafe { ptr_from_offset::<AllocationHeader>(state, header_offset).read() };
    let prefix_and_header = header
        .prefix
        .checked_add(size_of::<AllocationHeader>() as u32)
        .ok_or(Error::InvalidOffset)?;
    let block_start = header_offset
        .checked_sub(header.prefix)
        .ok_or(Error::InvalidOffset)?;
    let block_end = block_start
        .checked_add(header.block_len)
        .ok_or(Error::InvalidOffset)?;
    if header.block_len < prefix_and_header
        || header.block_len == 0
        || block_end as usize > state.capacity
        || header.initialized > header.capacity
    {
        return Err(Error::InvalidOffset);
    }
    Ok(header)
}

/// Read a header through the private, allocator-issued owner path.
///
/// This retains all generic bounds and initialization checks in
/// [`read_header`]. It omits [`validate_typed_header`] because the owner can
/// only be minted by [`CageAllocation::allocate`], which reserves enough bytes
/// for `size_of::<T>() * capacity`, and resized by `try_resize`, which updates
/// capacity and block length from the same checked formula. Header writes from
/// those paths preserve the payload-fit invariant. `CageAllocation` is not
/// cloneable and its fields are private, so safe callers cannot supply a
/// reconstructed offset here.
#[inline]
fn read_owner_header<T: CompactValue>(
    state: &CageState,
    owner: &CageAllocation<T>,
) -> Result<AllocationHeader> {
    let offset = owner.raw_offset();
    // SAFETY: this helper is called only with the private offset of a live
    // allocator-issued owner; its constructors and resize path preserve bounds.
    unsafe { read_header(state, offset) }
}

/// Read and fully validate a typed header for an offset that does not carry
/// the private owner provenance proof.
#[inline]
fn read_typed_header<T>(state: &CageState, offset: u32) -> Result<AllocationHeader> {
    // SAFETY: callers uphold the offset's basic liveness contract or are in an
    // unsafe offset-resolution API; `read_header` checks cage and header bounds.
    let header = unsafe { read_header(state, offset) }?;
    validate_typed_header::<T>(state, offset, header)?;
    Ok(header)
}

#[inline]
fn validate_typed_header<T>(
    state: &CageState,
    offset: u32,
    header: AllocationHeader,
) -> Result<()> {
    let payload_bytes = size_of::<T>()
        .checked_mul(header.capacity as usize)
        .ok_or(Error::OffsetOverflow)?
        .max(1);
    let payload_start = (offset as usize)
        .checked_add(payload_bytes)
        .ok_or(Error::InvalidOffset)?;
    let block_end = (offset as usize)
        .checked_sub(size_of::<AllocationHeader>())
        .and_then(|n| n.checked_sub(header.prefix as usize))
        .and_then(|n| n.checked_add(header.block_len as usize))
        .ok_or(Error::InvalidOffset)?;
    if payload_start > block_end || block_end > state.capacity {
        return Err(Error::InvalidOffset);
    }
    Ok(())
}

fn allocate_block(
    state: &CageState,
    allocator: &mut Allocator,
    bytes: usize,
    alignment: usize,
) -> Result<(u32, u32, u32)> {
    let required_bytes = u32::try_from(bytes).map_err(|_| Error::OffsetOverflow)?;
    // A2's object build allocates many blocks before releasing any. Keep this
    // fast path inside the existing mutex and only use it when neither free
    // structure has entries. Telemetry builds retain the full accounting path.
    if !cfg!(feature = "allocator-telemetry")
        && allocator.free_head == 0
        && !allocator.has_size_class_cache
    {
        return allocate_from_cursor(state, allocator, required_bytes, alignment);
    }
    #[cfg(feature = "allocator-telemetry")]
    let free_list_search_phase = PhaseTimer::start(&ALLOCATOR_PHASE_TELEMETRY.free_list_search_ns);
    // With the cage base, starts, and header aligned to eight bytes, requests
    // up to that alignment have a position-independent block length. Probe
    // only that exact class; wider alignments still need to check every class.
    let candidate_class = if alignment <= 8 {
        let block_len = required_bytes
            .checked_add(size_of::<AllocationHeader>() as u32 + 7)
            .ok_or(Error::OffsetOverflow)?
            & !7;
        size_class_index(block_len)
    } else {
        None
    };
    let matching_class = (ENABLE_SIZE_CLASS_CACHE && allocator.has_size_class_cache)
        .then_some(candidate_class)
        .flatten();
    #[cfg(feature = "allocator-telemetry")]
    let class_metric_index = if let Some(index) = candidate_class {
        Some(index)
    } else {
        size_class_index(
            block_layout(
                state.base(),
                allocator.cursor,
                required_bytes as usize,
                alignment,
            )?
            .2,
        )
    };
    #[cfg(feature = "allocator-telemetry")]
    let minimum_block_len = required_bytes
        .checked_add(size_of::<AllocationHeader>() as u32 + 7)
        .map(|raw| raw & !7);
    let class_range = match (ENABLE_SIZE_CLASS_CACHE, matching_class) {
        (true, Some(index)) => index..index + 1,
        (true, None) if alignment > 8 && allocator.has_size_class_cache => 0..SIZE_CLASSES.len(),
        (false, _) | (true, None) => 0..0,
    };
    #[cfg(feature = "allocator-telemetry")]
    let class_range_is_empty = class_range.is_empty();
    #[cfg(feature = "allocator-telemetry")]
    let mut class_alignment_incompatible = [false; SIZE_CLASS_COUNT];
    for class_index in class_range {
        let class_size = SIZE_CLASSES[class_index];
        let mut current = allocator.size_class_heads[class_index];
        while current != 0 {
            #[cfg(feature = "allocator-telemetry")]
            {
                allocator.free_list_nodes_visited =
                    allocator.free_list_nodes_visited.saturating_add(1);
            }
            let node = unsafe { read_free_node(state, current)? };
            if node.len != class_size {
                return Err(Error::InvalidOffset);
            }
            let (data, prefix, required_len) =
                block_layout(state.base(), current, required_bytes as usize, alignment)?;
            #[cfg(feature = "allocator-telemetry")]
            if minimum_block_len == Some(node.len) && required_len > node.len {
                class_alignment_incompatible[class_index] = true;
            }
            if required_len == node.len {
                let live_bytes = allocator
                    .live_bytes
                    .checked_add(node.len)
                    .ok_or(Error::OffsetOverflow)?;
                allocator.size_class_heads[class_index] = node.next;
                allocator.size_class_counts[class_index] -= 1;
                allocator.has_size_class_cache =
                    allocator.size_class_counts.iter().any(|count| *count != 0);
                allocator.live_bytes = live_bytes;
                #[cfg(feature = "allocator-telemetry")]
                {
                    allocator.size_class_hits = allocator.size_class_hits.saturating_add(1);
                    allocator.global_class_hits[class_index] =
                        allocator.global_class_hits[class_index].saturating_add(1);
                }
                record_allocation_size(allocator, node.len);
                return Ok((data, prefix, node.len));
            }
            current = node.next;
        }
        #[cfg(feature = "allocator-telemetry")]
        {
            allocator.global_class_misses[class_index] =
                allocator.global_class_misses[class_index].saturating_add(1);
            if allocator.size_class_heads[class_index] == 0 {
                allocator.global_class_empty[class_index] =
                    allocator.global_class_empty[class_index].saturating_add(1);
            }
            if class_alignment_incompatible[class_index] {
                allocator.global_class_alignment_incompatible[class_index] =
                    allocator.global_class_alignment_incompatible[class_index].saturating_add(1);
            }
        }
    }
    #[cfg(feature = "allocator-telemetry")]
    {
        if ENABLE_SIZE_CLASS_CACHE {
            if let Some(class_index) = class_metric_index {
                allocator.size_class_misses = allocator.size_class_misses.saturating_add(1);
                if class_range_is_empty {
                    allocator.global_class_misses[class_index] =
                        allocator.global_class_misses[class_index].saturating_add(1);
                    if allocator.size_class_heads[class_index] == 0 {
                        allocator.global_class_empty[class_index] =
                            allocator.global_class_empty[class_index].saturating_add(1);
                    }
                }
            } else {
                allocator.requested_size_no_class =
                    allocator.requested_size_no_class.saturating_add(1);
            }
        } else if class_metric_index.is_none() {
            allocator.requested_size_no_class = allocator.requested_size_no_class.saturating_add(1);
        }
    }

    let mut previous = 0;
    let mut current = allocator.free_head;
    while current != 0 {
        #[cfg(feature = "allocator-telemetry")]
        {
            allocator.free_list_nodes_visited = allocator.free_list_nodes_visited.saturating_add(1);
        }
        let node = unsafe { read_free_node(state, current)? };
        let (data, prefix, required_len) =
            block_layout(state.base(), current, required_bytes as usize, alignment)?;
        if required_len <= node.len {
            let remainder = node.len - required_len;
            let allocated_len = if remainder >= FREE_NODE_BYTES {
                required_len
            } else {
                node.len
            };
            let live_bytes = allocator
                .live_bytes
                .checked_add(allocated_len)
                .ok_or(Error::OffsetOverflow)?;
            let updated_link = if remainder >= FREE_NODE_BYTES {
                let remainder_start = current
                    .checked_add(required_len)
                    .ok_or(Error::OffsetOverflow)?;
                unsafe {
                    write_free_node(
                        state,
                        remainder_start,
                        FreeNode {
                            next: node.next,
                            len: remainder,
                        },
                    )
                };
                remainder_start
            } else {
                node.next
            };
            if previous == 0 {
                allocator.free_head = updated_link;
            } else {
                let mut previous_node = unsafe { read_free_node(state, previous)? };
                previous_node.next = updated_link;
                unsafe { write_free_node(state, previous, previous_node) };
            }
            allocator.live_bytes = live_bytes;
            #[cfg(feature = "allocator-telemetry")]
            {
                allocator.general_list_fallbacks =
                    allocator.general_list_fallbacks.saturating_add(1);
            }
            record_allocation_size(allocator, allocated_len);
            return Ok((data, prefix, allocated_len));
        }
        previous = current;
        current = node.next;
    }

    #[cfg(feature = "allocator-telemetry")]
    drop(free_list_search_phase);
    allocate_from_cursor(state, allocator, required_bytes, alignment)
}

fn allocate_from_cursor(
    state: &CageState,
    allocator: &mut Allocator,
    required_bytes: u32,
    alignment: usize,
) -> Result<(u32, u32, u32)> {
    #[cfg(feature = "allocator-telemetry")]
    let _bump_allocation_phase = PhaseTimer::start(&ALLOCATOR_PHASE_TELEMETRY.bump_allocation_ns);
    let start = allocator.cursor;
    let (data, prefix, len) =
        block_layout(state.base(), start, required_bytes as usize, alignment)?;
    let end = start.checked_add(len).ok_or(Error::OffsetOverflow)?;
    if end > state.capacity as u32 || end as u64 > MAX_CAGE_BYTES {
        return Err(Error::AllocationExhausted);
    }
    let live_bytes = allocator
        .live_bytes
        .checked_add(len)
        .ok_or(Error::OffsetOverflow)?;
    allocator.cursor = end;
    allocator.live_bytes = live_bytes;
    #[cfg(feature = "allocator-telemetry")]
    {
        allocator.cursor_fallbacks = allocator.cursor_fallbacks.saturating_add(1);
    }
    record_allocation_size(allocator, len);
    Ok((data, prefix, len))
}

#[cfg(feature = "allocator-telemetry")]
fn record_allocation_size(allocator: &mut Allocator, block_len: u32) {
    let bucket = ((block_len as usize) / 8).min(BLOCK_SIZE_BUCKETS - 1);
    allocator.allocation_size_histogram[bucket] =
        allocator.allocation_size_histogram[bucket].saturating_add(1);
}

#[cfg(not(feature = "allocator-telemetry"))]
#[inline]
fn record_allocation_size(_: &mut Allocator, _: u32) {}

fn size_class_index(block_len: u32) -> Option<usize> {
    match block_len {
        32 => Some(0),
        40 => Some(1),
        112 => Some(2),
        528 => Some(3),
        _ => None,
    }
}

unsafe fn read_free_node(state: &CageState, offset: u32) -> Result<FreeNode> {
    let end = offset
        .checked_add(FREE_NODE_BYTES)
        .ok_or(Error::InvalidOffset)?;
    if offset == 0 || end as usize > state.capacity {
        return Err(Error::InvalidOffset);
    }
    // SAFETY: the allocator only links free blocks inside its cage and the
    // node bytes are initialized whenever a block is inserted into the list.
    Ok(unsafe { ptr_from_offset::<FreeNode>(state, offset).read_unaligned() })
}

unsafe fn write_free_node(state: &CageState, offset: u32, node: FreeNode) {
    // SAFETY: callers prove this range belongs to a free block inside the cage.
    unsafe { ptr_from_offset::<FreeNode>(state, offset).write_unaligned(node) };
}

fn free_node_at(
    state: &CageState,
    allocator: &Allocator,
    offset: u32,
) -> Result<Option<(u32, u32)>> {
    let mut current = allocator.free_head;
    while current != 0 {
        let node = unsafe { read_free_node(state, current)? };
        if current == offset {
            return Ok(Some((node.len, node.next)));
        }
        if current > offset {
            break;
        }
        current = node.next;
    }
    Ok(None)
}

fn consume_free_prefix(
    state: &CageState,
    allocator: &mut Allocator,
    start: u32,
    consumed: u32,
) -> Result<()> {
    let mut previous = 0;
    let mut current = allocator.free_head;
    while current != 0 && current < start {
        previous = current;
        current = unsafe { read_free_node(state, current)? }.next;
    }
    if current != start {
        return Err(Error::InvalidOffset);
    }
    let node = unsafe { read_free_node(state, current)? };
    if consumed > node.len {
        return Err(Error::InvalidOffset);
    }
    let remainder = node.len - consumed;
    let updated_link = if remainder >= FREE_NODE_BYTES {
        let remainder_start = start.checked_add(consumed).ok_or(Error::OffsetOverflow)?;
        unsafe {
            write_free_node(
                state,
                remainder_start,
                FreeNode {
                    next: node.next,
                    len: remainder,
                },
            )
        };
        remainder_start
    } else if remainder == 0 {
        node.next
    } else {
        return Err(Error::InvalidOffset);
    };
    if previous == 0 {
        allocator.free_head = updated_link;
    } else {
        let mut previous_node = unsafe { read_free_node(state, previous)? };
        previous_node.next = updated_link;
        unsafe { write_free_node(state, previous, previous_node) };
    }
    Ok(())
}

fn insert_free(state: &CageState, allocator: &mut Allocator, start: u32, len: u32) -> Result<()> {
    if len == 0 || len < FREE_NODE_BYTES || len % 8 != 0 {
        return Err(Error::InvalidOffset);
    }
    let end = start.checked_add(len).ok_or(Error::OffsetOverflow)?;
    if start < INITIAL_CURSOR || end > allocator.cursor {
        return Err(Error::InvalidOffset);
    }

    let mut previous_previous = 0_u32;
    let mut previous_previous_node = None;
    let mut previous = 0_u32;
    let mut previous_node = None;
    let mut next = allocator.free_head;
    let mut previous_end = 0_u32;
    let mut steps = 0_u32;
    let max_steps = allocator.cursor / FREE_NODE_BYTES + 1;
    while next != 0 && next < start {
        if next < INITIAL_CURSOR || (previous_end != 0 && next <= previous_end) {
            return Err(Error::InvalidOffset);
        }
        let node = unsafe { read_free_node(state, next)? };
        #[cfg(feature = "allocator-telemetry")]
        {
            allocator.free_list_nodes_visited = allocator.free_list_nodes_visited.saturating_add(1);
        }
        let node_end = next.checked_add(node.len).ok_or(Error::InvalidOffset)?;
        if node.len < FREE_NODE_BYTES
            || node.len % 8 != 0
            || node_end >= allocator.cursor
            || node_end > start
            || (node.next != 0 && node_end >= node.next)
        {
            return Err(Error::InvalidOffset);
        }
        previous_previous = previous;
        previous_previous_node = previous_node;
        previous = next;
        previous_node = Some(node);
        previous_end = node_end;
        next = node.next;
        steps += 1;
        if steps > max_steps {
            return Err(Error::InvalidOffset);
        }
    }
    let next_node = if next == 0 {
        None
    } else {
        let node = unsafe { read_free_node(state, next)? };
        #[cfg(feature = "allocator-telemetry")]
        {
            allocator.free_list_nodes_visited = allocator.free_list_nodes_visited.saturating_add(1);
        }
        let node_end = next.checked_add(node.len).ok_or(Error::InvalidOffset)?;
        if next < INITIAL_CURSOR
            || (previous_end != 0 && next <= previous_end)
            || node.len < FREE_NODE_BYTES
            || node.len % 8 != 0
            || node_end >= allocator.cursor
            || end > next
            || (node.next != 0 && node_end >= node.next)
        {
            return Err(Error::InvalidOffset);
        }
        Some(node)
    };
    if previous_node.is_none() && previous != 0 {
        return Err(Error::InvalidOffset);
    }

    let previous_is_adjacent =
        previous_node.and_then(|node: FreeNode| previous.checked_add(node.len)) == Some(start);
    let merged_start = if previous_is_adjacent {
        previous
    } else {
        start
    };
    let mut merged_len = if previous_is_adjacent {
        previous_node
            .expect("adjacent previous free block")
            .len
            .checked_add(len)
            .ok_or(Error::OffsetOverflow)?
    } else {
        len
    };
    let mut merged_next = if previous_is_adjacent {
        previous_node.expect("adjacent previous free block").next
    } else {
        next
    };
    if merged_start.checked_add(merged_len) == Some(next) {
        let node = next_node.expect("adjacent next free block");
        merged_len = merged_len
            .checked_add(node.len)
            .ok_or(Error::OffsetOverflow)?;
        merged_next = node.next;
    }
    let merged_end = merged_start
        .checked_add(merged_len)
        .ok_or(Error::OffsetOverflow)?;
    if merged_end > allocator.cursor || (merged_end == allocator.cursor && merged_next != 0) {
        return Err(Error::InvalidOffset);
    }

    if merged_end == allocator.cursor {
        if previous_is_adjacent {
            if previous_previous == 0 {
                allocator.free_head = merged_next;
            } else {
                let mut node = previous_previous_node.expect("previous free-list predecessor");
                node.next = merged_next;
                unsafe { write_free_node(state, previous_previous, node) };
            }
        } else if previous == 0 {
            allocator.free_head = merged_next;
        } else {
            let mut node = previous_node.expect("previous free block");
            node.next = merged_next;
            unsafe { write_free_node(state, previous, node) };
        }
        allocator.cursor = merged_start;
    } else if previous_is_adjacent {
        let mut node = previous_node.expect("adjacent previous free block");
        node.next = merged_next;
        node.len = merged_len;
        unsafe { write_free_node(state, previous, node) };
    } else {
        if previous == 0 {
            allocator.free_head = start;
        } else {
            let mut node = previous_node.expect("previous free block");
            node.next = start;
            unsafe { write_free_node(state, previous, node) };
        }
        unsafe {
            write_free_node(
                state,
                start,
                FreeNode {
                    next: merged_next,
                    len: merged_len,
                },
            )
        };
    }
    Ok(())
}

fn release(offset: u32) {
    let Ok(state) = state() else {
        return;
    };
    let Some(extent) = release_extent(state, offset) else {
        // The owner is private and allocator-issued. An invalid release header
        // is therefore allocator corruption; fail closed instead of silently
        // pretending the range was reclaimed.
        state.allocator_faulted.store(true, Ordering::Release);
        return;
    };
    let collected = ACTIVE_RELEASE_COLLECTOR.with(|active| {
        let collector = active.get();
        if collector.is_null() {
            false
        } else {
            // SAFETY: the thread-local pointer is installed only while its
            // stack-local collector is alive on this same thread.
            unsafe { (*collector).push(extent) };
            true
        }
    });
    if !collected && !cache_released_extent(state, extent) {
        let mut one = [extent];
        let _ = release_many(&mut one);
    }
}

fn release_extent(state: &CageState, offset: u32) -> Option<ReleaseExtent> {
    // SAFETY: the owner releases this allocator-issued offset exactly once.
    let header = unsafe { read_header(state, offset) }.ok()?;
    let start = offset
        .checked_sub(size_of::<AllocationHeader>() as u32)?
        .checked_sub(header.prefix)?;
    Some(ReleaseExtent {
        start,
        len: header.block_len,
    })
}

fn release_many(extents: &mut [ReleaseExtent]) -> Result<()> {
    if extents.is_empty() {
        return Ok(());
    }
    if extents.len() > RELEASE_BATCH_CAPACITY {
        let state = state()?;
        for extent in extents.iter().copied() {
            enqueue_pending_release(state, extent);
        }
        return Ok(());
    }
    let state = state()?;
    let mut transaction = match lock(state) {
        Ok(transaction) => transaction,
        Err(_) => {
            for extent in extents.iter().copied() {
                enqueue_pending_release(state, extent);
            }
            return Ok(());
        }
    };
    match transaction.release_many(extents) {
        Ok(()) => Ok(()),
        Err(error) => {
            // Validation errors are fail-closed: preserve the descriptors in
            // the in-cage retry list and prevent further allocator mutation.
            state.allocator_faulted.store(true, Ordering::Release);
            for extent in extents.iter().copied() {
                enqueue_pending_release(state, extent);
            }
            let _ = error;
            Ok(())
        }
    }
}

fn next_merge_extent(
    state: &CageState,
    general_current: &mut u32,
    released: &[ReleaseExtent],
    released_index: &mut usize,
) -> Result<Option<(ReleaseExtent, bool)>> {
    if *general_current == 0 {
        let Some(extent) = released.get(*released_index).copied() else {
            return Ok(None);
        };
        *released_index += 1;
        return Ok(Some((extent, false)));
    }
    let node = unsafe { read_free_node(state, *general_current)? };
    let general_extent = ReleaseExtent {
        start: *general_current,
        len: node.len,
    };
    if let Some(extent) = released.get(*released_index).copied() {
        if extent.start < general_extent.start {
            *released_index += 1;
            return Ok(Some((extent, false)));
        }
    }
    *general_current = node.next;
    Ok(Some((general_extent, true)))
}

fn release_many_locked(
    state: &CageState,
    allocator: &mut Allocator,
    extents: &mut [ReleaseExtent],
    cache_small_classes: bool,
) -> Result<()> {
    if extents.len() > RELEASE_BATCH_CAPACITY {
        return Err(Error::InvalidOffset);
    }
    #[cfg(feature = "allocator-telemetry")]
    let released_exact_size_count = extents
        .iter()
        .filter(|extent| size_class_index(extent.len).is_some())
        .count() as u64;

    // Nested owners are often allocated as one tail run and then dropped
    // together. If no earlier holes exist, validate that run and contract it
    // directly instead of rebuilding the general free list for each chunk.
    if !extents.is_empty()
        && allocator.free_head == 0
        && allocator.size_class_counts.iter().all(|count| *count == 0)
    {
        extents.sort_unstable_by_key(|extent| extent.start);
        let mut released_bytes = 0_u32;
        let mut previous_end = 0_u32;
        let mut is_contiguous = true;
        for (index, extent) in extents.iter().copied().enumerate() {
            validate_free_extent(allocator, extent)?;
            if index != 0 && extent.start != previous_end {
                is_contiguous = false;
                break;
            }
            previous_end = extent
                .start
                .checked_add(extent.len)
                .ok_or(Error::OffsetOverflow)?;
            released_bytes = released_bytes
                .checked_add(extent.len)
                .ok_or(Error::OffsetOverflow)?;
        }
        if is_contiguous
            && previous_end == allocator.cursor
            && released_bytes == allocator.cursor.saturating_sub(extents[0].start)
        {
            let new_live_bytes = allocator
                .live_bytes
                .checked_sub(released_bytes)
                .ok_or(Error::InvalidOffset)?;
            if new_live_bytes != extents[0].start.saturating_sub(INITIAL_CURSOR) {
                return Err(Error::InitializationError);
            }
            allocator.live_bytes = new_live_bytes;
            allocator.cursor = extents[0].start;
            #[cfg(feature = "allocator-telemetry")]
            {
                allocator.released_exact_size_extents = allocator
                    .released_exact_size_extents
                    .saturating_add(released_exact_size_count);
                for (index, extent) in extents.iter().copied().enumerate() {
                    if size_class_index(extent.len).is_some()
                        && ((index > 0
                            && extents[index - 1].start.checked_add(extents[index - 1].len)
                                == Some(extent.start))
                            || (index + 1 < extents.len()
                                && extent.start.checked_add(extent.len)
                                    == Some(extents[index + 1].start)))
                    {
                        allocator.exact_size_extents_coalesced_before_cache = allocator
                            .exact_size_extents_coalesced_before_cache
                            .saturating_add(1);
                    }
                }
                allocator.release_batches = allocator.release_batches.saturating_add(1);
                allocator.released_extents = allocator
                    .released_extents
                    .saturating_add(extents.len() as u64);
                allocator.max_release_batch = allocator.max_release_batch.max(extents.len() as u32);
            }
            return Ok(());
        }
    }

    if extents.len() == 1 && allocator.size_class_counts.iter().all(|count| *count == 0) {
        let extent = extents[0];
        validate_free_extent(allocator, extent)?;
        let new_live_bytes = allocator
            .live_bytes
            .checked_sub(extent.len)
            .ok_or(Error::InvalidOffset)?;
        // With no cached ranges, the ordered list is the complete free
        // structure. Keep its established insertion path for coalescing and
        // cursor contraction instead of building batch scratch. This also
        // applies when global size-class caching is disabled.
        insert_free(state, allocator, extent.start, extent.len)?;
        allocator.live_bytes = new_live_bytes;
        #[cfg(feature = "allocator-telemetry")]
        {
            allocator.released_exact_size_extents = allocator
                .released_exact_size_extents
                .saturating_add(released_exact_size_count);
            allocator.release_batches = allocator.release_batches.saturating_add(1);
            allocator.released_extents = allocator.released_extents.saturating_add(1);
            allocator.max_release_batch = allocator.max_release_batch.max(1);
        }
        return Ok(());
    }

    let mut merged = [ReleaseExtent::default(); MAX_MERGE_EXTENTS];
    let mut merged_len = 0;
    let mut released_bytes = 0_u32;
    for extent in extents.iter().copied() {
        validate_free_extent(allocator, extent)?;
        released_bytes = released_bytes
            .checked_add(extent.len)
            .ok_or(Error::OffsetOverflow)?;
        merged[merged_len] = extent;
        merged_len += 1;
    }

    let mut class_bytes = 0_u32;
    #[cfg(feature = "allocator-telemetry")]
    let mut visited = 0_u64;
    for (class_index, class_size) in SIZE_CLASSES.iter().copied().enumerate() {
        let mut current = allocator.size_class_heads[class_index];
        let mut count = 0_u32;
        while current != 0 {
            if merged_len == MAX_MERGE_EXTENTS || count >= SIZE_CLASS_CACHE_CAPACITY {
                return Err(Error::InvalidOffset);
            }
            let node = unsafe { read_free_node(state, current)? };
            #[cfg(feature = "allocator-telemetry")]
            {
                visited += 1;
            }
            if node.len != class_size {
                return Err(Error::InvalidOffset);
            }
            let extent = ReleaseExtent {
                start: current,
                len: node.len,
            };
            validate_free_extent(allocator, extent)?;
            merged[merged_len] = extent;
            merged_len += 1;
            count += 1;
            class_bytes = class_bytes
                .checked_add(node.len)
                .ok_or(Error::OffsetOverflow)?;
            current = node.next;
        }
        if count != allocator.size_class_counts[class_index] {
            return Err(Error::InvalidOffset);
        }
    }

    merged[..merged_len].sort_unstable_by_key(|extent| extent.start);
    let mut previous_end = 0_u32;
    let mut release_bytes_sorted = 0_u32;
    for (index, extent) in merged[..merged_len].iter().copied().enumerate() {
        validate_free_extent(allocator, extent)?;
        if index != 0 && extent.start < previous_end {
            return Err(Error::InvalidOffset);
        }
        previous_end = extent
            .start
            .checked_add(extent.len)
            .ok_or(Error::OffsetOverflow)?;
        release_bytes_sorted = release_bytes_sorted
            .checked_add(extent.len)
            .ok_or(Error::OffsetOverflow)?;
    }
    if release_bytes_sorted
        != released_bytes
            .checked_add(class_bytes)
            .ok_or(Error::OffsetOverflow)?
    {
        return Err(Error::InvalidOffset);
    }

    let new_live_bytes = allocator
        .live_bytes
        .checked_sub(released_bytes)
        .ok_or(Error::InvalidOffset)?;
    let expected_prefix = allocator
        .cursor
        .checked_sub(INITIAL_CURSOR)
        .ok_or(Error::InvalidOffset)?;
    let mut check_general = allocator.free_head;
    let mut check_released = 0;
    let mut check_end = 0_u32;
    let mut general_bytes = 0_u32;
    let mut steps = 0_u32;
    let max_steps = allocator.cursor / FREE_NODE_BYTES + 1;
    while check_general != 0 || check_released < merged_len {
        let (next, from_general) = next_merge_extent(
            state,
            &mut check_general,
            &merged[..merged_len],
            &mut check_released,
        )?
        .ok_or(Error::InvalidOffset)?;
        if from_general {
            general_bytes = general_bytes
                .checked_add(next.len)
                .ok_or(Error::OffsetOverflow)?;
            #[cfg(feature = "allocator-telemetry")]
            {
                visited += 1;
            }
        }
        if next.start < check_end {
            return Err(Error::InvalidOffset);
        }
        check_end = next
            .start
            .checked_add(next.len)
            .ok_or(Error::OffsetOverflow)?;
        if check_end > allocator.cursor {
            return Err(Error::InvalidOffset);
        }
        steps += 1;
        if steps > max_steps {
            return Err(Error::InvalidOffset);
        }
    }
    if general_bytes
        .checked_add(class_bytes)
        .and_then(|bytes| bytes.checked_add(allocator.live_bytes))
        .ok_or(Error::OffsetOverflow)?
        != expected_prefix
    {
        return Err(Error::InitializationError);
    }

    allocator.size_class_heads = [0; SIZE_CLASSES.len()];
    allocator.size_class_counts = [0; SIZE_CLASSES.len()];
    allocator.has_size_class_cache = false;
    let mut general_current = allocator.free_head;
    let mut released_index = 0;
    let mut new_general_head = 0_u32;
    let mut general_tail = 0_u32;
    let mut last_start = 0_u32;
    let mut last_end = 0_u32;
    let mut last_general_previous = 0_u32;
    let mut last_class = None;
    #[cfg(feature = "allocator-telemetry")]
    let mut cached_exact_size_count = 0_u64;
    #[cfg(feature = "allocator-telemetry")]
    let mut coalesced_exact_size_count = 0_u64;
    while general_current != 0 || released_index < merged_len {
        let (mut run, _from_general) = next_merge_extent(
            state,
            &mut general_current,
            &merged[..merged_len],
            &mut released_index,
        )
        .expect("free ranges were validated before allocator mutation")
        .expect("validated merge input must contain the selected extent");
        #[cfg(feature = "allocator-telemetry")]
        if _from_general {
            visited += 1;
        }
        let run_start = run.start;
        let mut run_end = run
            .start
            .checked_add(run.len)
            .expect("validated free extent end fits the cage");
        loop {
            let mut general_probe = general_current;
            let mut released_probe = released_index;
            let Some((next, _from_general)) = next_merge_extent(
                state,
                &mut general_probe,
                &merged[..merged_len],
                &mut released_probe,
            )
            .expect("free ranges were validated before allocator mutation") else {
                break;
            };
            #[cfg(feature = "allocator-telemetry")]
            if _from_general {
                visited += 1;
            }
            if next.start != run_end {
                break;
            }
            // Commit the peek only after adjacency has been confirmed.
            let (_, _from_general) = next_merge_extent(
                state,
                &mut general_current,
                &merged[..merged_len],
                &mut released_index,
            )
            .expect("free ranges were validated before allocator mutation")
            .expect("adjacent merge input must still contain its peeked extent");
            #[cfg(feature = "allocator-telemetry")]
            if _from_general {
                visited += 1;
            }
            run.len = run
                .len
                .checked_add(next.len)
                .expect("validated free range total fits the cage");
            run_end = run_end
                .checked_add(next.len)
                .expect("validated adjacent range end fits the cage");
        }

        let class_index = cache_small_classes
            .then(|| size_class_index(run.len))
            .flatten()
            .filter(|index| allocator.size_class_counts[*index] < SIZE_CLASS_CACHE_CAPACITY);
        #[cfg(feature = "allocator-telemetry")]
        {
            for released in extents.iter().copied() {
                if size_class_index(released.len).is_some()
                    && released.start >= run_start
                    && released
                        .start
                        .checked_add(released.len)
                        .is_some_and(|end| end <= run_end)
                    && released.len < run.len
                {
                    coalesced_exact_size_count = coalesced_exact_size_count.saturating_add(1);
                }
            }
            if class_index.is_some()
                && extents
                    .iter()
                    .any(|released| released.start == run_start && released.len == run.len)
            {
                cached_exact_size_count = cached_exact_size_count.saturating_add(1);
            }
        }
        if let Some(index) = class_index {
            unsafe {
                write_free_node(
                    state,
                    run_start,
                    FreeNode {
                        next: allocator.size_class_heads[index],
                        len: run.len,
                    },
                )
            };
            allocator.size_class_heads[index] = run_start;
            allocator.size_class_counts[index] += 1;
            allocator.has_size_class_cache = true;
            last_class = Some(index);
            last_general_previous = general_tail;
        } else {
            unsafe {
                write_free_node(
                    state,
                    run_start,
                    FreeNode {
                        next: 0,
                        len: run.len,
                    },
                )
            };
            if general_tail == 0 {
                new_general_head = run_start;
            } else {
                let mut tail_node = unsafe { read_free_node(state, general_tail) }
                    .expect("new general free-list tail remains valid");
                tail_node.next = run_start;
                unsafe { write_free_node(state, general_tail, tail_node) };
            }
            last_general_previous = general_tail;
            general_tail = run_start;
            last_class = None;
        }
        last_start = run_start;
        last_end = run_end;
    }

    allocator.free_head = new_general_head;
    allocator.live_bytes = new_live_bytes;
    if last_end == allocator.cursor {
        if let Some(class_index) = last_class {
            assert_eq!(allocator.size_class_heads[class_index], last_start);
            let node = unsafe { read_free_node(state, last_start) }
                .expect("last cached free range remains valid");
            allocator.size_class_heads[class_index] = node.next;
            allocator.size_class_counts[class_index] -= 1;
            allocator.has_size_class_cache =
                allocator.size_class_counts.iter().any(|count| *count != 0);
        } else if last_general_previous == 0 {
            allocator.free_head = 0;
        } else {
            let mut previous = unsafe { read_free_node(state, last_general_previous) }
                .expect("last general free-list predecessor remains valid");
            previous.next = 0;
            unsafe { write_free_node(state, last_general_previous, previous) };
        }
        allocator.cursor = last_start;
    }

    #[cfg(feature = "allocator-telemetry")]
    {
        allocator.released_exact_size_extents = allocator
            .released_exact_size_extents
            .saturating_add(released_exact_size_count);
        allocator.exact_size_extents_cached = allocator
            .exact_size_extents_cached
            .saturating_add(cached_exact_size_count);
        allocator.exact_size_extents_coalesced_before_cache = allocator
            .exact_size_extents_coalesced_before_cache
            .saturating_add(coalesced_exact_size_count);
        allocator.free_list_nodes_visited =
            allocator.free_list_nodes_visited.saturating_add(visited);
        if !extents.is_empty() {
            allocator.release_batches = allocator.release_batches.saturating_add(1);
            allocator.released_extents = allocator
                .released_extents
                .saturating_add(extents.len() as u64);
            allocator.max_release_batch = allocator.max_release_batch.max(extents.len() as u32);
        }
    }
    Ok(())
}

fn merge_free_ranges_locked(state: &CageState, allocator: &mut Allocator) -> Result<()> {
    let mut no_releases = [];
    release_many_locked(state, allocator, &mut no_releases, false)
}

fn validate_free_extent(allocator: &Allocator, extent: ReleaseExtent) -> Result<()> {
    if extent.len < FREE_NODE_BYTES
        || extent.len % 8 != 0
        || extent.start < INITIAL_CURSOR
        || extent
            .start
            .checked_add(extent.len)
            .map_or(true, |end| end > allocator.cursor)
    {
        return Err(Error::InvalidOffset);
    }
    Ok(())
}

fn validate_allocator(state: &CageState, allocator: &Allocator) -> Result<()> {
    if allocator.cursor as usize > state.capacity || allocator.cursor < INITIAL_CURSOR {
        return Err(Error::InvalidOffset);
    }

    let mut cached = [ReleaseExtent::default(); MAX_SIZE_CLASS_EXTENTS];
    let mut cached_len = 0;
    let mut cached_bytes = 0_u32;
    for (class_index, class_size) in SIZE_CLASSES.iter().copied().enumerate() {
        let mut current = allocator.size_class_heads[class_index];
        let mut count = 0_u32;
        while current != 0 {
            if count >= SIZE_CLASS_CACHE_CAPACITY || cached_len == cached.len() {
                return Err(Error::InvalidOffset);
            }
            let node = unsafe { read_free_node(state, current)? };
            if node.len != class_size {
                return Err(Error::InvalidOffset);
            }
            let extent = ReleaseExtent {
                start: current,
                len: node.len,
            };
            validate_free_extent(allocator, extent)?;
            if extent
                .start
                .checked_add(extent.len)
                .map_or(true, |end| end >= allocator.cursor)
            {
                return Err(Error::InvalidOffset);
            }
            cached_bytes = cached_bytes
                .checked_add(extent.len)
                .ok_or(Error::OffsetOverflow)?;
            cached[cached_len] = extent;
            cached_len += 1;
            count += 1;
            current = node.next;
        }
        if count != allocator.size_class_counts[class_index] {
            return Err(Error::InvalidOffset);
        }
    }
    if allocator.has_size_class_cache != (cached_len != 0) {
        return Err(Error::InvalidOffset);
    }
    cached[..cached_len].sort_unstable_by_key(|extent| extent.start);

    let mut general_bytes = 0_u32;
    let mut general_count = 0_u32;
    let mut previous_end = 0_u32;
    let mut current = allocator.free_head;
    while current != 0 {
        if current < INITIAL_CURSOR || (previous_end != 0 && current <= previous_end) {
            return Err(Error::InvalidOffset);
        }
        let node = unsafe { read_free_node(state, current)? };
        let extent = ReleaseExtent {
            start: current,
            len: node.len,
        };
        validate_free_extent(allocator, extent)?;
        let end = current.checked_add(node.len).ok_or(Error::InvalidOffset)?;
        if end >= allocator.cursor || (node.next != 0 && end >= node.next) {
            return Err(Error::InvalidOffset);
        }
        general_bytes = general_bytes
            .checked_add(node.len)
            .ok_or(Error::OffsetOverflow)?;
        general_count = general_count.checked_add(1).ok_or(Error::OffsetOverflow)?;
        previous_end = end;
        current = node.next;
    }

    let mut general_current = allocator.free_head;
    let mut cached_index = 0;
    let mut merged_end = 0_u32;
    let mut total_free_bytes = 0_u32;
    let mut merged_count = 0_u32;
    while general_current != 0 || cached_index < cached_len {
        let (extent, _) = next_merge_extent(
            state,
            &mut general_current,
            &cached[..cached_len],
            &mut cached_index,
        )?
        .ok_or(Error::InvalidOffset)?;
        validate_free_extent(allocator, extent)?;
        let end = extent
            .start
            .checked_add(extent.len)
            .ok_or(Error::OffsetOverflow)?;
        if extent.start <= merged_end && merged_count != 0 {
            return Err(Error::InvalidOffset);
        }
        if end >= allocator.cursor {
            return Err(Error::InvalidOffset);
        }
        merged_end = end;
        merged_count = merged_count.checked_add(1).ok_or(Error::OffsetOverflow)?;
        total_free_bytes = total_free_bytes
            .checked_add(extent.len)
            .ok_or(Error::OffsetOverflow)?;
    }
    if merged_count != general_count.saturating_add(cached_len as u32) {
        return Err(Error::InvalidOffset);
    }
    if total_free_bytes
        != general_bytes
            .checked_add(cached_bytes)
            .ok_or(Error::OffsetOverflow)?
    {
        return Err(Error::InitializationError);
    }
    if total_free_bytes
        .checked_add(allocator.live_bytes)
        .ok_or(Error::OffsetOverflow)?
        != allocator.cursor - INITIAL_CURSOR
    {
        return Err(Error::InitializationError);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::sync::Arc;

    #[repr(align(64))]
    struct OverAlignedValue {
        _bytes: [u8; 24],
    }

    // SAFETY: the test-only value contains plain bytes and has no address-sensitive state.
    unsafe impl CompactValue for OverAlignedValue {}

    fn assert_allocator_issued_header<T: CompactValue>(
        state: &CageState,
        allocator: &mut Allocator,
        capacity: usize,
    ) -> (u32, AllocationHeader) {
        let capacity_u32 = u32::try_from(capacity).unwrap();
        let payload_bytes = size_of::<T>().checked_mul(capacity).unwrap().max(1);
        let (offset, prefix, block_len) =
            allocate_block(state, allocator, payload_bytes, align_of::<T>().max(4)).unwrap();
        let header = AllocationHeader {
            block_len,
            prefix,
            capacity: capacity_u32,
            initialized: 0,
        };
        initialize_allocation_header(state, offset, header);
        // SAFETY: the test owner uses the valid offset and matching header
        // emitted by `allocate_block` and `initialize_allocation_header` above.
        let owner = core::mem::ManuallyDrop::new(CageAllocation::<T> {
            offset: NonZeroOffset(core::num::NonZeroU32::new(offset).unwrap()),
            marker: PhantomData,
        });
        let owner_header = read_owner_header::<T>(state, &*owner).unwrap();
        let typed_header = read_typed_header::<T>(state, offset).unwrap();
        for observed in [owner_header, typed_header] {
            assert_eq!(observed.block_len, header.block_len);
            assert_eq!(observed.prefix, header.prefix);
            assert_eq!(observed.capacity, header.capacity);
            assert_eq!(observed.initialized, header.initialized);
        }
        (offset, header)
    }

    fn local_allocate(state: &CageState, allocator: &mut Allocator, bytes: usize) -> ReleaseExtent {
        let (data, prefix, len) = allocate_block(state, allocator, bytes, 8).unwrap();
        ReleaseExtent {
            start: data - size_of::<AllocationHeader>() as u32 - prefix,
            len,
        }
    }

    fn local_release(state: &CageState, allocator: &mut Allocator, extents: &mut [ReleaseExtent]) {
        release_many_locked(state, allocator, extents, ENABLE_SIZE_CLASS_CACHE).unwrap();
    }

    fn local_flush(state: &CageState, collector: &mut ReleaseCollector) {
        let mut allocator = lock(state).unwrap();
        collector.flush_with(|extents| {
            release_many_locked(state, &mut allocator, extents, ENABLE_SIZE_CLASS_CACHE)
        });
    }

    fn allocator_free_extents(state: &CageState, allocator: &Allocator) -> Vec<ReleaseExtent> {
        let mut extents = Vec::new();
        let mut current = allocator.free_head;
        while current != 0 {
            let node = unsafe { read_free_node(state, current).unwrap() };
            extents.push(ReleaseExtent {
                start: current,
                len: node.len,
            });
            current = node.next;
        }
        for class_head in allocator.size_class_heads {
            let mut current = class_head;
            while current != 0 {
                let node = unsafe { read_free_node(state, current).unwrap() };
                extents.push(ReleaseExtent {
                    start: current,
                    len: node.len,
                });
                current = node.next;
            }
        }
        extents
    }

    #[test]
    fn allocator_issued_headers_satisfy_typed_payload_bounds() {
        let state = local_state(16 * 1024);
        let mut allocator = lock(&state).unwrap();

        for capacity in [0, 1, 4, 16] {
            assert_allocator_issued_header::<u8>(&state, &mut allocator, capacity);
            assert_allocator_issued_header::<u64>(&state, &mut allocator, capacity);
            assert_allocator_issued_header::<OverAlignedValue>(&state, &mut allocator, capacity);
        }
    }

    #[test]
    fn resize_capacity_formula_preserves_typed_payload_bounds() {
        let state = local_state(16 * 1024);
        let mut allocator = lock(&state).unwrap();
        let (offset, original) =
            assert_allocator_issued_header::<OverAlignedValue>(&state, &mut allocator, 4);
        drop(allocator);

        // `try_resize` retains the prefix, recomputes block length from the
        // requested capacity, and only commits growth after reserving the
        // adjacent bytes. Exercise both shrink and grow candidate headers.
        for capacity in [0, 1, 2, 8, 32] {
            let payload_bytes = size_of::<OverAlignedValue>()
                .checked_mul(capacity)
                .unwrap()
                .max(1);
            let block_len = u32::try_from(
                checked_align_up(
                    (original.prefix as usize) + size_of::<AllocationHeader>() + payload_bytes,
                    8,
                )
                .unwrap(),
            )
            .unwrap();
            let resized = AllocationHeader {
                block_len,
                prefix: original.prefix,
                capacity: capacity as u32,
                initialized: 0,
            };
            assert!(validate_typed_header::<OverAlignedValue>(&state, offset, resized).is_ok());
        }
    }

    #[test]
    fn typed_raw_offset_reader_rejects_payload_beyond_block() {
        let state = local_state(128);
        let offset = 32;
        let malformed_for_u64 = AllocationHeader {
            block_len: size_of::<AllocationHeader>() as u32,
            prefix: 0,
            capacity: 2,
            initialized: 0,
        };
        // SAFETY: the local test cage has a writable, aligned header slot at
        // `offset - size_of::<AllocationHeader>()`.
        unsafe { header_ptr(&state, offset).write(malformed_for_u64) };

        // The header is structurally valid, but this external raw offset has
        // no owner provenance proof and must retain the typed payload check.
        assert!(read_typed_header::<u64>(&state, offset).is_err());
    }

    fn assert_pending_partition(
        state: &CageState,
        allocator: &Allocator,
        collector: &ReleaseCollector,
        live: &[ReleaseExtent],
        pending: &[ReleaseExtent],
    ) {
        assert_eq!(collector.len, pending.len());
        assert_eq!(&collector.extents[..collector.len], pending);
        let mut all = live.to_vec();
        all.extend_from_slice(pending);
        let free = allocator_free_extents(state, allocator);
        all.extend(free.iter().copied());
        all.sort_unstable_by_key(|extent| extent.start);
        for adjacent in all.windows(2) {
            assert!(adjacent[0].start + adjacent[0].len <= adjacent[1].start);
        }
        let modeled_live = live
            .iter()
            .chain(pending)
            .map(|extent| extent.len)
            .sum::<u32>();
        let modeled_free = free.iter().map(|extent| extent.len).sum::<u32>();
        assert_eq!(allocator.live_bytes, modeled_live);
        assert_eq!(
            modeled_live + modeled_free,
            allocator.cursor - INITIAL_CURSOR
        );
        validate_allocator(state, allocator).unwrap();
    }

    fn local_state(capacity: usize) -> CageState {
        let layout = Layout::from_size_align(capacity, 8).unwrap();
        // SAFETY: the test uses a nonzero, valid layout and `CageState` owns it.
        let memory = NonNull::new(unsafe { alloc(layout) }).unwrap();
        CageState {
            capacity,
            memory,
            allocator: Mutex::new(Allocator {
                cursor: INITIAL_CURSOR,
                live_bytes: 0,
                free_head: 0,
                size_class_heads: [0; SIZE_CLASSES.len()],
                size_class_counts: [0; SIZE_CLASSES.len()],
                has_size_class_cache: false,
                #[cfg(feature = "allocator-telemetry")]
                lock_acquisitions: 0,
                #[cfg(feature = "allocator-telemetry")]
                free_list_nodes_visited: 0,
                #[cfg(feature = "allocator-telemetry")]
                allocation_size_histogram: [0; BLOCK_SIZE_BUCKETS],
                #[cfg(feature = "allocator-telemetry")]
                release_batches: 0,
                #[cfg(feature = "allocator-telemetry")]
                released_extents: 0,
                #[cfg(feature = "allocator-telemetry")]
                max_release_batch: 0,
                #[cfg(feature = "allocator-telemetry")]
                size_class_hits: 0,
                #[cfg(feature = "allocator-telemetry")]
                size_class_misses: 0,
                #[cfg(feature = "allocator-telemetry")]
                global_class_hits: [0; SIZE_CLASS_COUNT],
                #[cfg(feature = "allocator-telemetry")]
                global_class_misses: [0; SIZE_CLASS_COUNT],
                #[cfg(feature = "allocator-telemetry")]
                global_class_empty: [0; SIZE_CLASS_COUNT],
                #[cfg(feature = "allocator-telemetry")]
                global_class_alignment_incompatible: [0; SIZE_CLASS_COUNT],
                #[cfg(feature = "allocator-telemetry")]
                requested_size_no_class: 0,
                #[cfg(feature = "allocator-telemetry")]
                general_list_fallbacks: 0,
                #[cfg(feature = "allocator-telemetry")]
                cursor_fallbacks: 0,
                #[cfg(feature = "allocator-telemetry")]
                released_exact_size_extents: 0,
                #[cfg(feature = "allocator-telemetry")]
                exact_size_extents_cached: 0,
                #[cfg(feature = "allocator-telemetry")]
                exact_size_extents_coalesced_before_cache: 0,
            }),
            local_reuse_activated: AtomicBool::new(false),
            active_local_cache_owners: AtomicUsize::new(0),
            local_cache_bytes: AtomicUsize::new(0),
            local_cache_budget: (capacity / 50).min(LOCAL_CACHE_BYTE_BUDGET),
            pending_releases: Mutex::new(PendingReleaseQueue::default()),
            pending_release_nonempty: AtomicBool::new(false),
            allocator_faulted: AtomicBool::new(false),
        }
    }

    #[test]
    fn pending_exact_reuse_preserves_live_accounting_and_compacts_middle() {
        let state = local_state(4096);
        let mut allocator = lock(&state).unwrap();
        let first_filler = local_allocate(&state, &mut allocator, 8);
        let middle = local_allocate(&state, &mut allocator, 24);
        let last_filler = local_allocate(&state, &mut allocator, 8);
        let live_before = allocator.live_bytes;
        drop(allocator);

        let mut collector = ReleaseCollector::new();
        for extent in [first_filler, middle, last_filler] {
            collector.push(extent);
        }
        let original_order = collector.extents[..collector.len].to_vec();
        let (lookup, recycled) = collector.take_compatible(state.base(), 24, 8);
        let recycled = recycled.expect("middle block has the exact requested size");
        assert_eq!(lookup, PendingLookup::Recycled);
        assert_eq!(recycled.data_offset.get(), middle.start + 16);
        assert_eq!(collector.len, 2);
        assert_eq!(collector.extents[0], original_order[0]);
        assert_eq!(collector.extents[1], original_order[2]);
        let allocator = lock(&state).unwrap();
        assert_eq!(allocator.live_bytes, live_before);
        drop(allocator);
        let pending = collector.extents[..collector.len].to_vec();
        assert_pending_partition(
            &state,
            &lock(&state).unwrap(),
            &collector,
            &[middle],
            &pending,
        );

        local_flush(&state, &mut collector);
        assert_eq!(collector.len, 0);
        let mut allocator = lock(&state).unwrap();
        assert_eq!(allocator.live_bytes, middle.len);
        let mut recycled_as_live = [middle];
        local_release(&state, &mut allocator, &mut recycled_as_live);
        assert_eq!(allocator.live_bytes, 0);
        assert_eq!(allocator.cursor, INITIAL_CURSOR);
        validate_allocator(&state, &allocator).unwrap();
    }

    #[test]
    fn pending_reuse_prefers_most_recent_matching_extent() {
        let state = local_state(4096);
        let mut allocator = lock(&state).unwrap();
        let older = local_allocate(&state, &mut allocator, 24);
        let filler = local_allocate(&state, &mut allocator, 8);
        let newer = local_allocate(&state, &mut allocator, 24);
        drop(allocator);
        let mut collector = ReleaseCollector::new();
        for extent in [older, filler, newer] {
            collector.push(extent);
        }

        let (_, first_reuse) = collector.take_compatible(state.base(), 24, 8);
        assert_eq!(first_reuse.unwrap().data_offset.get(), newer.start + 16);
        let (_, second_reuse) = collector.take_compatible(state.base(), 24, 8);
        assert_eq!(second_reuse.unwrap().data_offset.get(), older.start + 16);
        assert_eq!(collector.len, 1);
        assert_eq!(collector.extents[0], filler);
    }

    #[test]
    fn pending_reuse_reports_size_and_alignment_misses() {
        let state = local_state(4096);
        let mut allocator = lock(&state).unwrap();
        // The size-miss extent must be unreusable by the 64-byte-aligned probe
        // below for *every* cage base address. A 64-byte-aligned block for an
        // 8-byte request has length `prefix + 24` with `prefix` in
        // `0..=56 step 8`, so its exact block length is one of `24..=80`. A
        // 72-byte request leaves an 88-byte extent that can never coincide,
        // making the probe deterministic instead of a function of the base
        // address residue mod 64 (as a 24-byte request would be).
        let wrong_size = local_allocate(&state, &mut allocator, 72);
        assert_eq!(wrong_size.len, 88);
        let mut alignment_candidate = None;
        for _ in 0..8 {
            let extent = local_allocate(&state, &mut allocator, 8);
            let aligned_len = block_layout(state.base(), extent.start, 8, 64).unwrap().2;
            if aligned_len > extent.len {
                alignment_candidate = Some(extent);
                break;
            }
        }
        let alignment_candidate = alignment_candidate
            .expect("one of the consecutive blocks needs extra 64-byte alignment padding");
        drop(allocator);

        let mut collector = ReleaseCollector::new();
        collector.push(wrong_size);
        let (wrong_size_lookup, wrong_size_recycled) =
            collector.take_compatible(state.base(), 8, 8);
        assert!(wrong_size_recycled.is_none());
        assert_eq!(
            wrong_size_lookup,
            PendingLookup::NoExactBlock {
                alignment_incompatible: false
            }
        );
        collector.push(alignment_candidate);
        let (alignment_lookup, alignment_recycled) = collector.take_compatible(state.base(), 8, 64);
        assert!(alignment_recycled.is_none());
        assert_eq!(
            alignment_lookup,
            PendingLookup::NoExactBlock {
                alignment_incompatible: true
            }
        );
        assert_eq!(collector.len, 2);
    }

    #[test]
    fn pending_reuse_decision_tracks_cage_base_alignment() {
        // A released 24-byte allocation leaves a 40-byte extent. Whether it can
        // back an 8-byte/64-byte-aligned request depends only on the absolute
        // cage base alignment, so pin both residues with a 64-byte-aligned
        // scratch allocation. `block_layout` and `take_compatible` perform
        // integer math on the base only, so these pointers are never
        // dereferenced.
        let extent = ReleaseExtent { start: 8, len: 40 };
        let layout = Layout::from_size_align(128, 64).unwrap();
        // SAFETY: the test uses a valid, nonzero layout and frees it below.
        let memory = NonNull::new(unsafe { alloc(layout) }).unwrap();
        assert_eq!(memory.as_ptr() as usize % 64, 0);
        let non_fitting_base = memory.as_ptr();
        // SAFETY: the 24-byte offset stays inside the 128-byte allocation and
        // the derived pointer is only used for layout arithmetic.
        let fitting_base = unsafe { memory.as_ptr().add(24) };

        // base = 24 (mod 64) needs a 16-byte prefix, so an 8/64 request lands
        // on the exact 40-byte block length and can reuse the extent.
        assert_eq!(
            block_layout(fitting_base, extent.start, 8, 64).unwrap().2,
            40
        );
        // base = 0 (mod 64) needs a 40-byte prefix, giving a 64-byte block
        // that cannot reuse the 40-byte extent.
        assert_eq!(
            block_layout(non_fitting_base, extent.start, 8, 64)
                .unwrap()
                .2,
            64
        );

        let mut collector = ReleaseCollector::new();
        collector.push(extent);
        let (lookup, recycled) = collector.take_compatible(fitting_base, 8, 64);
        assert_eq!(lookup, PendingLookup::Recycled);
        let recycled = recycled.expect("the 40-byte extent fits exactly");
        assert_eq!(recycled.block_len, 40);
        assert_eq!(recycled.prefix, 16);
        assert_eq!(recycled.data_offset.get(), extent.start + 16 + 16);
        assert_eq!(collector.len, 0);

        collector.push(extent);
        let (lookup, recycled) = collector.take_compatible(non_fitting_base, 8, 64);
        assert_eq!(
            lookup,
            PendingLookup::NoExactBlock {
                alignment_incompatible: false
            }
        );
        assert!(recycled.is_none());
        assert_eq!(collector.len, 1);

        // SAFETY: same layout used for the allocation above.
        unsafe { dealloc(memory.as_ptr(), layout) };
    }

    #[test]
    fn pending_collector_preserves_diverse_unmatched_extents() {
        let state = local_state(1 << 16);
        let sizes = [8_usize, 16, 24, 32, 80, 512];
        let extents = {
            let mut allocator = lock(&state).unwrap();
            (0..RELEASE_BATCH_CAPACITY)
                .map(|index| local_allocate(&state, &mut allocator, sizes[index % sizes.len()]))
                .collect::<Vec<_>>()
        };
        let mut collector = ReleaseCollector::new();
        for extent in &extents {
            collector.push(*extent);
        }

        let target_index = (0..extents.len())
            .rev()
            .find(|index| sizes[index % sizes.len()] == 24)
            .unwrap();
        let target = extents[target_index];
        let (lookup, recycled) = collector.take_compatible(state.base(), 24, 8);
        assert_eq!(lookup, PendingLookup::Recycled);
        let recycled = recycled.unwrap();
        assert_eq!(recycled.data_offset.get(), target.start + 16);
        assert_eq!(recycled.block_len, target.len);

        let (lookup, miss) = collector.take_compatible(state.base(), 4096, 8);
        assert_eq!(
            lookup,
            PendingLookup::NoExactBlock {
                alignment_incompatible: false
            }
        );
        assert!(miss.is_none());
        let expected_pending = extents
            .iter()
            .copied()
            .filter(|extent| *extent != target)
            .collect::<Vec<_>>();
        assert_eq!(&collector.extents[..collector.len], expected_pending);

        let allocator = lock(&state).unwrap();
        assert_pending_partition(&state, &allocator, &collector, &[target], &expected_pending);
        drop(allocator);
        local_flush(&state, &mut collector);
        let mut allocator = lock(&state).unwrap();
        let mut recycled_live = [target];
        local_release(&state, &mut allocator, &mut recycled_live);
        assert_eq!(allocator.live_bytes, 0);
        assert_eq!(allocator.cursor, INITIAL_CURSOR);
        validate_allocator(&state, &allocator).unwrap();
    }

    #[test]
    fn small_alignment_layout_matches_general_layout() {
        let state = local_state(4096);
        for start in (INITIAL_CURSOR..512).step_by(8) {
            for bytes in [0_usize, 1, 7, 8, 9, 15, 16, 24, 31, 32, 80, 512] {
                for alignment in [1_usize, 2, 4, 8, 16, 64] {
                    assert_eq!(
                        block_layout(state.base(), start, bytes, alignment).unwrap(),
                        block_layout_general(state.base(), start, bytes, alignment).unwrap(),
                        "start={start} bytes={bytes} alignment={alignment}"
                    );
                }
            }
        }
        for alignment in [1_usize, 2, 4, 8] {
            assert_eq!(
                block_layout(state.base(), INITIAL_CURSOR + 1, 24, alignment).unwrap(),
                block_layout_general(state.base(), INITIAL_CURSOR + 1, 24, alignment).unwrap()
            );
        }
    }

    #[test]
    fn empty_reusable_ranges_allocate_from_the_existing_cursor() {
        let state = local_state(4096);
        let mut allocator = lock(&state).unwrap();
        let start = allocator.cursor;
        let (data, prefix, block_len) = allocate_block(&state, &mut allocator, 16, 8).unwrap();
        assert_eq!(prefix, 0);
        assert_eq!(data, start + size_of::<AllocationHeader>() as u32);
        assert_eq!(block_len, 32);
        assert_eq!(allocator.cursor, start + block_len);
        assert_eq!(allocator.live_bytes, block_len);
        assert_eq!(allocator.free_head, 0);
        assert!(!allocator.has_size_class_cache);
        validate_allocator(&state, &allocator).unwrap();
    }

    #[cfg(all(
        feature = "allocator-telemetry",
        any(feature = "benchmark-allocator-a", feature = "benchmark-allocator-c")
    ))]
    #[test]
    fn global_class_telemetry_separates_alignment_misses() {
        let state = local_state(16 * 1024);
        let mut allocator = lock(&state).unwrap();
        let mut alignment_misses = Vec::new();
        for _ in 0..16 {
            let extent = local_allocate(&state, &mut allocator, 16);
            if block_layout(state.base(), extent.start, 16, 64).unwrap().2 > extent.len {
                alignment_misses.push(extent);
            }
        }
        assert!(alignment_misses.len() >= 2);
        let first = alignment_misses[0];
        let second = *alignment_misses
            .iter()
            .find(|extent| extent.start > first.start + first.len)
            .expect("separated cached candidates avoid coalescing");
        let mut release = [first, second];
        local_release(&state, &mut allocator, &mut release);
        assert_eq!(allocator.size_class_counts[0], 2);
        let alignment_before = allocator.global_class_alignment_incompatible[0];

        let _ = allocate_block(&state, &mut allocator, 16, 64).unwrap();

        assert_eq!(
            allocator.global_class_alignment_incompatible[0],
            alignment_before + 1
        );
        assert!(allocator.global_class_misses[0] > 0);
        assert!(allocator.global_class_empty[1..]
            .iter()
            .all(|count| *count > 0));
        validate_allocator(&state, &allocator).unwrap();
    }

    #[cfg(feature = "allocator-telemetry")]
    #[test]
    fn pending_scan_telemetry_records_depth_and_candidate_sizes() {
        let state = local_state(4096);
        let extents = {
            let mut allocator = lock(&state).unwrap();
            [
                local_allocate(&state, &mut allocator, 8),
                local_allocate(&state, &mut allocator, 24),
            ]
        };
        let mut collector = ReleaseCollector::new();
        collector.push(extents[0]);
        collector.push(extents[1]);

        let depth_before = PENDING_REUSE_TELEMETRY.scan_depth_histogram[2].load(Ordering::Relaxed);
        let candidates_before = PENDING_REUSE_TELEMETRY
            .scan_candidates
            .load(Ordering::Relaxed);
        let small_bucket = (extents[0].len as usize) / 8;
        let large_bucket = (extents[1].len as usize) / 8;
        let small_before =
            PENDING_REUSE_TELEMETRY.candidate_size_histogram[small_bucket].load(Ordering::Relaxed);
        let large_before =
            PENDING_REUSE_TELEMETRY.candidate_size_histogram[large_bucket].load(Ordering::Relaxed);

        let (lookup, recycled) = collector.take_compatible(state.base(), 2048, 8);
        assert_eq!(
            lookup,
            PendingLookup::NoExactBlock {
                alignment_incompatible: false
            }
        );
        assert!(recycled.is_none());
        assert!(
            PENDING_REUSE_TELEMETRY.scan_depth_histogram[2].load(Ordering::Relaxed) > depth_before
        );
        assert!(
            PENDING_REUSE_TELEMETRY
                .scan_candidates
                .load(Ordering::Relaxed)
                >= candidates_before + 2
        );
        assert!(
            PENDING_REUSE_TELEMETRY.candidate_size_histogram[small_bucket].load(Ordering::Relaxed)
                > small_before
        );
        assert!(
            PENDING_REUSE_TELEMETRY.candidate_size_histogram[large_bucket].load(Ordering::Relaxed)
                > large_before
        );
    }

    #[test]
    fn full_pending_collector_accepts_64_and_flushes_remaining_exactly_once() {
        let state = local_state(8192);
        let mut allocator = lock(&state).unwrap();
        let extents = (0..RELEASE_BATCH_CAPACITY)
            .map(|_| local_allocate(&state, &mut allocator, 16))
            .collect::<Vec<_>>();
        drop(allocator);
        let mut collector = ReleaseCollector::new();
        for extent in &extents {
            collector.push(*extent);
        }
        assert_eq!(collector.len, RELEASE_BATCH_CAPACITY);
        let (lookup, recycled) = collector.take_compatible(state.base(), 16, 8);
        assert_eq!(lookup, PendingLookup::Recycled);
        assert_eq!(
            recycled.unwrap().data_offset.get(),
            extents.last().unwrap().start + 16
        );
        let pending = collector.extents[..collector.len].to_vec();
        assert_pending_partition(
            &state,
            &lock(&state).unwrap(),
            &collector,
            &[*extents.last().unwrap()],
            &pending,
        );
        local_flush(&state, &mut collector);
        let mut allocator = lock(&state).unwrap();
        let mut recycled_live = [*extents.last().unwrap()];
        local_release(&state, &mut allocator, &mut recycled_live);
        assert_eq!(allocator.live_bytes, 0);
        assert_eq!(allocator.cursor, INITIAL_CURSOR);
        validate_allocator(&state, &allocator).unwrap();
    }

    #[test]
    fn release_collector_keeps_descriptors_when_every_publication_attempt_fails() {
        let mut collector = ReleaseCollector::new();
        let first = ReleaseExtent { start: 64, len: 32 };
        let second = ReleaseExtent {
            start: 128,
            len: 40,
        };
        collector.push(first);
        collector.push(second);
        let mut attempted = Vec::new();

        collector.flush_with(|extents| {
            attempted.extend_from_slice(extents);
            Err(Error::InvalidOffset)
        });
        assert_eq!(collector.len, 2);
        assert_eq!(&collector.extents[..collector.len], &[first, second]);
        assert_eq!(attempted.len(), 4);

        collector.flush_with(|_| Ok(()));
        assert_eq!(collector.len, 0);
    }

    #[test]
    fn local_reuse_cache_recycles_an_exact_extent_before_global_publication() {
        let state = local_state(4096);
        state.local_reuse_activated.store(true, Ordering::Release);
        let extent = {
            let mut allocator = lock(&state).unwrap();
            local_allocate(&state, &mut allocator, 16)
        };
        let data_offset = extent.start + size_of::<AllocationHeader>() as u32;
        let original_header = AllocationHeader {
            block_len: extent.len,
            prefix: 0,
            capacity: 16,
            initialized: 0,
        };
        initialize_allocation_header(&state, data_offset, original_header);

        assert!(cache_released_extent(&state, extent));
        assert_eq!(
            state.local_cache_bytes.load(Ordering::Acquire),
            extent.len as usize
        );
        let recycled = take_local_reuse(&state, 16, 8, 16).unwrap();
        assert_eq!(recycled.data_offset.get(), data_offset);
        assert_eq!(recycled.block_len, extent.len);
        assert_eq!(state.local_cache_bytes.load(Ordering::Acquire), 0);
        // SAFETY: local reuse initialized the header before removing its only
        // cache descriptor.
        assert_eq!(
            unsafe { read_header(&state, data_offset) }
                .unwrap()
                .capacity,
            16
        );

        LOCAL_REUSE_CACHE.with(|slot| {
            assert!(slot.registered.replace(false));
            assert_eq!(slot.cache.borrow().len, 0);
        });
        assert_eq!(state.active_local_cache_owners.load(Ordering::Acquire), 1);
        state
            .active_local_cache_owners
            .fetch_sub(1, Ordering::AcqRel);
        let allocator = lock(&state).unwrap();
        assert_eq!(allocator.live_bytes, extent.len);
        validate_allocator(&state, &allocator).unwrap();
    }

    #[test]
    fn actual_allocator_lock_contention_activates_local_reuse() {
        let state = Arc::new(local_state(4096));
        let allocator_guard = state.allocator.lock().unwrap();
        let started = Arc::new(std::sync::Barrier::new(2));
        let worker_state = Arc::clone(&state);
        let worker_started = Arc::clone(&started);
        let worker = std::thread::spawn(move || {
            worker_started.wait();
            drop(lock_for_allocation(&worker_state).unwrap());
        });
        started.wait();
        while !state.local_reuse_activated.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        drop(allocator_guard);
        worker.join().unwrap();
        assert!(state.local_reuse_activated.load(Ordering::Acquire));
    }

    #[test]
    fn pending_release_queue_drains_in_bounded_batches_under_concurrent_publication() {
        const RELEASES: usize = RELEASE_BATCH_CAPACITY * 2 + 3;
        let state = Arc::new(local_state(8192));
        let extents = {
            let mut allocator = lock(&state).unwrap();
            (0..RELEASES)
                .map(|_| local_allocate(&state, &mut allocator, 8))
                .collect::<Vec<_>>()
        };

        std::thread::scope(|scope| {
            for partition in extents.chunks(RELEASE_BATCH_CAPACITY / 2) {
                let state = Arc::clone(&state);
                scope.spawn(move || {
                    for extent in partition.iter().copied() {
                        enqueue_pending_release(&state, extent);
                    }
                });
            }
        });

        {
            let allocator = lock(&state).unwrap();
            assert_eq!(
                allocator.live_bytes,
                (RELEASES - RELEASE_BATCH_CAPACITY) as u32 * 24
            );
            assert_ne!(state.pending_releases.lock().unwrap().head, 0);
            assert!(state.pending_release_nonempty.load(Ordering::Acquire));
        }
        {
            let allocator = lock(&state).unwrap();
            assert_eq!(
                allocator.live_bytes,
                (RELEASES - RELEASE_BATCH_CAPACITY * 2) as u32 * 24
            );
            assert_ne!(state.pending_releases.lock().unwrap().head, 0);
            assert!(state.pending_release_nonempty.load(Ordering::Acquire));
        }
        {
            let allocator = lock(&state).unwrap();
            assert_eq!(allocator.live_bytes, 0);
            assert_eq!(allocator.cursor, INITIAL_CURSOR);
            assert_eq!(state.pending_releases.lock().unwrap().head, 0);
            assert!(!state.pending_release_nonempty.load(Ordering::Acquire));
            validate_allocator(&state, &allocator).unwrap();
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, .. ProptestConfig::default() })]

        #[test]
        fn pending_global_allocator_model(operations in prop::collection::vec((0_u8..3, 0_u8..6), 1..240)) {
            let state = local_state(1 << 20);
            let mut allocator = lock(&state).unwrap();
            let mut collector = ReleaseCollector::new();
            let mut live = Vec::<ReleaseExtent>::new();
            let mut pending = Vec::<ReleaseExtent>::new();
            let sizes = [8_usize, 16, 24, 32, 80, 512];

            for (operation, size_index) in operations {
                match operation {
                    0 => {
                        let bytes = sizes[size_index as usize];
                        drop(allocator);
                        let (lookup, recycled) = collector.take_compatible(state.base(), bytes, 8);
                        if let Some(recycled) = recycled {
                            prop_assert_eq!(lookup, PendingLookup::Recycled);
                            let extent = ReleaseExtent {
                                start: recycled.data_offset.get()
                                    - size_of::<AllocationHeader>() as u32
                                    - recycled.prefix,
                                len: recycled.block_len,
                            };
                            let pending_index = pending.iter().position(|candidate| *candidate == extent)
                                .expect("recycled extent exists in pending category");
                            pending.remove(pending_index);
                            live.push(extent);
                        } else {
                            let mut guard = lock(&state).unwrap();
                            let (data, prefix, len) = allocate_block(&state, &mut guard, bytes, 8).unwrap();
                            live.push(ReleaseExtent {
                                start: data - size_of::<AllocationHeader>() as u32 - prefix,
                                len,
                            });
                        }
                        allocator = lock(&state).unwrap();
                    }
                    1 => {
                        if !live.is_empty() {
                            if collector.len == RELEASE_BATCH_CAPACITY {
                                drop(allocator);
                                local_flush(&state, &mut collector);
                                pending.clear();
                                allocator = lock(&state).unwrap();
                            }
                            let index = (size_index as usize) % live.len();
                            let extent = live.swap_remove(index);
                            collector.push(extent);
                            pending.push(extent);
                        }
                    }
                    _ => {
                        drop(allocator);
                        local_flush(&state, &mut collector);
                        pending.clear();
                        allocator = lock(&state).unwrap();
                    }
                }
                assert_pending_partition(&state, &allocator, &collector, &live, &pending);
            }

            drop(allocator);
            local_flush(&state, &mut collector);
            pending.clear();
            let mut allocator = lock(&state).unwrap();
            let mut batch = core::mem::take(&mut live);
            while !batch.is_empty() {
                let take = batch.len().min(RELEASE_BATCH_CAPACITY);
                let mut chunk = batch.split_off(batch.len() - take);
                local_release(&state, &mut allocator, &mut chunk);
            }
            assert_eq!(allocator.live_bytes, 0);
            assert_eq!(allocator.cursor, INITIAL_CURSOR);
            validate_allocator(&state, &allocator).unwrap();
        }
    }

    #[test]
    fn allocation_representation_and_intrusive_free_ranges() {
        assert!(size_of::<AllocationHeader>() <= 16);
        assert_eq!(size_of::<CageAllocation<u64>>(), 4);
        assert_eq!(size_of::<Option<CageAllocation<u64>>>(), 4);

        let state = local_state(4096);
        let mut allocator = lock(&state).unwrap();
        let mut starts = [0_u32; 4];
        let mut lengths = [0_u32; 4];
        for index in 0..4 {
            starts[index] = allocator.cursor;
            let (_, _, len) = allocate_block(&state, &mut allocator, 32, 8).unwrap();
            lengths[index] = len;
        }

        for index in [0, 2, 1] {
            allocator.live_bytes -= lengths[index];
            insert_free(&state, &mut allocator, starts[index], lengths[index]).unwrap();
        }
        let combined = unsafe { read_free_node(&state, starts[0]).unwrap() };
        assert_eq!(combined.len, lengths[0] + lengths[1] + lengths[2]);
        assert_eq!(combined.next, 0);

        let (_, _, split_len) = allocate_block(&state, &mut allocator, 16, 8).unwrap();
        assert_eq!(allocator.free_head, starts[0] + split_len);
        let split = unsafe { read_free_node(&state, allocator.free_head).unwrap() };
        assert_eq!(split.len, combined.len - split_len);

        allocator.live_bytes -= split_len;
        insert_free(&state, &mut allocator, starts[0], split_len).unwrap();
        let combined = unsafe { read_free_node(&state, starts[0]).unwrap() };
        let prefix = block_layout(state.base(), starts[0], 1, 8).unwrap().1;
        let exact_bytes = combined.len - prefix - size_of::<AllocationHeader>() as u32;
        let old_cursor = allocator.cursor;
        let (_, _, exact_len) =
            allocate_block(&state, &mut allocator, exact_bytes as usize, 8).unwrap();
        assert_eq!(exact_len, combined.len);
        assert_eq!(allocator.free_head, 0);
        assert_eq!(allocator.cursor, old_cursor);

        allocator.live_bytes -= exact_len;
        insert_free(&state, &mut allocator, starts[0], exact_len).unwrap();
        allocator.live_bytes -= lengths[3];
        insert_free(&state, &mut allocator, starts[3], lengths[3]).unwrap();
        assert_eq!(allocator.cursor, starts[0]);
        assert_eq!(allocator.free_head, 0);
        assert_eq!(allocator.live_bytes, 0);
        validate_allocator(&state, &allocator).unwrap();
    }

    #[cfg(any(feature = "benchmark-allocator-a", feature = "benchmark-allocator-c"))]
    #[test]
    fn batch_release_reuses_classes_merges_neighbors_and_contracts_tail() {
        let state = local_state(4096);
        let mut allocator = lock(&state).unwrap();
        let a = local_allocate(&state, &mut allocator, 16);
        let b = local_allocate(&state, &mut allocator, 16);
        let c = local_allocate(&state, &mut allocator, 16);
        let d = local_allocate(&state, &mut allocator, 16);

        let mut first_batch = [d, b];
        local_release(&state, &mut allocator, &mut first_batch);
        assert_eq!(allocator.size_class_counts[0], 1);
        assert_eq!(allocator.cursor, d.start);
        validate_allocator(&state, &allocator).unwrap();

        let reused = local_allocate(&state, &mut allocator, 16);
        assert_eq!(reused.start, b.start);
        assert_eq!(allocator.size_class_counts[0], 0);
        validate_allocator(&state, &allocator).unwrap();

        let mut last_batch = [c, reused, a];
        local_release(&state, &mut allocator, &mut last_batch);
        assert_eq!(allocator.cursor, INITIAL_CURSOR);
        assert_eq!(allocator.free_head, 0);
        assert_eq!(allocator.size_class_counts, [0; SIZE_CLASSES.len()]);
        assert_eq!(allocator.live_bytes, 0);
        validate_allocator(&state, &allocator).unwrap();
    }

    #[test]
    fn contiguous_tail_batch_contracts_without_creating_free_ranges() {
        let state = local_state(4096);
        let mut allocator = lock(&state).unwrap();
        let a = local_allocate(&state, &mut allocator, 16);
        let b = local_allocate(&state, &mut allocator, 16);
        let c = local_allocate(&state, &mut allocator, 16);

        let mut releases = [c, a, b];
        local_release(&state, &mut allocator, &mut releases);

        assert_eq!(allocator.cursor, INITIAL_CURSOR);
        assert_eq!(allocator.live_bytes, 0);
        assert_eq!(allocator.free_head, 0);
        assert!(!allocator.has_size_class_cache);
        validate_allocator(&state, &allocator).unwrap();
    }

    #[cfg(any(feature = "benchmark-allocator-a", feature = "benchmark-allocator-c"))]
    #[test]
    fn class_neighbors_merge_into_general_ranges_on_release() {
        let state = local_state(4096);
        let mut allocator = lock(&state).unwrap();
        let a = local_allocate(&state, &mut allocator, 16);
        let b = local_allocate(&state, &mut allocator, 16);
        let c = local_allocate(&state, &mut allocator, 16);
        let d = local_allocate(&state, &mut allocator, 16);
        let e = local_allocate(&state, &mut allocator, 16);

        let mut release_separated = [d, b];
        local_release(&state, &mut allocator, &mut release_separated);
        assert_eq!(allocator.size_class_counts[0], 2);

        let mut release_c = [c];
        local_release(&state, &mut allocator, &mut release_c);
        assert_eq!(allocator.size_class_counts[0], 0);
        assert_eq!(allocator.free_head, b.start);
        let merged = unsafe { read_free_node(&state, b.start).unwrap() };
        assert_eq!(merged.len, b.len + c.len + d.len);
        validate_allocator(&state, &allocator).unwrap();

        let mut release_edges = [e, a];
        local_release(&state, &mut allocator, &mut release_edges);
        assert_eq!(allocator.cursor, INITIAL_CURSOR);
        assert_eq!(allocator.live_bytes, 0);
        validate_allocator(&state, &allocator).unwrap();
    }

    #[cfg(any(feature = "benchmark-allocator-a", feature = "benchmark-allocator-c"))]
    #[test]
    fn resize_merge_exposes_cached_neighbor_to_ordered_free_list() {
        let state = local_state(4096);
        let mut allocator = lock(&state).unwrap();
        let a = local_allocate(&state, &mut allocator, 16);
        let b = local_allocate(&state, &mut allocator, 16);
        let c = local_allocate(&state, &mut allocator, 16);
        let d = local_allocate(&state, &mut allocator, 16);
        let e = local_allocate(&state, &mut allocator, 16);

        let mut release_neighbor = [d, b];
        local_release(&state, &mut allocator, &mut release_neighbor);
        assert_eq!(allocator.size_class_counts[0], 2);
        merge_free_ranges_locked(&state, &mut allocator).unwrap();
        assert_eq!(allocator.size_class_counts, [0; SIZE_CLASSES.len()]);
        assert_eq!(
            free_node_at(&state, &allocator, b.start).unwrap(),
            Some((b.len, d.start))
        );

        consume_free_prefix(&state, &mut allocator, b.start, b.len).unwrap();
        allocator.live_bytes += b.len;
        validate_allocator(&state, &allocator).unwrap();
        let grown_a = ReleaseExtent {
            start: a.start,
            len: a.len + b.len,
        };
        let mut release_all = [c, grown_a, e];
        local_release(&state, &mut allocator, &mut release_all);
        assert_eq!(allocator.cursor, INITIAL_CURSOR);
        validate_allocator(&state, &allocator).unwrap();
    }

    #[test]
    fn overlapping_batch_is_rejected_before_allocator_mutation() {
        let state = local_state(4096);
        let mut allocator = lock(&state).unwrap();
        let a = local_allocate(&state, &mut allocator, 16);
        let b = local_allocate(&state, &mut allocator, 16);
        let original_live_bytes = allocator.live_bytes;
        let mut duplicate_release = [a, a];
        assert!(release_many_locked(&state, &mut allocator, &mut duplicate_release, true).is_err());
        assert_eq!(allocator.live_bytes, original_live_bytes);
        assert_eq!(allocator.free_head, 0);
        assert_eq!(allocator.size_class_counts, [0; SIZE_CLASSES.len()]);
        validate_allocator(&state, &allocator).unwrap();
        let mut release_all = [b, a];
        local_release(&state, &mut allocator, &mut release_all);
        assert_eq!(allocator.cursor, INITIAL_CURSOR);
    }

    #[test]
    fn production_allocator_operations_use_model_legal_transitions() {
        use crate::allocator_model::{transition_is_legal, ByteState, ThreadKey};
        let owner = ThreadKey(1);
        let state = local_state(4096);
        let mut allocator = lock(&state).unwrap();

        // Keep a live block after the first allocation so releasing the first
        // creates a reusable extent instead of contracting the unallocated tail.
        let first = local_allocate(&state, &mut allocator, 24);
        let blocker = local_allocate(&state, &mut allocator, 24);
        let mut release_first = [first];
        local_release(&state, &mut allocator, &mut release_first);

        // Free -> Live on an actual free-list or size-class allocation.
        let (data, prefix, block_len) = allocate_block(&state, &mut allocator, 24, 8).unwrap();
        let reused = ReleaseExtent {
            start: data - size_of::<AllocationHeader>() as u32 - prefix,
            len: block_len,
        };
        assert_eq!(reused, first);
        assert!(transition_is_legal(
            ByteState::Free,
            ByteState::Live { owner }
        ));

        // Live -> PendingRelease when the reused extent enters a collector.
        let mut collector = ReleaseCollector::new();
        collector.push(reused);
        assert!(transition_is_legal(
            ByteState::Live { owner },
            ByteState::PendingRelease { owner }
        ));

        // PendingRelease -> Live on exact-size pending reuse.
        let (lookup, recycled) = collector.take_compatible(state.base(), 24, 8);
        assert!(matches!(lookup, PendingLookup::Recycled));
        let recycled = recycled.expect("the pending exact extent is compatible");
        assert_eq!(recycled.block_len, reused.len);
        assert_eq!(
            recycled.data_offset.get(),
            reused.start + recycled.prefix + size_of::<AllocationHeader>() as u32
        );
        assert!(transition_is_legal(
            ByteState::PendingRelease { owner },
            ByteState::Live { owner }
        ));

        // Live -> Free when the exact reused extent is flushed globally.
        let mut release_reused = [reused];
        local_release(&state, &mut allocator, &mut release_reused);
        assert!(transition_is_legal(
            ByteState::Live { owner },
            ByteState::Free
        ));
        let mut release_blocker = [blocker];
        local_release(&state, &mut allocator, &mut release_blocker);
        assert_eq!(allocator.live_bytes, 0);
        validate_allocator(&state, &allocator).unwrap();
    }

    #[test]
    fn deterministic_allocator_churn_matches_live_extent_model() {
        let state = local_state(64 * 1024);
        let mut allocator = lock(&state).unwrap();
        let mut live = Vec::<ReleaseExtent>::new();
        let mut random = 0x9e37_79b9_u32;
        let sizes = [1_usize, 8, 16, 24, 32, 80, 240, 512];

        for _ in 0..1_200 {
            random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if live.is_empty() || (live.len() < RELEASE_BATCH_CAPACITY && random % 100 < 61) {
                let bytes = sizes[(random as usize >> 8) % sizes.len()];
                match allocate_block(&state, &mut allocator, bytes, 8) {
                    Ok((data, prefix, len)) => {
                        assert_eq!((state.base() as usize + data as usize) % 8, 0);
                        live.push(ReleaseExtent {
                            start: data - size_of::<AllocationHeader>() as u32 - prefix,
                            len,
                        });
                    }
                    Err(Error::AllocationExhausted) => {
                        let mut release_all = core::mem::take(&mut live);
                        local_release(&state, &mut allocator, &mut release_all);
                    }
                    Err(error) => panic!("unexpected local allocation error: {error:?}"),
                }
            } else {
                let count = 1 + ((random as usize >> 16) % live.len().min(8));
                let mut batch = Vec::with_capacity(count);
                for _ in 0..count {
                    random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let index = random as usize % live.len();
                    batch.push(live.swap_remove(index));
                }
                local_release(&state, &mut allocator, &mut batch);
            }

            let live_bytes = live.iter().map(|extent| extent.len).sum::<u32>();
            assert_eq!(allocator.live_bytes, live_bytes);
            let mut ordered = live.clone();
            ordered.sort_unstable_by_key(|extent| extent.start);
            for adjacent in ordered.windows(2) {
                assert!(adjacent[0].start + adjacent[0].len <= adjacent[1].start);
            }
            validate_allocator(&state, &allocator).unwrap();
        }

        let mut release_all = core::mem::take(&mut live);
        local_release(&state, &mut allocator, &mut release_all);
        assert_eq!(allocator.live_bytes, 0);
        assert_eq!(allocator.cursor, INITIAL_CURSOR);
        validate_allocator(&state, &allocator).unwrap();
    }
}
