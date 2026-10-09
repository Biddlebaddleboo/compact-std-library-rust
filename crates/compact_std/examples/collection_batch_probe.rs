//! Round-four PLAN_COLLECTION_ACCESS diagnostic: fair batched-vs-per-op study.
//!
//! The shared-shape A4 arm matches its full 4,096-entry starting ring, including
//! timed growth for both compact per-op and compact view paths. A separate
//! oversized-reservation study preserves the earlier storage-hoisting floor,
//! granularity sweep, and compact API comparison. B6 measures the indexed update
//! phase on the shared 2,048-level input, with per-op and slice-borrow variants.
//!
//! Numbers are attribution evidence only; `benchmark_compare` remains the timing
//! authority. This harness is standalone and installs no counting allocator.

use compact_std::{CageConfig, CompactRuntime, CompactValue, CompactVec, CompactVecDeque};
use std::collections::VecDeque;
use std::error::Error;
use std::hint::black_box;
use std::time::Instant;

const CAGE_BYTES: usize = 128 * 1024 * 1024;
const DEQUE_POPULATION: usize = 4_096;
const A4_SHARED_CAPACITY: usize = DEQUE_POPULATION;
const A4_PRE_RESERVED_REFERENCE_CAPACITY: usize = 8_192;
const A4_PRE_RESERVED_HEAD_SHIFT_OPS: usize =
    A4_PRE_RESERVED_REFERENCE_CAPACITY - DEQUE_POPULATION / 2;
const DEQUE_OPERATIONS: u64 = 80_000;
/// Existing oversized reservation for the API study. Lockstep churn peaks at
/// population + 1, so this capacity intentionally differs from shared A4.
const A4_PRE_RESERVED_CAPACITY: usize = DEQUE_POPULATION + DEQUE_OPERATIONS as usize + 8;
const BOOK_LEVELS_TOTAL: usize = 2_048;
const BOOK_SIDE_LEVELS: usize = BOOK_LEVELS_TOTAL / 2;
const BOOK_ROUNDS: usize = 8;
const UPDATES_PER_ROUND: usize = 192;

type ProbeResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone, Copy)]
struct ProbeSample {
    elapsed_ns: u128,
    checksum: u64,
}

#[derive(Clone, Copy)]
struct PriceLevel {
    price_micros: u64,
    quantity: u64,
    order_count: u32,
    flags: u32,
}

// SAFETY: `PriceLevel` is a plain copyable scalar record.
unsafe impl CompactValue for PriceLevel {}

fn b6_seed() -> (Vec<PriceLevel>, Vec<PriceLevel>) {
    let level_data = (0..BOOK_LEVELS_TOTAL)
        .map(|index| PriceLevel {
            price_micros: 50_000_000 + index as u64 * 10_000,
            quantity: 100 + (index as u64 * 53 % 50_000),
            order_count: 1 + (index as u32 % 32),
            flags: index as u32 & 3,
        })
        .collect::<Vec<_>>();
    let midpoint = level_data.len() / 2;
    let bids = level_data[..midpoint].iter().rev().copied().collect();
    let asks = level_data[midpoint..].to_vec();
    (bids, asks)
}

fn summarize(scenario: &str, label: &str, mut samples: Vec<ProbeSample>) -> ProbeResult<u64> {
    if samples.is_empty() {
        return Err(format!("{scenario}/{label} produced no samples").into());
    }
    samples.sort_unstable_by_key(|sample| sample.elapsed_ns);
    let checksum = samples[0].checksum;
    if samples.iter().any(|sample| sample.checksum != checksum) {
        return Err(format!("{scenario}/{label} produced inconsistent checksums").into());
    }
    let median = samples[samples.len() / 2].elapsed_ns;
    // Nearest-rank p95: rank is ceil(0.95 * n), converted to zero-based index.
    let p95_rank = (samples.len() * 95).div_ceil(100);
    let p95 = samples[p95_rank - 1].elapsed_ns;
    println!(
        "PROBE\t{scenario}\t{label}\t{}\t{median}\t{p95}\t{checksum}",
        samples.len(),
    );
    Ok(checksum)
}

fn assert_same_checksum(
    scenario: &str,
    reference_label: &str,
    reference: u64,
    candidate_label: &str,
    candidate: u64,
) -> ProbeResult<()> {
    if reference != candidate {
        return Err(format!(
            "{scenario} checksum mismatch: {reference_label}={reference}, {candidate_label}={candidate}"
        )
        .into());
    }
    Ok(())
}

fn arg<'a>(arguments: &'a [String], name: &str) -> ProbeResult<&'a str> {
    arguments
        .iter()
        .position(|argument| argument == name)
        .and_then(|index| arguments.get(index + 1))
        .map(String::as_str)
        .ok_or_else(|| format!("missing {name}").into())
}

// ---------------------------------------------------------------------------
// A4 FIFO churn: push_back + pop_front in lockstep.
//
// The shared-shape arm times reserve(1) before borrowing, matching the growth
// triggered by the full ring's first per-op push. The preserved API study below
// uses an oversized ring; all paired paths share its capacity and head shift.
// ---------------------------------------------------------------------------

fn a4_prepare_native(capacity: usize, shift: usize) -> VecDeque<u64> {
    let mut queue: VecDeque<u64> = VecDeque::with_capacity(capacity);
    for value in 0..DEQUE_POPULATION as u64 {
        queue.push_back(value);
    }
    for value in 0..shift as u64 {
        let moved = queue.pop_front().expect("prepared queue is nonempty");
        queue.push_back(moved.wrapping_add(value));
    }
    queue
}

/// Build a compact ring at an exact capacity and rotate it through `shift` FIFO
/// operations. Constructing at final capacity preserves the requested geometry.
fn a4_prepare_compact(capacity: usize, shift: usize) -> ProbeResult<CompactVecDeque<u64>> {
    let mut queue = CompactVecDeque::with_capacity(capacity)?;
    for value in 0..DEQUE_POPULATION as u64 {
        queue.push_back(value)?;
    }
    for value in 0..shift as u64 {
        let moved = queue.pop_front().expect("prepared queue is nonempty");
        queue.push_back(moved.wrapping_add(value))?;
    }
    Ok(queue)
}

fn a4_native_perop(runs: usize, capacity: usize, shift: usize) -> Vec<ProbeSample> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let mut queue = a4_prepare_native(capacity, shift);
        let started = Instant::now();
        let mut checksum = 0_u64;
        for value in 0..DEQUE_OPERATIONS {
            queue.push_back(black_box(DEQUE_POPULATION as u64 + value));
            checksum = checksum.wrapping_add(black_box(
                queue.pop_front().expect("FIFO queue remains populated"),
            ));
        }
        let elapsed_ns = started.elapsed().as_nanos();
        black_box(checksum);
        samples.push(ProbeSample {
            elapsed_ns,
            checksum,
        });
    }
    samples
}

fn a4_compact_perop(runs: usize, capacity: usize, shift: usize) -> ProbeResult<Vec<ProbeSample>> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let mut queue = a4_prepare_compact(capacity, shift)?;
        let started = Instant::now();
        let mut checksum = 0_u64;
        for value in 0..DEQUE_OPERATIONS {
            queue.push_back(black_box(DEQUE_POPULATION as u64 + value))?;
            checksum = checksum.wrapping_add(black_box(
                queue.pop_front().expect("FIFO queue remains populated"),
            ));
        }
        let elapsed_ns = started.elapsed().as_nanos();
        black_box(checksum);
        samples.push(ProbeSample {
            elapsed_ns,
            checksum,
        });
    }
    Ok(samples)
}

fn a4_compact_batched(runs: usize, capacity: usize, shift: usize) -> ProbeResult<Vec<ProbeSample>> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let mut queue = a4_prepare_compact(capacity, shift)?;
        let started = Instant::now();
        let checksum = queue.with_view(|ring| -> ProbeResult<u64> {
            let mut checksum = 0_u64;
            for value in 0..DEQUE_OPERATIONS {
                ring.push_back(black_box(DEQUE_POPULATION as u64 + value))?;
                checksum = checksum.wrapping_add(black_box(
                    ring.pop_front().expect("FIFO queue remains populated"),
                ));
            }
            Ok(checksum)
        })?;
        let elapsed_ns = started.elapsed().as_nanos();
        black_box(checksum);
        samples.push(ProbeSample {
            elapsed_ns,
            checksum,
        });
    }
    Ok(samples)
}

/// Match the shared A4 pre-state: a full 4,096-slot ring at head zero. The
/// timed reserve mirrors the growth caused by the first per-op push, then the
/// view runs the same 80,000 push/pop pairs.
fn a4_compact_batched_shared_shape(runs: usize) -> ProbeResult<Vec<ProbeSample>> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let mut queue = a4_prepare_compact(A4_SHARED_CAPACITY, 0)?;
        let started = Instant::now();
        queue.reserve(1)?;
        let checksum = queue.with_view(|ring| -> ProbeResult<u64> {
            let mut checksum = 0_u64;
            for value in 0..DEQUE_OPERATIONS {
                ring.push_back(black_box(DEQUE_POPULATION as u64 + value))?;
                checksum = checksum.wrapping_add(black_box(
                    ring.pop_front().expect("FIFO queue remains populated"),
                ));
            }
            Ok(checksum)
        })?;
        let elapsed_ns = started.elapsed().as_nanos();
        black_box(checksum);
        samples.push(ProbeSample {
            elapsed_ns,
            checksum,
        });
    }
    Ok(samples)
}

/// A native ring with the same head/len bookkeeping as the compact deque but
/// plain `Vec` storage, so it isolates storage-hoisting from cage-header cost.
struct NativeRing {
    slots: Vec<u64>,
    head: usize,
    len: usize,
}

impl NativeRing {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: vec![0; capacity],
            head: 0,
            len: 0,
        }
    }
    fn push_back(&mut self, value: u64) {
        let until_wrap = self.slots.len() - self.head;
        let at = if self.len >= until_wrap {
            self.len - until_wrap
        } else {
            self.head + self.len
        };
        self.slots[at] = value;
        self.len += 1;
    }
    fn pop_front(&mut self) -> Option<u64> {
        if self.len == 0 {
            return None;
        }
        let at = self.head;
        let value = self.slots[at];
        self.head = if self.len == 1 || at + 1 == self.slots.len() {
            0
        } else {
            at + 1
        };
        self.len -= 1;
        Some(value)
    }
    /// Resolve the slot storage once, mirroring the compact borrow-scoped view.
    fn batched_churn(&mut self, operations: u64, base: u64) -> u64 {
        let slots = &mut self.slots[..];
        let capacity = slots.len();
        let mut head = self.head;
        let mut len = self.len;
        let mut checksum = 0_u64;
        for value in 0..operations {
            let until_wrap = capacity - head;
            let at = if len >= until_wrap {
                len - until_wrap
            } else {
                head + len
            };
            slots[at] = base + value;
            len += 1;
            let popped = slots[head];
            head = if len == 1 || head + 1 == capacity {
                0
            } else {
                head + 1
            };
            len -= 1;
            checksum = checksum.wrapping_add(popped);
        }
        self.head = head;
        self.len = len;
        checksum
    }
}

fn a4_native_batched(runs: usize, capacity: usize, shift: usize) -> Vec<ProbeSample> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let mut ring = NativeRing::with_capacity(capacity);
        for value in 0..DEQUE_POPULATION as u64 {
            ring.push_back(value);
        }
        for value in 0..shift as u64 {
            let moved = ring.pop_front().expect("prepared ring is nonempty");
            ring.push_back(moved.wrapping_add(value));
        }
        let started = Instant::now();
        let checksum = ring.batched_churn(DEQUE_OPERATIONS, DEQUE_POPULATION as u64);
        let elapsed_ns = started.elapsed().as_nanos();
        black_box(checksum);
        samples.push(ProbeSample {
            elapsed_ns,
            checksum,
        });
    }
    samples
}

/// Re-borrow a fresh compact view every `batch` operations: the practical
/// tradeoff between resolution overhead and borrow-scope size.
fn a4_compact_batched_granularity(
    runs: usize,
    capacity: usize,
    shift: usize,
    batch: u64,
) -> ProbeResult<Vec<ProbeSample>> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let mut queue = a4_prepare_compact(capacity, shift)?;
        let started = Instant::now();
        let checksum = queue.with_view(|ring| -> ProbeResult<u64> {
            let mut checksum = 0_u64;
            let mut remaining = DEQUE_OPERATIONS;
            let mut next = DEQUE_POPULATION as u64;
            while remaining > 0 {
                let step = batch.min(remaining);
                for _ in 0..step {
                    ring.push_back(black_box(next))?;
                    checksum = checksum.wrapping_add(black_box(
                        ring.pop_front().expect("FIFO queue remains populated"),
                    ));
                    next += 1;
                }
                remaining -= step;
            }
            Ok(checksum)
        })?;
        let elapsed_ns = started.elapsed().as_nanos();
        black_box(checksum);
        samples.push(ProbeSample {
            elapsed_ns,
            checksum,
        });
    }
    Ok(samples)
}

// ---------------------------------------------------------------------------
// B6 order-book update pattern: random indexed updates each round.
// ---------------------------------------------------------------------------

fn update_level(level: &mut PriceLevel, update: usize, modulus: usize, span: u64) {
    level.quantity = if update % modulus == 0 {
        0
    } else {
        level
            .quantity
            .saturating_sub(1 + (update % span as usize) as u64)
    };
    level.order_count = level
        .order_count
        .saturating_sub(if update % modulus == 0 { 1 } else { 0 });
}

fn b6_native_perop(runs: usize) -> Vec<ProbeSample> {
    let (bid_seed, ask_seed) = b6_seed();
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let mut bids = bid_seed.clone();
        let mut asks = ask_seed.clone();
        let started = Instant::now();
        for round in 0..BOOK_ROUNDS {
            for update in 0..UPDATES_PER_ROUND {
                let index = (round * 137 + update * 31) % bids.len();
                update_level(&mut bids[index], update, 11, 13);
                let ask_index = (round * 73 + update * 17) % asks.len();
                update_level(&mut asks[ask_index], update, 13, 9);
            }
        }
        let elapsed_ns = started.elapsed().as_nanos();
        let checksum = book_checksum(&bids, &asks);
        black_box(checksum);
        samples.push(ProbeSample {
            elapsed_ns,
            checksum,
        });
    }
    samples
}

fn b6_native_batched(runs: usize) -> Vec<ProbeSample> {
    let (bid_seed, ask_seed) = b6_seed();
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let mut bids = bid_seed.clone();
        let mut asks = ask_seed.clone();
        let started = Instant::now();
        for round in 0..BOOK_ROUNDS {
            {
                let bid_levels = bids.as_mut_slice();
                let ask_levels = asks.as_mut_slice();
                for update in 0..UPDATES_PER_ROUND {
                    let index = (round * 137 + update * 31) % bid_levels.len();
                    update_level(&mut bid_levels[index], update, 11, 13);
                    let ask_index = (round * 73 + update * 17) % ask_levels.len();
                    update_level(&mut ask_levels[ask_index], update, 13, 9);
                }
            }
        }
        let elapsed_ns = started.elapsed().as_nanos();
        let checksum = book_checksum(&bids, &asks);
        black_box(checksum);
        samples.push(ProbeSample {
            elapsed_ns,
            checksum,
        });
    }
    samples
}

fn b6_prepare_compact() -> ProbeResult<(CompactVec<PriceLevel>, CompactVec<PriceLevel>)> {
    let (bid_seed, ask_seed) = b6_seed();
    let mut bids = CompactVec::with_capacity(BOOK_SIDE_LEVELS)?;
    for level in &bid_seed {
        bids.push(*level)?;
    }
    let mut asks = CompactVec::with_capacity(BOOK_SIDE_LEVELS)?;
    for level in &ask_seed {
        asks.push(*level)?;
    }
    Ok((bids, asks))
}

fn b6_compact_perop(runs: usize) -> ProbeResult<Vec<ProbeSample>> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let (mut bids, mut asks) = b6_prepare_compact()?;
        let started = Instant::now();
        for round in 0..BOOK_ROUNDS {
            for update in 0..UPDATES_PER_ROUND {
                let index = (round * 137 + update * 31) % bids.len();
                update_level(&mut bids[index], update, 11, 13);
                let ask_index = (round * 73 + update * 17) % asks.len();
                update_level(&mut asks[ask_index], update, 13, 9);
            }
        }
        let elapsed_ns = started.elapsed().as_nanos();
        let checksum = compact_book_checksum(&bids, &asks);
        black_box(checksum);
        samples.push(ProbeSample {
            elapsed_ns,
            checksum,
        });
    }
    Ok(samples)
}

fn b6_compact_batched(runs: usize) -> ProbeResult<Vec<ProbeSample>> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let (mut bids, mut asks) = b6_prepare_compact()?;
        let started = Instant::now();
        for round in 0..BOOK_ROUNDS {
            {
                let bid_levels = bids.as_mut_slice();
                let ask_levels = asks.as_mut_slice();
                for update in 0..UPDATES_PER_ROUND {
                    let index = (round * 137 + update * 31) % bid_levels.len();
                    update_level(&mut bid_levels[index], update, 11, 13);
                    let ask_index = (round * 73 + update * 17) % ask_levels.len();
                    update_level(&mut ask_levels[ask_index], update, 13, 9);
                }
            }
        }
        let elapsed_ns = started.elapsed().as_nanos();
        let checksum = compact_book_checksum(&bids, &asks);
        black_box(checksum);
        samples.push(ProbeSample {
            elapsed_ns,
            checksum,
        });
    }
    Ok(samples)
}

fn book_checksum(bids: &[PriceLevel], asks: &[PriceLevel]) -> u64 {
    bids.iter()
        .chain(asks)
        .map(|level| {
            level.price_micros
                ^ level.quantity
                ^ u64::from(level.order_count)
                ^ u64::from(level.flags)
        })
        .fold(0_u64, u64::wrapping_add)
}

fn compact_book_checksum(bids: &CompactVec<PriceLevel>, asks: &CompactVec<PriceLevel>) -> u64 {
    book_checksum(bids.as_slice(), asks.as_slice())
}

fn main() -> ProbeResult<()> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let scenario = arg(&arguments, "--scenario")?.to_owned();
    let runs = arg(&arguments, "--runs")?.parse::<usize>()?;
    if runs < 2 {
        return Err("--runs must be at least 2".into());
    }
    CompactRuntime::init(CageConfig::new(CAGE_BYTES))?;
    println!("PROBE\tscenario\tlabel\truns\tmedian_ns\tp95_ns\tchecksum");
    match scenario.as_str() {
        "A4" => {
            println!(
                "META\tA4\tshared_shape\tpopulation={DEQUE_POPULATION}\tinitial_capacity={A4_SHARED_CAPACITY}\tinitial_head=0\toperations={DEQUE_OPERATIONS}\tmax_live_len={}",
                DEQUE_POPULATION + 1,
            );
            let shared_native = summarize(
                &scenario,
                "A4.shared_shape.native.per_op",
                a4_native_perop(runs, A4_SHARED_CAPACITY, 0),
            )?;
            let shared_compact = summarize(
                &scenario,
                "A4.shared_shape.compact.per_op",
                a4_compact_perop(runs, A4_SHARED_CAPACITY, 0)?,
            )?;
            assert_same_checksum(
                &scenario,
                "A4.shared_shape.native.per_op",
                shared_native,
                "A4.shared_shape.compact.per_op",
                shared_compact,
            )?;
            let shared_batched = summarize(
                &scenario,
                "A4.shared_shape.compact.batch_view_timed_reserve",
                a4_compact_batched_shared_shape(runs)?,
            )?;
            assert_same_checksum(
                &scenario,
                "A4.shared_shape.compact.per_op",
                shared_compact,
                "A4.shared_shape.compact.batch_view_timed_reserve",
                shared_batched,
            )?;

            let shift = A4_PRE_RESERVED_HEAD_SHIFT_OPS;
            println!(
                "META\tA4\tinitial_full_ring_study\tpopulation={DEQUE_POPULATION}\tinitial_capacity={A4_SHARED_CAPACITY}\tpre_shift_operations={shift}\toperations={DEQUE_OPERATIONS}\tfirst_push_growth_in_timed_region=true",
            );
            let shifted_native = summarize(
                &scenario,
                "A4.initial_full_ring.native.per_op",
                a4_native_perop(runs, A4_SHARED_CAPACITY, shift),
            )?;
            let shifted_compact = summarize(
                &scenario,
                "A4.initial_full_ring.compact.per_op",
                a4_compact_perop(runs, A4_SHARED_CAPACITY, shift)?,
            )?;
            assert_same_checksum(
                &scenario,
                "A4.initial_full_ring.native.per_op",
                shifted_native,
                "A4.initial_full_ring.compact.per_op",
                shifted_compact,
            )?;

            let reserved_prefix =
                format!("A4.pre_reserved_api.capacity_{}", A4_PRE_RESERVED_CAPACITY);
            println!(
                "META\tA4\tpre_reserved_api_study\tpopulation={DEQUE_POPULATION}\tcapacity={A4_PRE_RESERVED_CAPACITY}\tpre_shift_operations={shift}\toperations={DEQUE_OPERATIONS}\tmax_live_len={}\tspare_slots_at_peak={}",
                DEQUE_POPULATION + 1,
                A4_PRE_RESERVED_CAPACITY - (DEQUE_POPULATION + 1),
            );
            let reserved_native = summarize(
                &scenario,
                &format!("{reserved_prefix}.native.per_op"),
                a4_native_perop(runs, A4_PRE_RESERVED_CAPACITY, shift),
            )?;
            let reserved_compact = summarize(
                &scenario,
                &format!("{reserved_prefix}.compact.per_op"),
                a4_compact_perop(runs, A4_PRE_RESERVED_CAPACITY, shift)?,
            )?;
            let reserved_native_batch = summarize(
                &scenario,
                &format!("{reserved_prefix}.native.batch_storage_floor"),
                a4_native_batched(runs, A4_PRE_RESERVED_CAPACITY, shift),
            )?;
            let reserved_compact_batch = summarize(
                &scenario,
                &format!("{reserved_prefix}.compact.batch_view"),
                a4_compact_batched(runs, A4_PRE_RESERVED_CAPACITY, shift)?,
            )?;
            assert_same_checksum(
                &scenario,
                &format!("{reserved_prefix}.native.per_op"),
                reserved_native,
                &format!("{reserved_prefix}.compact.per_op"),
                reserved_compact,
            )?;
            assert_same_checksum(
                &scenario,
                &format!("{reserved_prefix}.native.per_op"),
                reserved_native,
                &format!("{reserved_prefix}.native.batch_storage_floor"),
                reserved_native_batch,
            )?;
            assert_same_checksum(
                &scenario,
                &format!("{reserved_prefix}.native.per_op"),
                reserved_native,
                &format!("{reserved_prefix}.compact.batch_view"),
                reserved_compact_batch,
            )?;
            // Batching granularity: reopen a fresh borrow every N operations.
            for batch in [16_u64, 64, 256, 1024, 8192] {
                let label = format!("{reserved_prefix}.compact.batch_every_{batch}");
                let batch_checksum = summarize(
                    &scenario,
                    &label,
                    a4_compact_batched_granularity(runs, A4_PRE_RESERVED_CAPACITY, shift, batch)?,
                )?;
                assert_same_checksum(
                    &scenario,
                    &format!("{reserved_prefix}.native.per_op"),
                    reserved_native,
                    &label,
                    batch_checksum,
                )?;
            }
        }
        "B6" => {
            println!(
                "META\tB6\tupdates_only\tlevels_total={BOOK_LEVELS_TOTAL}\tlevels_per_side={BOOK_SIDE_LEVELS}\trounds={BOOK_ROUNDS}\tupdates_per_round={UPDATES_PER_ROUND}\tinput_split=first_half_reversed_bids_second_half_asks\tretain_and_clone_excluded=true",
            );
            let native = summarize(
                &scenario,
                "B6.shared_shape.native.per_op",
                b6_native_perop(runs),
            )?;
            let native_batch = summarize(
                &scenario,
                "B6.shared_shape.native.batch_slice",
                b6_native_batched(runs),
            )?;
            let compact = summarize(
                &scenario,
                "B6.shared_shape.compact.per_op",
                b6_compact_perop(runs)?,
            )?;
            let compact_batch = summarize(
                &scenario,
                "B6.shared_shape.compact.batch_slice",
                b6_compact_batched(runs)?,
            )?;
            assert_same_checksum(
                &scenario,
                "B6.shared_shape.native.per_op",
                native,
                "B6.shared_shape.native.batch_slice",
                native_batch,
            )?;
            assert_same_checksum(
                &scenario,
                "B6.shared_shape.native.per_op",
                native,
                "B6.shared_shape.compact.per_op",
                compact,
            )?;
            assert_same_checksum(
                &scenario,
                "B6.shared_shape.native.per_op",
                native,
                "B6.shared_shape.compact.batch_slice",
                compact_batch,
            )?;
        }
        _ => return Err("--scenario must be A4 or B6".into()),
    }
    Ok(())
}
