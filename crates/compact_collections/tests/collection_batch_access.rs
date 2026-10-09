//! PLAN_COLLECTION_ACCESS (v2.4 round four): batched borrow-scoped access must
//! behave exactly like ordinary per-operation access for the A4 FIFO-churn and
//! B6 order-book caller patterns, including when every unsafe slot access is
//! checked by Miri.
//!
//! These tests deliberately exercise the *caller* shapes the round-four probe
//! measures, so a regression in either the per-op path or the borrow-scoped
//! path is caught by a differential comparison rather than by a golden value.

use compact_backend_std::{CageConfig, CompactRuntime};
use compact_collections::{CompactVec, CompactVecDeque};
use compact_core::CompactValue;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

static INIT: OnceLock<()> = OnceLock::new();

fn init() {
    INIT.get_or_init(|| CompactRuntime::init(CageConfig::new(64 * 1024 * 1024)).unwrap());
}

fn smaller(miri_value: usize, native_value: usize) -> usize {
    if cfg!(miri) {
        miri_value
    } else {
        native_value
    }
}

fn contents(queue: &CompactVecDeque<u64>) -> Vec<u64> {
    queue.iter().copied().collect()
}

// ---------------------------------------------------------------------------
// A4: FIFO churn (push_back then pop_front in lockstep).
// ---------------------------------------------------------------------------

/// Drive one FIFO churn phase. `batched` selects the borrow-scoped view; the
/// caller must have reserved enough capacity when it is set.
fn compact_fifo_churn(queue: &mut CompactVecDeque<u64>, operations: u64, base: u64, batched: bool) {
    if batched {
        queue
            .with_view(|ring| -> Result<(), compact_collections::CollectionError> {
                for value in 0..operations {
                    ring.push_back(base + value)?;
                    let popped = ring.pop_front().expect("FIFO queue remains populated");
                    assert!(popped < base + value);
                }
                Ok(())
            })
            .expect("reserved view never needs to grow");
    } else {
        for value in 0..operations {
            queue.push_back(base + value).unwrap();
            let popped = queue.pop_front().expect("FIFO queue remains populated");
            assert!(popped < base + value);
        }
    }
}

#[test]
fn fifo_churn_view_matches_per_op_and_native_through_wrap() {
    init();

    let population = smaller(64, 4_096);
    let operations = smaller(256, 80_000) as u64;
    let capacity = population + operations as usize + 8;
    // Start the ring part-way through its storage so the churn crosses the wrap
    // boundary, matching the wrapped scenario in the shared harness.
    let shift = population / 2;

    let mut native: VecDeque<u64> = VecDeque::with_capacity(capacity);
    let mut per_op = CompactVecDeque::with_capacity(capacity).unwrap();
    let mut batched = CompactVecDeque::with_capacity(capacity).unwrap();
    for value in 0..population as u64 {
        native.push_back(value);
        per_op.push_back(value).unwrap();
        batched.push_back(value).unwrap();
    }
    for value in 0..shift as u64 {
        let moved = native.pop_front().unwrap();
        native.push_back(moved.wrapping_add(value));
        let moved = per_op.pop_front().unwrap();
        per_op.push_back(moved.wrapping_add(value)).unwrap();
        let moved = batched.pop_front().unwrap();
        batched.push_back(moved.wrapping_add(value)).unwrap();
    }

    compact_fifo_churn(&mut per_op, operations, population as u64, false);
    compact_fifo_churn(&mut batched, operations, population as u64, true);
    for value in 0..operations {
        native.push_back(population as u64 + value);
        native.pop_front().expect("FIFO queue remains populated");
    }

    assert_eq!(
        per_op.iter().copied().collect::<Vec<_>>(),
        native.iter().copied().collect::<Vec<_>>()
    );
    assert_eq!(
        batched.iter().copied().collect::<Vec<_>>(),
        native.iter().copied().collect::<Vec<_>>()
    );
    assert_eq!(contents(&batched), contents(&per_op));
}

/// Reopening a fresh borrow every `batch` operations must remain equivalent to
/// one long borrow; the round-four probe shows this keeps nearly all of the
/// resolution win, so it is the recommended caller pattern for long loops.
#[test]
fn fifo_churn_view_reopen_granularity_matches_single_borrow() {
    init();

    let population = smaller(32, 1_024);
    let operations = smaller(192, 4_096) as u64;
    let capacity = population + operations as usize + 8;
    let shift = population / 3;
    let batch = 16_u64;

    let mut single = CompactVecDeque::with_capacity(capacity).unwrap();
    let mut reopened = CompactVecDeque::with_capacity(capacity).unwrap();
    for value in 0..population as u64 {
        single.push_back(value).unwrap();
        reopened.push_back(value).unwrap();
    }
    for value in 0..shift as u64 {
        let moved = single.pop_front().unwrap();
        single.push_back(moved.wrapping_add(value)).unwrap();
        let moved = reopened.pop_front().unwrap();
        reopened.push_back(moved.wrapping_add(value)).unwrap();
    }

    single
        .with_view(|ring| -> Result<(), compact_collections::CollectionError> {
            for value in 0..operations {
                ring.push_back(population as u64 + value)?;
                ring.pop_front().expect("populated");
            }
            Ok(())
        })
        .unwrap();

    let mut remaining = operations;
    let mut next = population as u64;
    while remaining > 0 {
        let step = batch.min(remaining);
        reopened
            .with_view(|ring| -> Result<(), compact_collections::CollectionError> {
                for _ in 0..step {
                    ring.push_back(next)?;
                    ring.pop_front().expect("populated");
                    next += 1;
                }
                Ok(())
            })
            .unwrap();
        remaining -= step;
    }

    assert_eq!(contents(&single), contents(&reopened));
}

/// Reserving a wrapped full ring must preserve its logical order before a
/// borrow-scoped view uses the new capacity.
#[test]
fn reserve_after_wrap_preserves_values_for_view_and_vecdeque() {
    init();

    let mut compact = CompactVecDeque::with_capacity(4).unwrap();
    let mut native = VecDeque::with_capacity(4);
    for value in 0..4_u64 {
        compact.push_back(value).unwrap();
        native.push_back(value);
    }
    for _ in 0..2 {
        assert_eq!(compact.pop_front(), native.pop_front());
    }
    for value in 4..6_u64 {
        compact.push_back(value).unwrap();
        native.push_back(value);
    }
    assert_eq!(
        contents(&compact),
        native.iter().copied().collect::<Vec<_>>()
    );

    compact.reserve(4).unwrap();
    native.reserve(4);
    assert!(compact.capacity() >= compact.len() + 4);
    assert!(native.capacity() >= native.len() + 4);
    assert_eq!(
        contents(&compact),
        native.iter().copied().collect::<Vec<_>>()
    );

    compact.with_view(|ring| {
        ring.push_back(6).unwrap();
        native.push_back(6);
        assert_eq!(ring.pop_front(), native.pop_front());
        ring.push_front(1).unwrap();
        native.push_front(1);
        assert_eq!(ring.pop_back(), native.pop_back());
        assert_eq!(
            ring.iter().copied().collect::<Vec<_>>(),
            native.iter().copied().collect::<Vec<_>>()
        );
    });
    assert_eq!(
        contents(&compact),
        native.iter().copied().collect::<Vec<_>>()
    );
}

/// A view that would exceed its reservation fails the push but must leave the
/// deque exactly as the last completed operation left it, and further per-op
/// use must keep working.
#[test]
fn view_push_overflow_leaves_a_reusable_deque() {
    init();

    let mut queue = CompactVecDeque::with_capacity(4).unwrap();
    for value in 0..4_u64 {
        queue.push_back(value).unwrap();
    }
    queue.with_view(|ring| {
        assert_eq!(ring.pop_front(), Some(0));
        assert_eq!(ring.pop_front(), Some(1));
        // Two slots free again: the next two pushes succeed.
        ring.push_back(40).unwrap();
        ring.push_back(41).unwrap();
        // Now full; the extra push must fail without corrupting state.
        assert!(ring.push_back(42).is_err());
        assert_eq!(ring.len(), 4);
        assert_eq!(ring.front().copied(), Some(2));
        assert_eq!(ring.back().copied(), Some(41));
    });

    // Per-op access agrees with the view's final write-back.
    assert_eq!(
        queue.iter().copied().collect::<Vec<_>>(),
        vec![2, 3, 40, 41]
    );
    queue.push_back(99).unwrap();
    assert_eq!(queue.pop_front(), Some(2));
    assert_eq!(queue.pop_back(), Some(99));
}

/// A panic raised after a view performed some operations must still see those
/// operations, matching per-op semantics where every call is independent.
#[test]
fn view_unwind_matches_completed_per_op_prefix() {
    init();

    let mut batched = CompactVecDeque::with_capacity(8).unwrap();
    let mut per_op = CompactVecDeque::with_capacity(8).unwrap();
    for value in 0..4_u64 {
        batched.push_back(value).unwrap();
        per_op.push_back(value).unwrap();
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        batched
            .with_view(|ring| -> Result<(), compact_collections::CollectionError> {
                ring.push_back(10)?;
                ring.pop_front();
                panic!("intentional unwind inside the view");
            })
            .unwrap();
    }));
    assert!(result.is_err());

    per_op.push_back(10).unwrap();
    per_op.pop_front();

    assert_eq!(contents(&batched), contents(&per_op));
}

// ---------------------------------------------------------------------------
// B6: order-book style batched mutable slice updates.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Level {
    quantity: u64,
    order_count: u32,
}

// SAFETY: `Level` is a plain copyable scalar record.
unsafe impl CompactValue for Level {}

/// Apply one round of pseudo-random in-place updates. `batched` borrows the
/// writable slice once for the round; otherwise every access re-resolves.
fn update_level(level: &mut Level, update: usize) {
    level.quantity = level.quantity.saturating_sub(1 + (update % 13) as u64);
    level.order_count = level.order_count.saturating_sub(1);
}

fn update_round(levels: &mut CompactVec<Level>, round: usize, updates: usize, batched: bool) {
    if batched {
        let slice = levels.as_mut_slice();
        for update in 0..updates {
            let index = (round * 137 + update * 31) % slice.len();
            update_level(&mut slice[index], update);
        }
    } else {
        for update in 0..updates {
            let index = (round * 137 + update * 31) % levels.len();
            update_level(&mut levels[index], update);
        }
    }
}

fn update_round_native(levels: &mut [Level], round: usize, updates: usize) {
    for update in 0..updates {
        let index = (round * 137 + update * 31) % levels.len();
        update_level(&mut levels[index], update);
    }
}

#[test]
fn order_book_batched_slice_matches_per_index_updates() {
    init();

    let population = smaller(64, 2_048);
    let rounds = 8;
    let updates = smaller(24, 192);

    let mut batched = CompactVec::with_capacity(population).unwrap();
    let mut indexed = CompactVec::with_capacity(population).unwrap();
    let mut native = Vec::with_capacity(population);
    for index in 0..population {
        let level = Level {
            // The first update removes this level, so the following retain
            // checks matching compaction as well as the unchanged-length case.
            quantity: if index == 0 {
                1
            } else {
                100 + (index as u64 * 53 % 50_000)
            },
            order_count: 1 + (index as u32 % 32),
        };
        batched.push(level).unwrap();
        indexed.push(level).unwrap();
        native.push(level);
    }

    for round in 0..rounds {
        update_round(&mut batched, round, updates, true);
        update_round(&mut indexed, round, updates, false);
        update_round_native(&mut native, round, updates);
        assert_eq!(batched.as_slice(), indexed.as_slice());
        assert_eq!(batched.as_slice(), native.as_slice());

        // Batched updates compose with `retain`, which itself re-resolves the
        // header once; the borrow must not outlive the round.
        batched.retain(|level| level.quantity != 0);
        indexed.retain(|level| level.quantity != 0);
        native.retain(|level| level.quantity != 0);
        assert_eq!(batched.as_slice(), indexed.as_slice());
        assert_eq!(batched.as_slice(), native.as_slice());

        // Growth/replacement between rounds keeps both forms equivalent.
        if round % 2 == 1 {
            batched = batched.try_clone_copy().unwrap();
            indexed = indexed.try_clone_copy().unwrap();
            native = native.clone();
            assert_eq!(batched.as_slice(), indexed.as_slice());
            assert_eq!(batched.as_slice(), native.as_slice());
        }
    }

    assert_eq!(batched.len(), native.len());
}

/// Repeated mutable lookups of distinct slots remain valid within one view, and
/// the deque metadata is written back after the borrow ends.
#[test]
fn view_updates_distinct_slots_within_one_scope() {
    init();

    let mut queue = CompactVecDeque::with_capacity(8).unwrap();
    for value in 0..6_u64 {
        queue.push_back(value).unwrap();
    }
    queue.with_view(|ring| {
        for index in 0..ring.len() {
            if let Some(value) = ring.get_mut(index) {
                *value = value.wrapping_mul(3);
            }
        }
        assert_eq!(ring.front().copied(), Some(0));
        assert_eq!(ring.back().copied(), Some(15));
    });

    assert_eq!(
        queue.iter().copied().collect::<Vec<_>>(),
        vec![0, 3, 6, 9, 12, 15]
    );
}

/// A wide (32-byte) element keeps batched and per-index updates identical; the
/// borrow-scoped slice must not assume a particular element stride.
#[test]
fn order_book_batched_slice_matches_for_wide_elements() {
    init();

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct WideLevel {
        price_micros: u64,
        quantity: u64,
        order_count: u64,
        flags: u64,
    }
    // SAFETY: `WideLevel` is a plain copyable scalar record.
    unsafe impl CompactValue for WideLevel {}

    let population = smaller(48, 512);
    let mut batched = CompactVec::with_capacity(population).unwrap();
    let mut indexed = CompactVec::with_capacity(population).unwrap();
    let mut native = Vec::with_capacity(population);
    for index in 0..population {
        let level = WideLevel {
            price_micros: 50_000_000 + index as u64 * 10_000,
            quantity: 100 + index as u64,
            order_count: index as u64 % 32,
            flags: index as u64 & 3,
        };
        batched.push(level).unwrap();
        indexed.push(level).unwrap();
        native.push(level);
    }

    for round in 0..6_usize {
        {
            let slice = batched.as_mut_slice();
            for (lane, level) in slice.iter_mut().enumerate() {
                level.quantity = level.quantity.wrapping_add((round + lane) as u64);
                level.flags ^= (round + lane) as u64 & 1;
            }
        }
        for lane in 0..indexed.len() {
            indexed[lane].quantity = indexed[lane].quantity.wrapping_add((round + lane) as u64);
            indexed[lane].flags ^= (round + lane) as u64 & 1;
        }
        for (lane, level) in native.iter_mut().enumerate() {
            level.quantity = level.quantity.wrapping_add((round + lane) as u64);
            level.flags ^= (round + lane) as u64 & 1;
        }
        assert_eq!(batched.as_slice(), indexed.as_slice());
        assert_eq!(batched.as_slice(), native.as_slice());
    }
}

/// Zero-sized values behave identically through the view and ordinary calls as
/// long as the per-op side has the same spare capacity; the only intended
/// divergence is that the view refuses to grow.
#[test]
fn view_zero_sized_values_match_per_op() {
    init();

    let mut batched = CompactVecDeque::<()>::with_capacity(4).unwrap();
    let mut per_op = CompactVecDeque::<()>::with_capacity(4).unwrap();
    let mut native = VecDeque::with_capacity(4);
    // Give the per-op deque the same slack the view would need, so neither side
    // has to grow during the compared phase.
    per_op.reserve(4).unwrap();

    batched.with_view(|ring| {
        for _ in 0..4 {
            ring.push_back(()).unwrap();
            per_op.push_back(()).unwrap();
            native.push_back(());
        }
        assert_eq!(ring.len(), per_op.len());
        assert_eq!(ring.len(), native.len());
        for _ in 0..2 {
            assert_eq!(ring.pop_front(), per_op.pop_front());
            assert_eq!(native.pop_front(), Some(()));
        }
        // The view is full at four slots and must refuse growth...
        for _ in 0..2 {
            ring.push_back(()).unwrap();
            native.push_back(());
        }
        assert!(ring.push_back(()).is_err());
    });
    // Mirror the two view-appended pushes on the per-op side so both hold four
    // values again.
    per_op.push_back(()).unwrap();
    per_op.push_back(()).unwrap();
    assert_eq!(batched.len(), per_op.len());
    assert_eq!(per_op.len(), native.len());

    // ...while ordinary per-op access is allowed to grow past the reservation.
    per_op.push_back(()).unwrap();
    native.push_back(());
    assert_eq!(per_op.len(), batched.len() + 1);
    assert_eq!(per_op.len(), native.len());
}
