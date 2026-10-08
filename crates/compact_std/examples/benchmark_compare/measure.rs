use compact_std::{AllocatorStats, CompactRuntime};
use std::alloc::{GlobalAlloc, Layout, System};
use std::error::Error;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::time::Instant;

pub type BenchResult<T> = Result<T, Box<dyn Error>>;

/// Version of the measurement-mode contract carried by every run.
///
/// Bump this whenever the meaning of a mode changes so saved artifacts can be
/// distinguished without guessing.
pub const MEASUREMENT_MODE_VERSION: u32 = 1;

/// Explicit measurement modes for the shared benchmark harness.
///
/// `Measure` keeps full allocator accounting: per-allocation counters on the
/// native side and per-phase allocator snapshots on the compact side, so phase
/// timings and memory statistics stay comparable across runs.
///
/// `Profile` drops that accounting so a CPU profiler observes the workload
/// instead of the instrumentation, and must therefore never be compared
/// numerically against `Measure` output. Only the logical checksum, which is
/// identical in both modes, is comparable across them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasurementMode {
    Measure,
    Profile,
}

impl MeasurementMode {
    pub fn name(self) -> &'static str {
        match self {
            MeasurementMode::Measure => "measure",
            MeasurementMode::Profile => "profile",
        }
    }

    pub fn from_name(value: &str) -> Option<Self> {
        match value {
            "measure" => Some(MeasurementMode::Measure),
            "profile" => Some(MeasurementMode::Profile),
            _ => None,
        }
    }
}

static MEASUREMENT_MODE: AtomicU8 = AtomicU8::new(0);
static EMIT_PHASES: AtomicBool = AtomicBool::new(true);

pub fn set_measurement_mode(mode: MeasurementMode) {
    MEASUREMENT_MODE.store(mode as u8, Ordering::Relaxed);
}

pub fn measurement_mode() -> MeasurementMode {
    if MEASUREMENT_MODE.load(Ordering::Relaxed) == MeasurementMode::Profile as u8 {
        MeasurementMode::Profile
    } else {
        MeasurementMode::Measure
    }
}

/// Controls whether per-phase `PHASE` rows are printed.
///
/// Long sampling runs (see the `benchmark_profile` example) disable them so a
/// child emits one summary line instead of one row per phase per repetition.
/// Only `benchmark_profile` calls this, so the shared module is expected to
/// report it as unused from `benchmark_compare`.
#[allow(dead_code)]
pub fn set_phase_output(enabled: bool) {
    EMIT_PHASES.store(enabled, Ordering::Relaxed);
}

static TRACKING: AtomicBool = AtomicBool::new(false);
static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
static DEALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
static REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static PEAK_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

pub struct CountingAllocator;

// SAFETY: accounting uses independent atomics and delegates allocation
// behavior unchanged to System.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies the layout required by GlobalAlloc.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && TRACKING.load(Ordering::Relaxed) {
            record_allocation(layout.size() as u64);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies the layout required by GlobalAlloc.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() && TRACKING.load(Ordering::Relaxed) {
            record_allocation(layout.size() as u64);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if TRACKING.load(Ordering::Relaxed) {
            DEALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
            LIVE_BYTES.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        }
        // SAFETY: the caller supplies the allocation and its original layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller supplies a live allocation and its original layout.
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        if !replacement.is_null() && TRACKING.load(Ordering::Relaxed) {
            ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
            DEALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
            LIVE_BYTES.fetch_sub(layout.size() as u64, Ordering::Relaxed);
            let live = LIVE_BYTES.fetch_add(new_size as u64, Ordering::Relaxed) + new_size as u64;
            PEAK_LIVE_BYTES.fetch_max(live, Ordering::Relaxed);
        }
        replacement
    }
}

fn record_allocation(size: u64) {
    ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
    REQUESTED_BYTES.fetch_add(size, Ordering::Relaxed);
    let live = LIVE_BYTES.fetch_add(size, Ordering::Relaxed) + size;
    PEAK_LIVE_BYTES.fetch_max(live, Ordering::Relaxed);
}

#[derive(Debug)]
struct PhaseStart {
    started: Instant,
    baseline_live: u64,
    cage_before: Option<AllocatorStats>,
}

#[derive(Clone, Copy, Debug, Default)]
struct PhaseSample {
    elapsed_ns: u128,
    allocation_calls: u64,
    deallocation_calls: u64,
    requested_bytes: u64,
    live_delta: i64,
    peak_extra: u64,
    cage_live_delta: i64,
    cage_cursor: u32,
    free_bytes: u32,
    free_blocks: u32,
    largest_free_block: u32,
    allocator_lock_acquisitions: u64,
    free_list_nodes_visited: u64,
    release_batches: u64,
    released_extents: u64,
    max_release_batch: u32,
    size_class_hits: u64,
    size_class_misses: u64,
    pending_reuse_hits: u64,
    pending_reuse_misses: u64,
    pending_reuse_no_active_collector: u64,
    pending_reuse_no_exact_block: u64,
    pending_reuse_alignment_incompatible: u64,
    global_class_hits: [u64; 4],
    global_class_misses: [u64; 4],
    global_class_empty: [u64; 4],
    global_class_alignment_incompatible: [u64; 4],
    requested_size_no_class: u64,
    general_list_fallbacks: u64,
    cursor_fallbacks: u64,
    released_exact_size_extents: u64,
    exact_size_extents_cached: u64,
    exact_size_extents_coalesced_before_cache: u64,
}

#[derive(Debug)]
struct PhaseAggregate {
    name: &'static str,
    runs: usize,
    median_ns: u128,
    p95_ns: u128,
    min_ns: u128,
    max_ns: u128,
    allocation_calls: u64,
    deallocation_calls: u64,
    requested_bytes: u64,
    live_delta: i64,
    peak_extra: u64,
    cage_live_delta: i64,
    cage_cursor: u32,
    free_bytes: u32,
    free_blocks: u32,
    largest_free_block: u32,
    allocator_lock_acquisitions: u64,
    free_list_nodes_visited: u64,
    release_batches: u64,
    released_extents: u64,
    max_release_batch: u32,
    size_class_hits: u64,
    size_class_misses: u64,
    pending_reuse_hits: u64,
    pending_reuse_misses: u64,
    pending_reuse_no_active_collector: u64,
    pending_reuse_no_exact_block: u64,
    pending_reuse_alignment_incompatible: u64,
    global_class_hits: [u64; 4],
    global_class_misses: [u64; 4],
    global_class_empty: [u64; 4],
    global_class_alignment_incompatible: [u64; 4],
    requested_size_no_class: u64,
    general_list_fallbacks: u64,
    cursor_fallbacks: u64,
    released_exact_size_extents: u64,
    exact_size_extents_cached: u64,
    exact_size_extents_coalesced_before_cache: u64,
}

pub type Mutation<'a, T> = (
    &'static str,
    Box<dyn FnMut(&mut T) -> BenchResult<u64> + 'a>,
);
pub type Read<'a, T> = (&'static str, Box<dyn FnMut(&T) -> BenchResult<u64> + 'a>);

pub fn run_case<T>(
    scenario: &str,
    variant: &str,
    repetitions: usize,
    compact: bool,
    mut build: impl FnMut() -> BenchResult<T>,
    mut mutations: Vec<Mutation<'_, T>>,
    mut reads: Vec<Read<'_, T>>,
) -> BenchResult<u64> {
    let mode = measurement_mode();
    // Reset per-allocation tracking for this child so a reused process cannot
    // carry counters across scenarios.
    TRACKING.store(false, Ordering::Relaxed);
    let mut warm_state = build()?;
    let mut warm_checksum = 0_u64;
    for (_, mutation) in &mut mutations {
        warm_checksum = mix(
            warm_checksum,
            black_box(mutation(black_box(&mut warm_state))?),
        );
    }
    for (_, read) in &mut reads {
        warm_checksum = mix(warm_checksum, black_box(read(black_box(&warm_state))?));
    }
    black_box(warm_checksum);
    drop(warm_state);

    let mut samples: Vec<(&'static str, Vec<PhaseSample>)> = Vec::new();
    for (name, _) in &mutations {
        samples.push((*name, Vec::with_capacity(repetitions)));
    }
    for (name, _) in &reads {
        samples.push((*name, Vec::with_capacity(repetitions)));
    }
    samples.push(("build", Vec::with_capacity(repetitions)));
    samples.push(("drop", Vec::with_capacity(repetitions)));
    samples.push(("end_to_end", Vec::with_capacity(repetitions)));

    let mut expected_checksum = None;
    for _ in 0..repetitions {
        let mut total_ns = 0_u128;
        let build_start = begin_phase(compact)?;
        let mut state = build()?;
        let build_sample = finish_phase(build_start, compact)?;
        total_ns += build_sample.elapsed_ns;
        samples
            .iter_mut()
            .find(|(name, _)| *name == "build")
            .expect("build phase registered")
            .1
            .push(build_sample);

        let mut checksum = 0_u64;
        for (name, mutation) in &mut mutations {
            let start = begin_phase(compact)?;
            checksum = mix(checksum, black_box(mutation(black_box(&mut state))?));
            let sample = finish_phase(start, compact)?;
            total_ns += sample.elapsed_ns;
            samples
                .iter_mut()
                .find(|(sample_name, _)| sample_name == name)
                .expect("mutation phase registered")
                .1
                .push(sample);
        }
        for (name, read) in &mut reads {
            let start = begin_phase(compact)?;
            checksum = mix(checksum, black_box(read(black_box(&state))?));
            let sample = finish_phase(start, compact)?;
            total_ns += sample.elapsed_ns;
            samples
                .iter_mut()
                .find(|(sample_name, _)| sample_name == name)
                .expect("read phase registered")
                .1
                .push(sample);
        }

        let drop_start = begin_phase(compact)?;
        drop(state);
        let drop_sample = finish_phase(drop_start, compact)?;
        total_ns += drop_sample.elapsed_ns;
        samples
            .iter_mut()
            .find(|(name, _)| *name == "drop")
            .expect("drop phase registered")
            .1
            .push(drop_sample);
        samples
            .iter_mut()
            .find(|(name, _)| *name == "end_to_end")
            .expect("end-to-end phase registered")
            .1
            .push(PhaseSample {
                elapsed_ns: total_ns,
                ..PhaseSample::default()
            });

        if let Some(expected) = expected_checksum {
            if expected != checksum {
                return Err(format!(
                    "{scenario}/{variant} produced inconsistent checksum {checksum:#x}; expected {expected:#x}"
                )
                .into());
            }
        } else {
            expected_checksum = Some(checksum);
        }
        if mode == MeasurementMode::Measure && LIVE_BYTES.load(Ordering::Relaxed) != 0 {
            return Err(format!(
                "{scenario}/{variant} left {} requested native bytes live after drop",
                LIVE_BYTES.load(Ordering::Relaxed)
            )
            .into());
        }
    }

    if compact {
        if CompactRuntime::used_bytes()? != 0 {
            return Err(format!(
                "{scenario}/{variant} left {} cage bytes live after drop",
                CompactRuntime::used_bytes()?
            )
            .into());
        }
        CompactRuntime::validate_allocator_state()?;
    }
    let checksum = expected_checksum.unwrap_or(0);
    println!(
        "META\tmeasurement_mode\t{scenario}\t{variant}\t{}\t{MEASUREMENT_MODE_VERSION}\t{}",
        mode.name(),
        if mode == MeasurementMode::Measure {
            "allocator_accounting=on"
        } else {
            "allocator_accounting=off"
        }
    );
    if !EMIT_PHASES.load(Ordering::Relaxed) {
        println!("META\tchecksum\t{scenario}\t{variant}\t{checksum}");
        if let Some(rss) = peak_rss_kb() {
            println!("META\tpeak_rss_kb\t{scenario}\t{variant}\t{rss}");
        }
        return Ok(checksum);
    }
    for (name, phase_samples) in samples {
        emit_phase(scenario, variant, aggregate(name, phase_samples));
    }
    println!("META\tchecksum\t{scenario}\t{variant}\t{checksum}");
    if let Some(rss) = peak_rss_kb() {
        println!("META\tpeak_rss_kb\t{scenario}\t{variant}\t{rss}");
    }
    Ok(checksum)
}

pub fn mix(checksum: u64, value: u64) -> u64 {
    checksum
        .rotate_left(9)
        .wrapping_add(value ^ 0x9e37_79b9_7f4a_7c15)
        .wrapping_mul(0x1000_0000_01b3)
}

fn begin_phase(compact: bool) -> BenchResult<PhaseStart> {
    if measurement_mode() == MeasurementMode::Profile {
        // Profiling runs deliberately skip allocator accounting: the
        // per-allocation counters and the per-phase allocator snapshots are a
        // large share of the sampled native cost, and the snapshots also
        // contend on the cage lock, so they would distort exactly the
        // attribution the profile is meant to establish.
        return Ok(PhaseStart {
            started: Instant::now(),
            baseline_live: 0,
            cage_before: None,
        });
    }
    TRACKING.store(false, Ordering::Relaxed);
    ALLOC_CALLS.store(0, Ordering::Relaxed);
    DEALLOC_CALLS.store(0, Ordering::Relaxed);
    REQUESTED_BYTES.store(0, Ordering::Relaxed);
    let baseline_live = LIVE_BYTES.load(Ordering::Relaxed);
    PEAK_LIVE_BYTES.store(baseline_live, Ordering::Relaxed);
    let cage_before = if compact {
        Some(CompactRuntime::allocator_stats()?)
    } else {
        None
    };
    TRACKING.store(true, Ordering::Relaxed);
    Ok(PhaseStart {
        started: Instant::now(),
        baseline_live,
        cage_before,
    })
}

fn finish_phase(start: PhaseStart, compact: bool) -> BenchResult<PhaseSample> {
    let elapsed_ns = start.started.elapsed().as_nanos();
    if measurement_mode() == MeasurementMode::Profile {
        return Ok(PhaseSample {
            elapsed_ns,
            ..PhaseSample::default()
        });
    }
    TRACKING.store(false, Ordering::Relaxed);
    let current_live = LIVE_BYTES.load(Ordering::Relaxed);
    let peak_live = PEAK_LIVE_BYTES.load(Ordering::Relaxed);
    let cage_after = if compact {
        Some(CompactRuntime::allocator_stats()?)
    } else {
        None
    };
    let (cage_live_delta, cage_cursor, free_bytes, free_blocks, largest_free_block) =
        match (start.cage_before, cage_after) {
            (Some(before), Some(after)) => (
                after.live_bytes as i64 - before.live_bytes as i64,
                after.high_water_cursor,
                after.free_bytes,
                after.free_blocks,
                after.largest_free_block,
            ),
            _ => (0, 0, 0, 0, 0),
        };
    let (
        allocator_lock_acquisitions,
        free_list_nodes_visited,
        release_batches,
        released_extents,
        max_release_batch,
        size_class_hits,
        size_class_misses,
    ) = match (start.cage_before, cage_after) {
        (Some(before), Some(after)) => (
            after
                .lock_acquisitions
                .saturating_sub(before.lock_acquisitions)
                .saturating_sub(1),
            after
                .free_list_nodes_visited
                .saturating_sub(before.free_list_nodes_visited),
            after.release_batches.saturating_sub(before.release_batches),
            after
                .released_extents
                .saturating_sub(before.released_extents),
            after.max_release_batch,
            after.size_class_hits.saturating_sub(before.size_class_hits),
            after
                .size_class_misses
                .saturating_sub(before.size_class_misses),
        ),
        _ => (0, 0, 0, 0, 0, 0, 0),
    };
    let (allocator_before, allocator_after) = match (start.cage_before, cage_after) {
        (Some(before), Some(after)) => (before, after),
        _ => (AllocatorStats::default(), AllocatorStats::default()),
    };
    Ok(PhaseSample {
        elapsed_ns,
        allocation_calls: ALLOC_CALLS.load(Ordering::Relaxed),
        deallocation_calls: DEALLOC_CALLS.load(Ordering::Relaxed),
        requested_bytes: REQUESTED_BYTES.load(Ordering::Relaxed),
        live_delta: current_live as i64 - start.baseline_live as i64,
        peak_extra: peak_live.saturating_sub(start.baseline_live),
        cage_live_delta,
        cage_cursor,
        free_bytes,
        free_blocks,
        largest_free_block,
        allocator_lock_acquisitions,
        free_list_nodes_visited,
        release_batches,
        released_extents,
        max_release_batch,
        size_class_hits,
        size_class_misses,
        pending_reuse_hits: allocator_after
            .pending_reuse_hits
            .saturating_sub(allocator_before.pending_reuse_hits),
        pending_reuse_misses: allocator_after
            .pending_reuse_misses
            .saturating_sub(allocator_before.pending_reuse_misses),
        pending_reuse_no_active_collector: allocator_after
            .pending_reuse_no_active_collector
            .saturating_sub(allocator_before.pending_reuse_no_active_collector),
        pending_reuse_no_exact_block: allocator_after
            .pending_reuse_no_exact_block
            .saturating_sub(allocator_before.pending_reuse_no_exact_block),
        pending_reuse_alignment_incompatible: allocator_after
            .pending_reuse_alignment_incompatible
            .saturating_sub(allocator_before.pending_reuse_alignment_incompatible),
        global_class_hits: core::array::from_fn(|index| {
            allocator_after.global_class_hits[index]
                .saturating_sub(allocator_before.global_class_hits[index])
        }),
        global_class_misses: core::array::from_fn(|index| {
            allocator_after.global_class_misses[index]
                .saturating_sub(allocator_before.global_class_misses[index])
        }),
        global_class_empty: core::array::from_fn(|index| {
            allocator_after.global_class_empty[index]
                .saturating_sub(allocator_before.global_class_empty[index])
        }),
        global_class_alignment_incompatible: core::array::from_fn(|index| {
            allocator_after.global_class_alignment_incompatible[index]
                .saturating_sub(allocator_before.global_class_alignment_incompatible[index])
        }),
        requested_size_no_class: allocator_after
            .requested_size_no_class
            .saturating_sub(allocator_before.requested_size_no_class),
        general_list_fallbacks: allocator_after
            .general_list_fallbacks
            .saturating_sub(allocator_before.general_list_fallbacks),
        cursor_fallbacks: allocator_after
            .cursor_fallbacks
            .saturating_sub(allocator_before.cursor_fallbacks),
        released_exact_size_extents: allocator_after
            .released_exact_size_extents
            .saturating_sub(allocator_before.released_exact_size_extents),
        exact_size_extents_cached: allocator_after
            .exact_size_extents_cached
            .saturating_sub(allocator_before.exact_size_extents_cached),
        exact_size_extents_coalesced_before_cache: allocator_after
            .exact_size_extents_coalesced_before_cache
            .saturating_sub(allocator_before.exact_size_extents_coalesced_before_cache),
    })
}

fn aggregate(name: &'static str, samples: Vec<PhaseSample>) -> PhaseAggregate {
    let mut times: Vec<u128> = samples.iter().map(|sample| sample.elapsed_ns).collect();
    times.sort_unstable();
    let median_ns = median_u128(&times);
    let p95_ns = times[((times.len() * 95).div_ceil(100)).saturating_sub(1)];
    PhaseAggregate {
        name,
        runs: samples.len(),
        median_ns,
        p95_ns,
        min_ns: *times.first().unwrap_or(&0),
        max_ns: *times.last().unwrap_or(&0),
        allocation_calls: median_u64(samples.iter().map(|sample| sample.allocation_calls)),
        deallocation_calls: median_u64(samples.iter().map(|sample| sample.deallocation_calls)),
        requested_bytes: median_u64(samples.iter().map(|sample| sample.requested_bytes)),
        live_delta: median_i64(samples.iter().map(|sample| sample.live_delta)),
        peak_extra: samples
            .iter()
            .map(|sample| sample.peak_extra)
            .max()
            .unwrap_or(0),
        cage_live_delta: median_i64(samples.iter().map(|sample| sample.cage_live_delta)),
        cage_cursor: samples
            .iter()
            .map(|sample| sample.cage_cursor)
            .max()
            .unwrap_or(0),
        free_bytes: median_u64(samples.iter().map(|sample| sample.free_bytes as u64)) as u32,
        free_blocks: median_u64(samples.iter().map(|sample| sample.free_blocks as u64)) as u32,
        largest_free_block: median_u64(
            samples
                .iter()
                .map(|sample| sample.largest_free_block as u64),
        ) as u32,
        allocator_lock_acquisitions: median_u64(
            samples
                .iter()
                .map(|sample| sample.allocator_lock_acquisitions),
        ),
        free_list_nodes_visited: median_u64(
            samples.iter().map(|sample| sample.free_list_nodes_visited),
        ),
        release_batches: median_u64(samples.iter().map(|sample| sample.release_batches)),
        released_extents: median_u64(samples.iter().map(|sample| sample.released_extents)),
        max_release_batch: samples
            .iter()
            .map(|sample| sample.max_release_batch)
            .max()
            .unwrap_or(0),
        size_class_hits: median_u64(samples.iter().map(|sample| sample.size_class_hits)),
        size_class_misses: median_u64(samples.iter().map(|sample| sample.size_class_misses)),
        pending_reuse_hits: median_u64(samples.iter().map(|sample| sample.pending_reuse_hits)),
        pending_reuse_misses: median_u64(samples.iter().map(|sample| sample.pending_reuse_misses)),
        pending_reuse_no_active_collector: median_u64(
            samples
                .iter()
                .map(|sample| sample.pending_reuse_no_active_collector),
        ),
        pending_reuse_no_exact_block: median_u64(
            samples
                .iter()
                .map(|sample| sample.pending_reuse_no_exact_block),
        ),
        pending_reuse_alignment_incompatible: median_u64(
            samples
                .iter()
                .map(|sample| sample.pending_reuse_alignment_incompatible),
        ),
        global_class_hits: core::array::from_fn(|index| {
            median_u64(samples.iter().map(|sample| sample.global_class_hits[index]))
        }),
        global_class_misses: core::array::from_fn(|index| {
            median_u64(
                samples
                    .iter()
                    .map(|sample| sample.global_class_misses[index]),
            )
        }),
        global_class_empty: core::array::from_fn(|index| {
            median_u64(
                samples
                    .iter()
                    .map(|sample| sample.global_class_empty[index]),
            )
        }),
        global_class_alignment_incompatible: core::array::from_fn(|index| {
            median_u64(
                samples
                    .iter()
                    .map(|sample| sample.global_class_alignment_incompatible[index]),
            )
        }),
        requested_size_no_class: median_u64(
            samples.iter().map(|sample| sample.requested_size_no_class),
        ),
        general_list_fallbacks: median_u64(
            samples.iter().map(|sample| sample.general_list_fallbacks),
        ),
        cursor_fallbacks: median_u64(samples.iter().map(|sample| sample.cursor_fallbacks)),
        released_exact_size_extents: median_u64(
            samples
                .iter()
                .map(|sample| sample.released_exact_size_extents),
        ),
        exact_size_extents_cached: median_u64(
            samples
                .iter()
                .map(|sample| sample.exact_size_extents_cached),
        ),
        exact_size_extents_coalesced_before_cache: median_u64(
            samples
                .iter()
                .map(|sample| sample.exact_size_extents_coalesced_before_cache),
        ),
    }
}

fn median_u128(values: &[u128]) -> u128 {
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2
    } else {
        values[middle]
    }
}

fn median_u64(values: impl Iterator<Item = u64>) -> u64 {
    let mut values: Vec<u64> = values.collect();
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2
    } else {
        values[middle]
    }
}

fn median_i64(values: impl Iterator<Item = i64>) -> i64 {
    let mut values: Vec<i64> = values.collect();
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2
    } else {
        values[middle]
    }
}

fn emit_phase(scenario: &str, variant: &str, summary: PhaseAggregate) {
    let class_hits = summary
        .global_class_hits
        .map(|value| value.to_string())
        .join(",");
    let class_misses = summary
        .global_class_misses
        .map(|value| value.to_string())
        .join(",");
    let class_empty = summary
        .global_class_empty
        .map(|value| value.to_string())
        .join(",");
    let class_alignment = summary
        .global_class_alignment_incompatible
        .map(|value| value.to_string())
        .join(",");
    let fields = [
        summary.name.to_owned(),
        summary.runs.to_string(),
        summary.median_ns.to_string(),
        summary.p95_ns.to_string(),
        summary.min_ns.to_string(),
        summary.max_ns.to_string(),
        summary.allocation_calls.to_string(),
        summary.deallocation_calls.to_string(),
        summary.requested_bytes.to_string(),
        summary.live_delta.to_string(),
        summary.peak_extra.to_string(),
        summary.cage_live_delta.to_string(),
        summary.cage_cursor.to_string(),
        summary.free_bytes.to_string(),
        summary.free_blocks.to_string(),
        summary.largest_free_block.to_string(),
        summary.allocator_lock_acquisitions.to_string(),
        summary.free_list_nodes_visited.to_string(),
        summary.release_batches.to_string(),
        summary.released_extents.to_string(),
        summary.max_release_batch.to_string(),
        summary.size_class_hits.to_string(),
        summary.size_class_misses.to_string(),
        summary.pending_reuse_hits.to_string(),
        summary.pending_reuse_misses.to_string(),
        summary.pending_reuse_no_active_collector.to_string(),
        summary.pending_reuse_no_exact_block.to_string(),
        summary.pending_reuse_alignment_incompatible.to_string(),
        class_hits,
        class_misses,
        class_empty,
        class_alignment,
        summary.requested_size_no_class.to_string(),
        summary.general_list_fallbacks.to_string(),
        summary.cursor_fallbacks.to_string(),
        summary.released_exact_size_extents.to_string(),
        summary.exact_size_extents_cached.to_string(),
        summary
            .exact_size_extents_coalesced_before_cache
            .to_string(),
    ];
    println!("PHASE\t{scenario}\t{variant}\t{}", fields.join("\t"));
}

fn peak_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmHWM:")?.split_whitespace().next()?;
        value.parse().ok()
    })
}
