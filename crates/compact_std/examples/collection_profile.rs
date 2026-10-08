//! Focused, standalone collection diagnostics for `PLAN_COLLECTION_PROFILING.md`.
//!
//! This example is deliberately separate from `benchmark_compare`. It times
//! individual deque operation groups and the individual order-book mutation
//! subphases, so diagnostic instrumentation does not change the shared harness.

use compact_std::{
    CageConfig, CompactBuildHasher, CompactHashMap, CompactHashSet, CompactRuntime, CompactString,
    CompactValue, CompactVec, CompactVecDeque,
};
use core::hash::{BuildHasher, Hash, Hasher};
use std::collections::{hash_map::RandomState, BTreeSet, HashMap, VecDeque};
use std::error::Error;
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const CAGE_BYTES: usize = 128 * 1024 * 1024;
const DEQUE_POPULATION: usize = 4_096;
const DEQUE_CAPACITY: usize = 8_192;
const DEQUE_OPERATIONS: u64 = 80_000;
const LEVEL_COUNT: usize = 2_048;
const BOOK_ROUNDS: usize = 8;
const UPDATES_PER_ROUND: usize = 192;
const HASH_ENTRIES: usize = 16_000;
const COLLISION_ENTRIES: usize = 512;

type ProfileResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone, Copy)]
struct PriceLevel {
    price_micros: u64,
    quantity: u64,
    order_count: u32,
    flags: u32,
}

// SAFETY: the order-book level contains only copyable scalar fields.
unsafe impl CompactValue for PriceLevel {}

trait ProfileQueue: Sized {
    const COMPACT: bool;

    fn with_capacity(capacity: usize) -> ProfileResult<Self>;
    fn capacity(&self) -> usize;
    fn len(&self) -> usize;
    fn push_back(&mut self, value: u64) -> ProfileResult<()>;
    fn push_front(&mut self, value: u64) -> ProfileResult<()>;
    fn pop_front(&mut self) -> Option<u64>;
    fn pop_back(&mut self) -> Option<u64>;
}

impl ProfileQueue for VecDeque<u64> {
    const COMPACT: bool = false;

    #[inline(always)]
    fn with_capacity(capacity: usize) -> ProfileResult<Self> {
        Ok(VecDeque::with_capacity(capacity))
    }

    #[inline(always)]
    fn capacity(&self) -> usize {
        VecDeque::capacity(self)
    }

    #[inline(always)]
    fn len(&self) -> usize {
        VecDeque::len(self)
    }

    #[inline(always)]
    fn push_back(&mut self, value: u64) -> ProfileResult<()> {
        VecDeque::push_back(self, value);
        Ok(())
    }

    #[inline(always)]
    fn push_front(&mut self, value: u64) -> ProfileResult<()> {
        VecDeque::push_front(self, value);
        Ok(())
    }

    #[inline(always)]
    fn pop_front(&mut self) -> Option<u64> {
        VecDeque::pop_front(self)
    }

    #[inline(always)]
    fn pop_back(&mut self) -> Option<u64> {
        VecDeque::pop_back(self)
    }
}

impl ProfileQueue for CompactVecDeque<u64> {
    const COMPACT: bool = true;

    #[inline(always)]
    fn with_capacity(capacity: usize) -> ProfileResult<Self> {
        Ok(CompactVecDeque::with_capacity(capacity)?)
    }

    #[inline(always)]
    fn capacity(&self) -> usize {
        CompactVecDeque::capacity(self)
    }

    #[inline(always)]
    fn len(&self) -> usize {
        CompactVecDeque::len(self)
    }

    #[inline(always)]
    fn push_back(&mut self, value: u64) -> ProfileResult<()> {
        CompactVecDeque::push_back(self, value)?;
        Ok(())
    }

    #[inline(always)]
    fn push_front(&mut self, value: u64) -> ProfileResult<()> {
        CompactVecDeque::push_front(self, value)?;
        Ok(())
    }

    #[inline(always)]
    fn pop_front(&mut self) -> Option<u64> {
        CompactVecDeque::pop_front(self)
    }

    #[inline(always)]
    fn pop_back(&mut self) -> Option<u64> {
        CompactVecDeque::pop_back(self)
    }
}

fn main() -> ProfileResult<()> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let scenario = argument(&arguments, "--scenario")?.to_owned();
    let variant = argument(&arguments, "--variant")?.to_owned();
    let runs = argument(&arguments, "--runs")?.parse::<usize>()?;
    if runs < 2 {
        return Err("--runs must be at least 2".into());
    }
    let compact = match variant.as_str() {
        "native" => false,
        "compact" => true,
        _ => return Err("--variant must be native or compact".into()),
    };
    if compact {
        CompactRuntime::init(CageConfig::new(CAGE_BYTES))?;
    }

    match scenario.as_str() {
        "A4" if compact => profile_deque::<CompactVecDeque<u64>>(&variant, runs)?,
        "A4" => profile_deque::<VecDeque<u64>>(&variant, runs)?,
        "A5" => profile_hashing(compact, &variant, runs)?,
        "B6" => profile_order_book(compact, &variant, runs)?,
        _ => return Err("--scenario must be A4, A5, or B6".into()),
    }
    emit_memory(&scenario, &variant, compact);
    Ok(())
}

fn argument<'a>(arguments: &'a [String], name: &str) -> ProfileResult<&'a str> {
    let index = arguments
        .iter()
        .position(|argument| argument == name)
        .ok_or_else(|| format!("missing {name}"))?;
    arguments
        .get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("missing value for {name}").into())
}

fn profile_hashing(compact: bool, variant: &str, runs: usize) -> ProfileResult<()> {
    let scalar_keys = (0..HASH_ENTRIES as u32).collect::<Vec<_>>();
    let short_keys = (0..HASH_ENTRIES)
        .map(|key| format!("k{key:05}"))
        .collect::<Vec<_>>();
    let long_keys = (0..HASH_ENTRIES)
        .map(|key| {
            format!(
                "tenant/{:04}/market/snapshot/record/{key:08}/revision/0003",
                key % 128
            )
        })
        .collect::<Vec<_>>();

    if compact {
        let production = CompactBuildHasher::new();
        run_hash_only("default_u32_hash", variant, runs, &production, &scalar_keys);
        run_hash_only(
            "default_short_string_hash",
            variant,
            runs,
            &production,
            &short_keys,
        );
        run_hash_only(
            "default_long_string_hash",
            variant,
            runs,
            &production,
            &long_keys,
        );
        let custom = FnvBuildHasher;
        run_hash_only("fnv_u32_hash", variant, runs, &custom, &scalar_keys);
        run_hash_only("fnv_short_string_hash", variant, runs, &custom, &short_keys);
        run_hash_only("fnv_long_string_hash", variant, runs, &custom, &long_keys);
        profile_a5_probe_counts(variant)?;
        profile_collision_comparisons(true, variant)?;
        profile_fnv_map(compact, variant, runs)?;
    } else {
        let production = RandomState::new();
        run_hash_only("default_u32_hash", variant, runs, &production, &scalar_keys);
        run_hash_only(
            "default_short_string_hash",
            variant,
            runs,
            &production,
            &short_keys,
        );
        run_hash_only(
            "default_long_string_hash",
            variant,
            runs,
            &production,
            &long_keys,
        );
        let custom = FnvBuildHasher;
        run_hash_only("fnv_u32_hash", variant, runs, &custom, &scalar_keys);
        run_hash_only("fnv_short_string_hash", variant, runs, &custom, &short_keys);
        run_hash_only("fnv_long_string_hash", variant, runs, &custom, &long_keys);
        profile_collision_comparisons(false, variant)?;
        profile_fnv_map(compact, variant, runs)?;
    }
    Ok(())
}

fn run_hash_only<B: BuildHasher, K: Hash>(
    phase: &str,
    variant: &str,
    runs: usize,
    builder: &B,
    keys: &[K],
) {
    let mut samples = Vec::new();
    for run in 0..runs {
        let started = Instant::now();
        let mut checksum = 0_u64;
        for key in keys {
            checksum = checksum.wrapping_add(builder.hash_one(black_box(key)));
        }
        let elapsed = started.elapsed().as_nanos();
        black_box(checksum);
        samples.push(elapsed);
        println!("SAMPLE\tA5\t{variant}\t{phase}\t{run}\t{elapsed}\t{checksum}");
    }
    emit_summary("A5", variant, phase, &mut samples);
}

#[derive(Clone, Copy, Default)]
struct FnvBuildHasher;

// SAFETY: this zero-sized builder retains no process-local or borrowed state.
unsafe impl CompactValue for FnvBuildHasher {}

impl BuildHasher for FnvBuildHasher {
    type Hasher = FnvHasher;

    fn build_hasher(&self) -> Self::Hasher {
        FnvHasher(0xcbf2_9ce4_8422_2325)
    }
}

struct FnvHasher(u64);

impl Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

fn profile_fnv_map(compact: bool, variant: &str, runs: usize) -> ProfileResult<()> {
    let mut build = Vec::new();
    let mut lookup = Vec::new();
    let mut churn = Vec::new();
    let mut final_len = 0_usize;
    for run in 0..runs {
        if compact {
            let started = Instant::now();
            let mut map = CompactHashMap::with_capacity_and_hasher(HASH_ENTRIES, FnvBuildHasher)?;
            for key in 0..HASH_ENTRIES as u32 {
                map.insert(key, u64::from(key) * 17)?;
            }
            let build_ns = started.elapsed().as_nanos();
            build.push(build_ns);
            println!("SAMPLE\tA5\t{variant}\tfnv_map_build\t{run}\t{build_ns}");

            let started = Instant::now();
            let mut checksum = 0_u64;
            for key in 0..HASH_ENTRIES as u32 {
                checksum = checksum.wrapping_add(map.get(&key).copied().unwrap_or(0));
            }
            let lookup_ns = started.elapsed().as_nanos();
            lookup.push(lookup_ns);
            println!("SAMPLE\tA5\t{variant}\tfnv_map_hit_lookup\t{run}\t{lookup_ns}\t{checksum}");

            let started = Instant::now();
            for key in 0..4_000_u32 {
                if key % 7 == 0 {
                    let _ = map.remove(&key);
                }
                map.insert(32_000 + key, u64::from(32_000 + key) * 17)?;
            }
            let churn_ns = started.elapsed().as_nanos();
            churn.push(churn_ns);
            final_len = map.len();
            println!("SAMPLE\tA5\t{variant}\tfnv_map_churn\t{run}\t{churn_ns}\t{final_len}");
        } else {
            let started = Instant::now();
            let mut map = HashMap::with_capacity_and_hasher(HASH_ENTRIES, FnvBuildHasher);
            for key in 0..HASH_ENTRIES as u32 {
                map.insert(key, u64::from(key) * 17);
            }
            let build_ns = started.elapsed().as_nanos();
            build.push(build_ns);
            println!("SAMPLE\tA5\t{variant}\tfnv_map_build\t{run}\t{build_ns}");

            let started = Instant::now();
            let mut checksum = 0_u64;
            for key in 0..HASH_ENTRIES as u32 {
                checksum = checksum.wrapping_add(map.get(&key).copied().unwrap_or(0));
            }
            let lookup_ns = started.elapsed().as_nanos();
            lookup.push(lookup_ns);
            println!("SAMPLE\tA5\t{variant}\tfnv_map_hit_lookup\t{run}\t{lookup_ns}\t{checksum}");

            let started = Instant::now();
            for key in 0..4_000_u32 {
                if key % 7 == 0 {
                    let _ = map.remove(&key);
                }
                map.insert(32_000 + key, u64::from(32_000 + key) * 17);
            }
            let churn_ns = started.elapsed().as_nanos();
            churn.push(churn_ns);
            final_len = map.len();
            println!("SAMPLE\tA5\t{variant}\tfnv_map_churn\t{run}\t{churn_ns}\t{final_len}");
        }
    }
    emit_summary("A5", variant, "fnv_map_build", &mut build);
    emit_summary("A5", variant, "fnv_map_hit_lookup", &mut lookup);
    emit_summary("A5", variant, "fnv_map_churn", &mut churn);
    println!("META\tA5\t{variant}\tfnv_map_final_len\t{final_len}");
    Ok(())
}

static EQUALITY_CHECKS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Hash)]
struct CountedKey(u32);

// SAFETY: this diagnostic key contains only a scalar integer.
unsafe impl CompactValue for CountedKey {}

impl PartialEq for CountedKey {
    fn eq(&self, other: &Self) -> bool {
        EQUALITY_CHECKS.fetch_add(1, Ordering::Relaxed);
        self.0 == other.0
    }
}
impl Eq for CountedKey {}

#[derive(Clone, Copy, Default)]
struct ConstantBuildHasher;

// SAFETY: this zero-sized builder retains no process-local or borrowed state.
unsafe impl CompactValue for ConstantBuildHasher {}

impl BuildHasher for ConstantBuildHasher {
    type Hasher = ConstantHasher;

    fn build_hasher(&self) -> Self::Hasher {
        ConstantHasher
    }
}

struct ConstantHasher;

impl Hasher for ConstantHasher {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, _bytes: &[u8]) {}
}

fn profile_collision_comparisons(compact: bool, variant: &str) -> ProfileResult<()> {
    let comparisons;
    let capacity;
    if compact {
        let mut map =
            CompactHashMap::with_capacity_and_hasher(COLLISION_ENTRIES, ConstantBuildHasher)?;
        for key in 0..COLLISION_ENTRIES as u32 {
            map.insert(CountedKey(key), u64::from(key))?;
        }
        EQUALITY_CHECKS.store(0, Ordering::Relaxed);
        let mut checksum = 0_u64;
        for key in 0..COLLISION_ENTRIES as u32 {
            checksum = checksum.wrapping_add(
                map.get(&CountedKey(key))
                    .copied()
                    .expect("all collision keys are present"),
            );
        }
        black_box(checksum);
        comparisons = EQUALITY_CHECKS.load(Ordering::Relaxed);
        capacity = map.capacity();
    } else {
        let mut map = HashMap::with_capacity_and_hasher(COLLISION_ENTRIES, ConstantBuildHasher);
        for key in 0..COLLISION_ENTRIES as u32 {
            map.insert(CountedKey(key), u64::from(key));
        }
        EQUALITY_CHECKS.store(0, Ordering::Relaxed);
        let mut checksum = 0_u64;
        for key in 0..COLLISION_ENTRIES as u32 {
            checksum = checksum.wrapping_add(
                map.get(&CountedKey(key))
                    .copied()
                    .expect("all collision keys are present"),
            );
        }
        black_box(checksum);
        comparisons = EQUALITY_CHECKS.load(Ordering::Relaxed);
        capacity = map.capacity();
    }
    let compact_groups = (0..COLLISION_ENTRIES)
        .map(|slot| slot / 16 + 1)
        .sum::<usize>();
    println!("META\tA5\t{variant}\tconstant_hash_entries\t{COLLISION_ENTRIES}");
    println!("META\tA5\t{variant}\tconstant_hash_capacity\t{capacity}");
    println!("META\tA5\t{variant}\tconstant_hash_full_comparisons\t{comparisons}");
    if compact {
        println!("META\tA5\t{variant}\tconstant_hash_control_groups\t{compact_groups}");
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ModelSlot {
    Empty,
    Full(u32),
    Tombstone,
}

#[derive(Clone, Copy, Default)]
struct ProbeCounts {
    operations: u64,
    groups: u64,
    full_candidates: u64,
    tombstones_seen: u64,
}

struct CompactProbeModel {
    slots: Vec<ModelSlot>,
    len: usize,
    tombstones: usize,
    rehashes: usize,
    rehash_groups: u64,
    counts: ProbeCounts,
}

impl CompactProbeModel {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            len: 0,
            tombstones: 0,
            rehashes: 0,
            rehash_groups: 0,
            counts: ProbeCounts::default(),
        }
    }

    fn capacity(&self) -> usize {
        self.slots.len()
    }

    fn reserve(&mut self, additional: usize, builder: &CompactBuildHasher) {
        let required = self.len + additional;
        let current = self.capacity();
        let mut slots = current.max(8);
        while required.saturating_mul(8) >= slots.saturating_mul(7) {
            slots *= 2;
        }
        if current == 0 || slots > current || self.tombstones > current / 4 {
            self.rehash(slots, builder);
        }
    }

    fn ensure_insert_capacity(&mut self, builder: &CompactBuildHasher) {
        let capacity = self.capacity();
        if capacity == 0
            || (self.len + self.tombstones + 1).saturating_mul(8) >= capacity.saturating_mul(7)
        {
            self.reserve(1, builder);
        }
    }

    fn rehash(&mut self, slots: usize, builder: &CompactBuildHasher) {
        let slots = slots.max(8).next_power_of_two();
        let old = std::mem::replace(&mut self.slots, vec![ModelSlot::Empty; slots]);
        let mask = slots - 1;
        for entry in old {
            let ModelSlot::Full(key) = entry else {
                continue;
            };
            let hash = builder.hash_one(key);
            let start = hash as usize & mask;
            let mut consumed = 0;
            'search: while consumed < slots {
                let count = (slots - consumed).min(16);
                let cursor = start.wrapping_add(consumed) & mask;
                self.rehash_groups += 1;
                for lane in 0..count {
                    let index = cursor.wrapping_add(lane) & mask;
                    if matches!(self.slots[index], ModelSlot::Empty) {
                        self.slots[index] = ModelSlot::Full(key);
                        break 'search;
                    }
                }
                consumed += count;
            }
        }
        self.tombstones = 0;
        self.rehashes += 1;
    }

    fn probe(&mut self, key: u32, hash: u64) -> Option<(usize, bool)> {
        let mask = self.capacity().checked_sub(1)?;
        let start = hash as usize & mask;
        let mut first_tombstone = None;
        let mut consumed = 0;
        self.counts.operations += 1;
        while consumed < self.capacity() {
            let count = (self.capacity() - consumed).min(16);
            let cursor = start.wrapping_add(consumed) & mask;
            self.counts.groups += 1;
            for lane in 0..count {
                let index = cursor.wrapping_add(lane) & mask;
                match self.slots[index] {
                    ModelSlot::Empty => {
                        return Some((first_tombstone.unwrap_or(index), false));
                    }
                    ModelSlot::Full(stored) => {
                        self.counts.full_candidates += 1;
                        if stored == key {
                            return Some((index, true));
                        }
                    }
                    ModelSlot::Tombstone => {
                        self.counts.tombstones_seen += 1;
                        first_tombstone.get_or_insert(index);
                    }
                }
            }
            consumed += count;
        }
        first_tombstone.map(|index| (index, false))
    }

    fn insert(&mut self, key: u32, hash: u64, builder: &CompactBuildHasher) {
        self.ensure_insert_capacity(builder);
        let Some((index, found)) = self.probe(key, hash) else {
            return;
        };
        if found {
            return;
        }
        if matches!(self.slots[index], ModelSlot::Tombstone) {
            self.tombstones -= 1;
        }
        self.slots[index] = ModelSlot::Full(key);
        self.len += 1;
    }

    fn get(&mut self, key: u32, hash: u64) -> bool {
        self.probe(key, hash).is_some_and(|(_, found)| found)
    }

    fn remove(&mut self, key: u32, hash: u64) -> bool {
        let Some((index, found)) = self.probe(key, hash) else {
            return false;
        };
        if !found {
            return false;
        }
        self.slots[index] = ModelSlot::Tombstone;
        self.len -= 1;
        self.tombstones += 1;
        true
    }

    fn take_counts(&mut self) -> ProbeCounts {
        std::mem::take(&mut self.counts)
    }
}

fn profile_a5_probe_counts(variant: &str) -> ProfileResult<()> {
    let map_builder = CompactBuildHasher::new();
    let set_builder = CompactBuildHasher::new();
    let mut map = CompactHashMap::with_capacity_and_hasher(HASH_ENTRIES, map_builder)?;
    let mut set = CompactHashSet::with_capacity_and_hasher(HASH_ENTRIES, set_builder)?;
    let mut map_model = CompactProbeModel::new();
    let mut set_model = CompactProbeModel::new();
    map_model.reserve(HASH_ENTRIES, &map_builder);
    set_model.reserve(HASH_ENTRIES, &set_builder);

    for key in 0..HASH_ENTRIES as u32 {
        map.insert(key, u64::from(key) * 17)?;
        set.insert(key)?;
        map_model.insert(key, map_builder.hash_one(key), &map_builder);
        set_model.insert(key, set_builder.hash_one(key), &set_builder);
    }
    emit_probe_counts(variant, "build", map_model.take_counts(), &map_model);
    emit_probe_counts(variant, "set_build", set_model.take_counts(), &set_model);

    for key in 0..4_000_u32 {
        let map_hash = map_builder.hash_one(key);
        let set_hash = set_builder.hash_one(key);
        black_box(map.get_mut(&key));
        black_box(map_model.get(key, map_hash));
        if key % 7 == 0 {
            black_box(map.remove(&key));
            black_box(set.remove(&key));
            black_box(map_model.remove(key, map_hash));
            black_box(set_model.remove(key, set_hash));
        }
        let new_key = 32_000 + key;
        map.insert(new_key, u64::from(new_key) * 17)?;
        set.insert(new_key)?;
        map_model.insert(new_key, map_builder.hash_one(new_key), &map_builder);
        set_model.insert(new_key, set_builder.hash_one(new_key), &set_builder);
    }
    emit_probe_counts(variant, "churn_map", map_model.take_counts(), &map_model);
    emit_probe_counts(variant, "churn_set", set_model.take_counts(), &set_model);

    for key in (0..36_000_u32).step_by(13) {
        black_box(map.get(&key));
        black_box(set.contains(&key));
        black_box(map_model.get(key, map_builder.hash_one(key)));
        black_box(set_model.get(key, set_builder.hash_one(key)));
    }
    emit_probe_counts(variant, "lookup_map", map_model.take_counts(), &map_model);
    emit_probe_counts(variant, "lookup_set", set_model.take_counts(), &set_model);

    if map.capacity() != map_model.capacity() || map.len() != map_model.len {
        return Err("compact A5 map probe model diverged from collection capacity/length".into());
    }
    if set.capacity() != set_model.capacity() || set.len() != set_model.len {
        return Err("compact A5 set probe model diverged from collection capacity/length".into());
    }
    let actual_map_keys = map.keys().copied().collect::<BTreeSet<_>>();
    let model_map_keys = map_model
        .slots
        .iter()
        .filter_map(|slot| match slot {
            ModelSlot::Full(key) => Some(*key),
            ModelSlot::Empty | ModelSlot::Tombstone => None,
        })
        .collect::<BTreeSet<_>>();
    let actual_set_keys = set.iter().copied().collect::<BTreeSet<_>>();
    let model_set_keys = set_model
        .slots
        .iter()
        .filter_map(|slot| match slot {
            ModelSlot::Full(key) => Some(*key),
            ModelSlot::Empty | ModelSlot::Tombstone => None,
        })
        .collect::<BTreeSet<_>>();
    if actual_map_keys != model_map_keys || actual_set_keys != model_set_keys {
        return Err("compact A5 probe model diverged from collection key sets".into());
    }
    println!("META\tA5\t{variant}\tfinal_len\t{}", map.len());
    println!("META\tA5\t{variant}\tfinal_capacity\t{}", map.capacity());
    println!(
        "META\tA5\t{variant}\tfinal_tombstones\t{}",
        map_model.tombstones
    );
    println!(
        "META\tA5\t{variant}\tfinal_tombstone_rate_ppm\t{}",
        map_model.tombstones * 1_000_000 / map_model.capacity()
    );
    println!("META\tA5\t{variant}\trehashes_map\t{}", map_model.rehashes);
    println!("META\tA5\t{variant}\trehashes_set\t{}", set_model.rehashes);
    println!(
        "META\tA5\t{variant}\trehash_control_groups_map\t{}",
        map_model.rehash_groups
    );
    println!(
        "META\tA5\t{variant}\trehash_control_groups_set\t{}",
        set_model.rehash_groups
    );
    Ok(())
}

fn emit_probe_counts(variant: &str, phase: &str, counts: ProbeCounts, model: &CompactProbeModel) {
    println!("PROBE\tA5\t{variant}\t{phase}\toperations\t{}\tgroups\t{}\tfull_candidates\t{}\ttombstones_seen\t{}\tcapacity\t{}\ttombstone_slots\t{}", counts.operations, counts.groups, counts.full_candidates, counts.tombstones_seen, model.capacity(), model.tombstones);
}

fn profile_deque<Q: ProfileQueue>(variant: &str, runs: usize) -> ProfileResult<()> {
    let mut samples = Vec::new();
    let mut peak_cage_bytes = 0_usize;
    for operation in ["push_back_only", "push_front_only"] {
        let phase = operation;
        samples.clear();
        for run in 0..runs {
            let mut queue = Q::with_capacity(DEQUE_OPERATIONS as usize)?;
            let started = Instant::now();
            for value in 0..DEQUE_OPERATIONS {
                if phase == "push_back_only" {
                    queue.push_back(black_box(value))?;
                } else {
                    queue.push_front(black_box(value))?;
                }
            }
            let elapsed = started.elapsed().as_nanos();
            let check = black_box(queue.len() as u64 + queue.capacity() as u64);
            if Q::COMPACT {
                peak_cage_bytes = peak_cage_bytes.max(CompactRuntime::used_bytes()?);
            }
            samples.push(elapsed);
            println!("SAMPLE\tA4\t{variant}\t{phase}\t{run}\t{elapsed}\t{check}");
        }
        emit_summary("A4", variant, phase, &mut samples);
    }

    for operation in ["pop_front_only", "pop_back_only"] {
        let phase = operation;
        samples.clear();
        for run in 0..runs {
            let mut queue = Q::with_capacity(DEQUE_OPERATIONS as usize)?;
            for value in 0..DEQUE_OPERATIONS {
                queue.push_back(value)?;
            }
            let started = Instant::now();
            let mut checksum = 0_u64;
            for _ in 0..DEQUE_OPERATIONS {
                checksum = checksum.wrapping_add(black_box(if phase == "pop_front_only" {
                    queue.pop_front().expect("prepared queue is nonempty")
                } else {
                    queue.pop_back().expect("prepared queue is nonempty")
                }));
            }
            let elapsed = started.elapsed().as_nanos();
            black_box(checksum);
            if Q::COMPACT {
                peak_cage_bytes = peak_cage_bytes.max(CompactRuntime::used_bytes()?);
            }
            samples.push(elapsed);
            println!("SAMPLE\tA4\t{variant}\t{phase}\t{run}\t{elapsed}\t{checksum}");
        }
        emit_summary("A4", variant, phase, &mut samples);
    }

    for (phase, capacity, head_shift) in [
        ("fifo_churn_contiguous", DEQUE_CAPACITY, 0),
        (
            "fifo_churn_wrapped",
            DEQUE_CAPACITY,
            DEQUE_CAPACITY - DEQUE_POPULATION / 2,
        ),
        ("fifo_churn_initial_growth", DEQUE_POPULATION, 0),
    ] {
        samples.clear();
        for run in 0..runs {
            let mut queue = Q::with_capacity(capacity)?;
            for value in 0..DEQUE_POPULATION as u64 {
                queue.push_back(value)?;
            }
            if head_shift != 0 {
                for value in 0..head_shift {
                    let moved = queue.pop_front().expect("prepared queue is nonempty");
                    queue.push_back(moved.wrapping_add(value as u64))?;
                }
            }
            let initial_capacity = queue.capacity();
            let started = Instant::now();
            let mut checksum = 0_u64;
            for value in 0..DEQUE_OPERATIONS {
                queue.push_back(black_box(DEQUE_POPULATION as u64 + value))?;
                checksum = checksum.wrapping_add(black_box(
                    queue.pop_front().expect("FIFO queue remains populated"),
                ));
            }
            let elapsed = started.elapsed().as_nanos();
            let capacity_after = queue.capacity();
            black_box(checksum);
            if Q::COMPACT {
                peak_cage_bytes = peak_cage_bytes.max(CompactRuntime::used_bytes()?);
            }
            samples.push(elapsed);
            println!(
                "SAMPLE\tA4\t{variant}\t{phase}\t{run}\t{elapsed}\t{checksum}\t{initial_capacity}\t{capacity_after}"
            );
        }
        emit_summary("A4", variant, phase, &mut samples);
    }

    // These separate microprobes put a lower bound on the two scalar portions
    // of the deque path. They duplicate the current `physical_index` arithmetic
    // and head/length updates without changing collection code.
    samples.clear();
    for run in 0..runs {
        let started = Instant::now();
        let mut checksum = 0_usize;
        for offset in 0..DEQUE_OPERATIONS as usize {
            let head = (offset.wrapping_mul(31)) % DEQUE_CAPACITY;
            let logical = (offset.wrapping_mul(137)) % DEQUE_POPULATION;
            checksum = checksum.wrapping_add(physical_index_shadow(
                black_box(head),
                black_box(logical),
                DEQUE_CAPACITY,
            ));
        }
        let elapsed = started.elapsed().as_nanos();
        black_box(checksum);
        samples.push(elapsed);
        println!("SAMPLE\tA4\t{variant}\tphysical_index_shadow\t{run}\t{elapsed}\t{checksum}");
    }
    emit_summary("A4", variant, "physical_index_shadow", &mut samples);

    samples.clear();
    for run in 0..runs {
        let started = Instant::now();
        let mut head = DEQUE_CAPACITY - DEQUE_POPULATION / 2;
        let mut len = DEQUE_POPULATION;
        for _ in 0..DEQUE_OPERATIONS {
            len = black_box(len + 1);
            head = if len == 1 || head + 1 == DEQUE_CAPACITY {
                0
            } else {
                head + 1
            };
            len = black_box(len - 1);
        }
        let elapsed = started.elapsed().as_nanos();
        let check = black_box(u64::from(head as u32) + len as u64);
        samples.push(elapsed);
        println!("SAMPLE\tA4\t{variant}\tmetadata_updates_shadow\t{run}\t{elapsed}\t{check}");
    }
    emit_summary("A4", variant, "metadata_updates_shadow", &mut samples);

    profile_slice_write_read(Q::COMPACT, variant, runs)?;

    if Q::COMPACT {
        profile_header_resolution(variant, runs)?;
        println!("META\tA4\t{variant}\tpeak_cage_live_bytes\t{peak_cage_bytes}");
    }
    Ok(())
}

fn physical_index_shadow(head: usize, logical: usize, capacity: usize) -> usize {
    let until_wrap = capacity - head;
    if logical >= until_wrap {
        logical - until_wrap
    } else {
        head + logical
    }
}

fn profile_header_resolution(variant: &str, runs: usize) -> ProfileResult<()> {
    let mut samples = Vec::new();
    for run in 0..runs {
        let mut storage = CompactRuntime::alloc_owned_slice::<u64>(DEQUE_CAPACITY)?;
        let started = Instant::now();
        let mut address = 0_usize;
        for _ in 0..DEQUE_OPERATIONS {
            address ^= black_box(storage.uninit_capacity_mut().as_mut_ptr() as usize);
        }
        let elapsed = started.elapsed().as_nanos();
        black_box(address);
        samples.push(elapsed);
        println!("SAMPLE\tA4\t{variant}\theader_resolution_only\t{run}\t{elapsed}\t{address}");
    }
    emit_summary("A4", variant, "header_resolution_only", &mut samples);
    Ok(())
}

fn profile_slice_write_read(compact: bool, variant: &str, runs: usize) -> ProfileResult<()> {
    let mut samples = Vec::new();
    for run in 0..runs {
        let elapsed;
        let mut checksum = 0_u64;
        if compact {
            let mut storage = CompactRuntime::alloc_owned_slice::<u64>(DEQUE_CAPACITY)?;
            let slots = storage.uninit_capacity_mut();
            let started = Instant::now();
            for index in 0..DEQUE_OPERATIONS as usize {
                let slot = index % DEQUE_CAPACITY;
                // SAFETY: `slot` is within the full capacity slice; this u64 is
                // written before the matching read, and needs no destructor.
                unsafe { slots.get_unchecked_mut(slot).write(index as u64) };
                // SAFETY: the previous statement initialized this exact slot.
                checksum =
                    checksum.wrapping_add(unsafe { *slots.get_unchecked(slot).assume_init_ref() });
            }
            elapsed = started.elapsed().as_nanos();
        } else {
            let mut storage = vec![0_u64; DEQUE_CAPACITY];
            let started = Instant::now();
            for index in 0..DEQUE_OPERATIONS as usize {
                let slot = index % DEQUE_CAPACITY;
                // SAFETY: `slot` is within the vector's initialized slice.
                unsafe { *storage.get_unchecked_mut(slot) = index as u64 };
                // SAFETY: the previous statement initialized this exact slot.
                checksum = checksum.wrapping_add(unsafe { *storage.get_unchecked(slot) });
            }
            elapsed = started.elapsed().as_nanos();
        }
        black_box(checksum);
        samples.push(elapsed);
        println!("SAMPLE\tA4\t{variant}\tslice_write_read_only\t{run}\t{elapsed}\t{checksum}");
    }
    emit_summary("A4", variant, "slice_write_read_only", &mut samples);
    Ok(())
}

enum OrderBook {
    Native {
        symbol: String,
        venue: String,
        bids: Vec<PriceLevel>,
        asks: Vec<PriceLevel>,
    },
    Compact {
        symbol: CompactString,
        venue: CompactString,
        bids: CompactVec<PriceLevel>,
        asks: CompactVec<PriceLevel>,
    },
}

impl OrderBook {
    fn build(compact: bool, levels: &[PriceLevel]) -> ProfileResult<Self> {
        let midpoint = levels.len() / 2;
        if compact {
            let mut bids = CompactVec::with_capacity(midpoint)?;
            let mut asks = CompactVec::with_capacity(midpoint)?;
            for level in levels[..midpoint].iter().rev() {
                bids.push(*level)?;
            }
            for level in &levels[midpoint..] {
                asks.push(*level)?;
            }
            Ok(Self::Compact {
                symbol: CompactString::from_str("ACME-USD")?,
                venue: CompactString::from_str("northstar")?,
                bids,
                asks,
            })
        } else {
            Ok(Self::Native {
                symbol: "ACME-USD".to_owned(),
                venue: "northstar".to_owned(),
                bids: levels[..midpoint].iter().rev().copied().collect(),
                asks: levels[midpoint..].to_vec(),
            })
        }
    }

    fn update_quotes_profiled(&mut self) -> ProfileResult<(u128, u128, u128)> {
        let (mut update_ns, mut retain_ns, mut rebuild_ns) = (0_u128, 0_u128, 0_u128);
        match self {
            Self::Native { bids, asks, .. } => {
                for round in 0..BOOK_ROUNDS {
                    let started = Instant::now();
                    for update in 0..UPDATES_PER_ROUND {
                        let index = (round * 137 + update * 31) % bids.len();
                        let level = &mut bids[index];
                        level.quantity = if update % 11 == 0 {
                            0
                        } else {
                            level.quantity.saturating_sub(1 + (update % 13) as u64)
                        };
                        level.order_count =
                            level
                                .order_count
                                .saturating_sub(if update % 11 == 0 { 1 } else { 0 });
                        let ask_index = (round * 73 + update * 17) % asks.len();
                        let ask = &mut asks[ask_index];
                        ask.quantity = if update % 13 == 0 {
                            0
                        } else {
                            ask.quantity.saturating_sub(1 + (update % 9) as u64)
                        };
                        ask.order_count =
                            ask.order_count
                                .saturating_sub(if update % 13 == 0 { 1 } else { 0 });
                    }
                    update_ns += started.elapsed().as_nanos();

                    let started = Instant::now();
                    bids.retain(|level| level.quantity != 0);
                    asks.retain(|level| level.quantity != 0);
                    retain_ns += started.elapsed().as_nanos();

                    if round % 2 == 1 {
                        let started = Instant::now();
                        *bids = bids.clone();
                        *asks = asks.clone();
                        rebuild_ns += started.elapsed().as_nanos();
                    }
                }
            }
            Self::Compact { bids, asks, .. } => {
                for round in 0..BOOK_ROUNDS {
                    let started = Instant::now();
                    for update in 0..UPDATES_PER_ROUND {
                        let index = (round * 137 + update * 31) % bids.len();
                        let level = &mut bids[index];
                        level.quantity = if update % 11 == 0 {
                            0
                        } else {
                            level.quantity.saturating_sub(1 + (update % 13) as u64)
                        };
                        level.order_count =
                            level
                                .order_count
                                .saturating_sub(if update % 11 == 0 { 1 } else { 0 });
                        let ask_index = (round * 73 + update * 17) % asks.len();
                        let ask = &mut asks[ask_index];
                        ask.quantity = if update % 13 == 0 {
                            0
                        } else {
                            ask.quantity.saturating_sub(1 + (update % 9) as u64)
                        };
                        ask.order_count =
                            ask.order_count
                                .saturating_sub(if update % 13 == 0 { 1 } else { 0 });
                    }
                    update_ns += started.elapsed().as_nanos();

                    let started = Instant::now();
                    bids.retain(|level| level.quantity != 0);
                    asks.retain(|level| level.quantity != 0);
                    retain_ns += started.elapsed().as_nanos();

                    if round % 2 == 1 {
                        let started = Instant::now();
                        *bids = bids.try_clone_copy()?;
                        *asks = asks.try_clone_copy()?;
                        rebuild_ns += started.elapsed().as_nanos();
                    }
                }
            }
        }
        Ok((update_ns, retain_ns, rebuild_ns))
    }

    fn read_depth(&self) -> u64 {
        match self {
            Self::Native {
                symbol,
                venue,
                bids,
                asks,
            } => depth_checksum(symbol.len(), venue.len(), bids, asks),
            Self::Compact {
                symbol,
                venue,
                bids,
                asks,
            } => depth_checksum(symbol.len(), venue.len(), bids, asks),
        }
    }

    fn snapshot_copy(&self) -> ProfileResult<u64> {
        Ok(match self {
            Self::Native { bids, asks, .. } => bids
                .clone()
                .into_iter()
                .chain(asks.clone())
                .map(level_checksum)
                .sum(),
            Self::Compact { bids, asks, .. } => {
                let mut bids = bids.try_clone_copy()?;
                let mut asks = asks.try_clone_copy()?;
                let checksum = bids
                    .iter()
                    .chain(asks.iter())
                    .map(|level| level_checksum(*level))
                    .sum();
                bids.clear();
                asks.clear();
                checksum
            }
        })
    }
}

fn order_book_levels() -> Vec<PriceLevel> {
    (0..LEVEL_COUNT)
        .map(|index| PriceLevel {
            price_micros: 50_000_000 + index as u64 * 10_000,
            quantity: 100 + (index as u64 * 53 % 50_000),
            order_count: 1 + (index as u32 % 32),
            flags: (index as u32) & 3,
        })
        .collect()
}

fn depth_checksum(
    symbol_len: usize,
    venue_len: usize,
    bids: &[PriceLevel],
    asks: &[PriceLevel],
) -> u64 {
    let best_bid = bids.first().map_or(0, |level| level.price_micros);
    let best_ask = asks.first().map_or(0, |level| level.price_micros);
    let bid_depth: u64 = bids.iter().map(|level| level.quantity).sum();
    let ask_depth: u64 = asks.iter().map(|level| level.quantity).sum();
    let flags: u64 = bids
        .iter()
        .chain(asks.iter())
        .map(|level| u64::from(level.flags))
        .sum();
    best_bid + best_ask + bid_depth + ask_depth + flags + symbol_len as u64 + venue_len as u64
}

fn level_checksum(level: PriceLevel) -> u64 {
    level.price_micros ^ level.quantity ^ u64::from(level.flags)
}

fn profile_order_book(compact: bool, variant: &str, runs: usize) -> ProfileResult<()> {
    let level_data = order_book_levels();
    let mut phases: [Vec<u128>; 6] = std::array::from_fn(|_| Vec::new());
    let mut updates = Vec::new();
    let mut retains = Vec::new();
    let mut rebuilds = Vec::new();
    let mut expected = None;
    for run in 0..runs {
        let end_to_end = Instant::now();
        let started = Instant::now();
        let mut book = OrderBook::build(compact, &level_data)?;
        let build_ns = started.elapsed().as_nanos();
        phases[0].push(build_ns);
        println!("SAMPLE\tB6\t{variant}\tbuild\t{run}\t{build_ns}");
        if compact {
            println!(
                "META\tB6\t{variant}\tcage_live_after_build\t{}",
                CompactRuntime::used_bytes()?
            );
        }

        let started = Instant::now();
        let (update_ns, retain_ns, rebuild_ns) = book.update_quotes_profiled()?;
        let mutation_ns = started.elapsed().as_nanos();
        phases[1].push(mutation_ns);
        updates.push(update_ns);
        retains.push(retain_ns);
        rebuilds.push(rebuild_ns);
        println!(
            "SAMPLE\tB6\t{variant}\tquote_updates_retain_snapshot_rebuild\t{run}\t{mutation_ns}"
        );
        println!("SAMPLE\tB6\t{variant}\tindexed_quote_updates\t{run}\t{update_ns}");
        println!("SAMPLE\tB6\t{variant}\tretain\t{run}\t{retain_ns}");
        println!("SAMPLE\tB6\t{variant}\tsnapshot_rebuild\t{run}\t{rebuild_ns}");
        if compact {
            println!(
                "META\tB6\t{variant}\tcage_live_after_mutation\t{}",
                CompactRuntime::used_bytes()?
            );
        }

        let started = Instant::now();
        let depth = black_box(book.read_depth());
        let read_ns = started.elapsed().as_nanos();
        phases[2].push(read_ns);
        println!("SAMPLE\tB6\t{variant}\tbest_price_depth_read\t{run}\t{read_ns}\t{depth}");

        let started = Instant::now();
        let snapshot = black_box(book.snapshot_copy()?);
        let snapshot_ns = started.elapsed().as_nanos();
        phases[3].push(snapshot_ns);
        println!("SAMPLE\tB6\t{variant}\tsnapshot_copy\t{run}\t{snapshot_ns}\t{snapshot}");
        if compact {
            println!(
                "META\tB6\t{variant}\tcage_live_after_snapshot\t{}",
                CompactRuntime::used_bytes()?
            );
        }

        let started = Instant::now();
        drop(book);
        let drop_ns = started.elapsed().as_nanos();
        phases[4].push(drop_ns);
        let total = end_to_end.elapsed().as_nanos();
        phases[5].push(total);
        println!("SAMPLE\tB6\t{variant}\tdrop\t{run}\t{drop_ns}");
        println!("SAMPLE\tB6\t{variant}\tend_to_end\t{run}\t{total}");

        let checksum = depth ^ snapshot;
        if expected
            .replace(checksum)
            .is_some_and(|previous| previous != checksum)
        {
            return Err("B6 produced inconsistent checksums across runs".into());
        }
    }
    let phase_names = [
        "build",
        "quote_updates_retain_snapshot_rebuild",
        "best_price_depth_read",
        "snapshot_copy",
        "drop",
        "end_to_end",
    ];
    for (index, phase) in phase_names.into_iter().enumerate() {
        emit_summary("B6", variant, phase, &mut phases[index]);
    }
    emit_summary("B6", variant, "indexed_quote_updates", &mut updates);
    emit_summary("B6", variant, "retain", &mut retains);
    emit_summary("B6", variant, "snapshot_rebuild", &mut rebuilds);
    Ok(())
}

fn emit_summary(scenario: &str, variant: &str, phase: &str, samples: &mut [u128]) {
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    let p95 = samples[((samples.len() * 95).div_ceil(100)).saturating_sub(1)];
    println!(
        "SUMMARY\t{scenario}\t{variant}\t{phase}\t{}\t{median}\t{p95}\t{}\t{}",
        samples.len(),
        samples[0],
        samples[samples.len() - 1]
    );
}

fn emit_memory(scenario: &str, variant: &str, compact: bool) {
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                let value = line.strip_prefix("VmRSS:")?.split_whitespace().next()?;
                value.parse::<u64>().ok()
            })
        });
    let peak_rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                let value = line.strip_prefix("VmHWM:")?.split_whitespace().next()?;
                value.parse::<u64>().ok()
            })
        });
    let used = if compact {
        CompactRuntime::used_bytes().ok()
    } else {
        None
    };
    println!(
        "META\t{scenario}\t{variant}\tretained_cage_bytes\t{}",
        used.map_or_else(|| "na".to_owned(), |value| value.to_string())
    );
    println!(
        "META\t{scenario}\t{variant}\tprocess_rss_kb\t{}",
        rss.map_or_else(|| "na".to_owned(), |value| value.to_string())
    );
    println!(
        "META\t{scenario}\t{variant}\tprocess_peak_rss_kb\t{}",
        peak_rss.map_or_else(|| "na".to_owned(), |value| value.to_string())
    );
}
