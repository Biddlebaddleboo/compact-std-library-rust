//! Randomized cage-backed hash map and set.

use crate::hash_control::{self, WIDTH as CONTROL_GROUP_WIDTH};
use compact_backend_std::{CageAllocation, CompactRuntime};
use compact_core::CompactValue;
use core::borrow::Borrow;
use core::hash::{BuildHasher, Hash, Hasher};
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use std::collections::hash_map::RandomState;

use crate::{CollectionError, Result};

const EMPTY: u8 = 0;
const FULL: u8 = 1;
const TOMBSTONE: u8 = 2;

fn classify_control_group(
    control: &[u8],
    start: usize,
    count: usize,
) -> hash_control::ControlGroupMask {
    if count == CONTROL_GROUP_WIDTH && start <= control.len() - CONTROL_GROUP_WIDTH {
        // The complete group is within the table, so let the classifier read
        // the control bytes directly without lane-by-lane scratch copying.
        let contiguous: &[u8; CONTROL_GROUP_WIDTH] = control[start..start + CONTROL_GROUP_WIDTH]
            .try_into()
            .expect("full control group has the requested width");
        return hash_control::classify(contiguous);
    }

    let mut group = [FULL; CONTROL_GROUP_WIDTH];
    let first_count = count.min(control.len() - start);
    group[..first_count].copy_from_slice(&control[start..start + first_count]);
    if first_count < count {
        group[first_count..count].copy_from_slice(&control[..count - first_count]);
    }
    hash_control::classify(&group)
}

fn first_empty_slot(control: &[u8], start: usize) -> Option<usize> {
    let mask = control.len().checked_sub(1)?;
    let mut consumed = 0;
    while consumed < control.len() {
        let count = (control.len() - consumed).min(CONTROL_GROUP_WIDTH);
        let cursor = start.wrapping_add(consumed) & mask;
        let classes = classify_control_group(control, cursor, count);
        let active = lane_mask(count);
        let empty = classes.empty & active;
        if empty != 0 {
            let lane = empty.trailing_zeros() as usize;
            return Some(cursor.wrapping_add(lane) & mask);
        }
        consumed += count;
    }
    None
}

fn lane_mask(count: usize) -> u16 {
    if count == CONTROL_GROUP_WIDTH {
        u16::MAX
    } else {
        (1_u16 << count) - 1
    }
}

/// Compact SipHash key pair used by default to retain randomized hashing.
#[derive(Clone, Copy, Debug)]
pub struct CompactBuildHasher {
    k0: u64,
    k1: u64,
}
impl CompactBuildHasher {
    /// Create fresh process-randomized hash keys.
    pub fn new() -> Self {
        let random = RandomState::new();
        let mut hasher = random.build_hasher();
        hasher.write_u8(0);
        let k0 = hasher.finish();
        hasher.write_u8(1);
        let k1 = hasher.finish();
        Self { k0, k1 }
    }
}
impl Default for CompactBuildHasher {
    fn default() -> Self {
        Self::new()
    }
}
impl BuildHasher for CompactBuildHasher {
    type Hasher = SipHasher24;
    fn build_hasher(&self) -> Self::Hasher {
        SipHasher24::new(self.k0, self.k1)
    }
}
// SAFETY: the builder retains only two integer hash keys.
unsafe impl CompactValue for CompactBuildHasher {}

/// SipHash-2-4 state used transiently while hashing a key.
#[derive(Clone, Copy)]
pub struct SipHasher24 {
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    tail: u64,
    tail_len: u8,
    length: u64,
}
impl SipHasher24 {
    fn new(k0: u64, k1: u64) -> Self {
        Self {
            v0: 0x736f6d6570736575 ^ k0,
            v1: 0x646f72616e646f6d ^ k1,
            v2: 0x6c7967656e657261 ^ k0,
            v3: 0x7465646279746573 ^ k1,
            tail: 0,
            tail_len: 0,
            length: 0,
        }
    }
    fn round(&mut self) {
        self.v0 = self.v0.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(13);
        self.v1 ^= self.v0;
        self.v0 = self.v0.rotate_left(32);
        self.v2 = self.v2.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(16);
        self.v3 ^= self.v2;
        self.v0 = self.v0.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(21);
        self.v3 ^= self.v0;
        self.v2 = self.v2.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(17);
        self.v1 ^= self.v2;
        self.v2 = self.v2.rotate_left(32);
    }
    fn compress(&mut self, word: u64) {
        self.v3 ^= word;
        self.round();
        self.round();
        self.v0 ^= word;
    }
}
impl Hasher for SipHasher24 {
    fn finish(&self) -> u64 {
        let mut state = *self;
        let final_word = state.tail | ((state.length & 0xff) << 56);
        state.compress(final_word);
        state.v2 ^= 0xff;
        for _ in 0..4 {
            state.round();
        }
        state.v0 ^ state.v1 ^ state.v2 ^ state.v3
    }
    fn write(&mut self, bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);
        for byte in bytes {
            self.tail |= (*byte as u64) << (self.tail_len * 8);
            self.tail_len += 1;
            if self.tail_len == 8 {
                self.compress(self.tail);
                self.tail = 0;
                self.tail_len = 0;
            }
        }
    }
}

/// Open-addressed compact map with randomized hashing and cage-backed tables.
pub struct CompactHashMap<
    K: CompactValue + Hash + Eq,
    V: CompactValue,
    S: BuildHasher + CompactValue = CompactBuildHasher,
> {
    control: Option<CageAllocation<u8>>,
    entries: Option<CageAllocation<MaybeUninit<(K, V)>>>,
    len: u32,
    tombstones: u32,
    hash_builder: S,
}

impl<K: CompactValue + Hash + Eq, V: CompactValue, S: BuildHasher + CompactValue>
    CompactHashMap<K, V, S>
{
    /// Construct an empty map with a custom randomized or deterministic hasher.
    pub fn with_hasher(hash_builder: S) -> Self {
        Self {
            control: None,
            entries: None,
            len: 0,
            tombstones: 0,
            hash_builder,
        }
    }
    /// Construct a map with initial capacity and a custom hasher.
    pub fn with_capacity_and_hasher(capacity: usize, hash_builder: S) -> Result<Self> {
        let mut map = Self::with_hasher(hash_builder);
        map.reserve(capacity)?;
        Ok(map)
    }
    /// Return the number of key-value pairs.
    pub const fn len(&self) -> usize {
        self.len as usize
    }
    /// Return the number of available hash slots.
    pub fn capacity(&self) -> usize {
        self.control.as_ref().map_or(0, |c| c.len())
    }
    /// Return whether the map is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Borrow the hash builder.
    pub const fn hasher(&self) -> &S {
        &self.hash_builder
    }

    /// Insert a key-value pair, returning the previous value when replacing.
    pub fn insert(&mut self, key: K, value: V) -> Result<Option<V>> {
        self.ensure_insert_capacity()?;
        let hash = self.hash(&key);
        if let Some((index, found)) = self.find_slot(&key, hash) {
            if found {
                let pair = unsafe {
                    self.entries.as_mut().unwrap().as_mut_slice()[index].assume_init_mut()
                };
                return Ok(Some(core::mem::replace(&mut pair.1, value)));
            }
            let control = &mut self.control.as_mut().unwrap().as_mut_slice()[index];
            if *control == TOMBSTONE {
                self.tombstones -= 1;
            }
            self.entries.as_mut().unwrap().as_mut_slice()[index].write((key, value));
            *control = FULL;
            self.len += 1;
            return Ok(None);
        }
        Err(CollectionError::Core(
            compact_core::Error::AllocationExhausted,
        ))
    }

    /// Return a value by key.
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let control = self.control.as_ref()?.as_slice();
        let entries = self.entries.as_ref()?.as_slice();
        let (index, found) = Self::find_slot_in(control, entries, key, self.hash(key))?;
        found.then(|| {
            // SAFETY: FULL control state corresponds to one initialized pair.
            unsafe { &entries[index].assume_init_ref().1 }
        })
    }
    /// Return key and value by borrowed key.
    pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&K, &V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let control = self.control.as_ref()?.as_slice();
        let entries = self.entries.as_ref()?.as_slice();
        let (index, found) = Self::find_slot_in(control, entries, key, self.hash(key))?;
        found.then(|| {
            // SAFETY: FULL control state corresponds to one initialized pair.
            let pair = unsafe { entries[index].assume_init_ref() };
            (&pair.0, &pair.1)
        })
    }
    /// Mutably borrow a value by key.
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let hash = self.hash(key);
        let (index, found) = self.find_slot(key, hash)?;
        found.then(|| {
            // SAFETY: the map is exclusively borrowed and the slot is initialized.
            unsafe {
                &mut self.entries.as_mut().unwrap().as_mut_slice()[index]
                    .assume_init_mut()
                    .1
            }
        })
    }
    /// Return whether a key is present.
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.get(key).is_some()
    }
    /// Remove a pair and return its key and value.
    pub fn remove_entry<Q>(&mut self, key: &Q) -> Option<(K, V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let hash = self.hash(key);
        let (index, found) = self.find_slot(key, hash)?;
        if !found {
            return None;
        }
        self.control.as_mut().unwrap().as_mut_slice()[index] = TOMBSTONE;
        self.len -= 1;
        self.tombstones += 1;
        // SAFETY: control state was FULL and the slot is now logically vacant.
        Some(unsafe { self.entries.as_mut().unwrap().as_mut_slice()[index].assume_init_read() })
    }
    /// Remove a key and return its value.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.remove_entry(key).map(|(_, value)| value)
    }
    /// Drop every pair while retaining table capacity.
    pub fn clear(&mut self) {
        CompactRuntime::with_batched_releases(|| {
            let Some(control) = &mut self.control else {
                return;
            };
            let controls = control.as_mut_slice();
            let entries = self
                .entries
                .as_mut()
                .expect("allocated control has entries")
                .as_mut_slice();
            if !core::mem::needs_drop::<(K, V)>() {
                for control in controls {
                    if *control == FULL {
                        *control = EMPTY;
                    }
                }
                self.len = 0;
                self.tombstones = 0;
                return;
            }
            for index in 0..controls.len() {
                if controls[index] == FULL {
                    controls[index] = EMPTY;
                    self.len -= 1;
                    // SAFETY: the control byte marks one initialized pair and
                    // is cleared before either destructor can unwind.
                    let pair = unsafe { entries[index].assume_init_read() };
                    drop(pair);
                }
            }
            debug_assert_eq!(self.len, 0);
            self.len = 0;
            self.tombstones = 0;
        });
    }
    /// Ensure room for at least `additional` more entries.
    pub fn reserve(&mut self, additional: usize) -> Result<()> {
        let required = (self.len as usize)
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        let current_capacity = self.capacity();
        let mut slots = current_capacity.max(8);
        while required.saturating_mul(8) >= slots.saturating_mul(7) {
            slots = slots
                .checked_mul(2)
                .ok_or(CollectionError::CapacityOverflow)?;
        }
        if current_capacity == 0
            || slots > current_capacity
            || self.tombstones as usize > current_capacity / 4
        {
            self.rehash(slots)?;
        }
        Ok(())
    }
    /// Release unused slots.
    pub fn shrink_to_fit(&mut self) -> Result<()> {
        if self.len == 0 {
            CompactRuntime::with_batched_releases(|| {
                drop(self.entries.take());
                drop(self.control.take());
            });
            self.tombstones = 0;
            return Ok(());
        }
        let mut slots = 8_usize;
        while (self.len as usize).saturating_mul(8) >= slots.saturating_mul(7) {
            slots = slots
                .checked_mul(2)
                .ok_or(CollectionError::CapacityOverflow)?;
        }
        if slots != self.capacity() || self.tombstones != 0 {
            self.rehash(slots)?;
        }
        Ok(())
    }
    /// Iterate over key-value pairs.
    pub fn iter(&self) -> CompactHashMapIter<'_, K, V> {
        CompactHashMapIter {
            control: self.control.as_ref().map_or(&[], |c| c.as_slice()),
            entries: self.entries.as_ref().map_or(&[], |e| e.as_slice()),
            index: 0,
            remaining: self.len as usize,
        }
    }
    /// Mutably iterate over values and immutably borrow keys.
    pub fn iter_mut(&mut self) -> CompactHashMapIterMut<'_, K, V> {
        let control = self.control.as_ref().map_or(&[][..], |c| c.as_slice());
        let entries = self
            .entries
            .as_mut()
            .map_or(&mut [][..], |e| e.as_mut_slice());
        CompactHashMapIterMut {
            control,
            entries: entries.as_mut_ptr(),
            slots: entries.len(),
            index: 0,
            remaining: self.len as usize,
            marker: PhantomData,
        }
    }
    /// Iterate over keys.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(key, _)| key)
    }
    /// Iterate over values.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, value)| value)
    }
    /// Mutably iterate over values.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.iter_mut().map(|(_, value)| value)
    }
    /// Retain entries accepted by `keep`.
    pub fn retain<F: FnMut(&K, &mut V) -> bool>(&mut self, mut keep: F) {
        let Some(control) = &mut self.control else {
            return;
        };
        let controls = control.as_mut_slice();
        let entries = self
            .entries
            .as_mut()
            .expect("allocated control has entries")
            .as_mut_slice();
        for index in 0..controls.len() {
            if controls[index] == FULL {
                // SAFETY: the map is exclusively borrowed and the slot is live.
                let pair = unsafe { entries[index].assume_init_mut() };
                if !keep(&pair.0, &mut pair.1) {
                    controls[index] = TOMBSTONE;
                    self.len -= 1;
                    self.tombstones += 1;
                    // SAFETY: control was made vacant and the pair is moved out exactly once.
                    let pair = unsafe { entries[index].assume_init_read() };
                    drop(pair);
                }
            }
        }
    }

    fn hash<Q: Hash + ?Sized>(&self, key: &Q) -> u64 {
        self.hash_builder.hash_one(key)
    }
    fn find_slot<Q>(&self, key: &Q, hash: u64) -> Option<(usize, bool)>
    where
        K: Borrow<Q>,
        Q: Eq + ?Sized,
    {
        let control = self.control.as_ref()?.as_slice();
        let entries = self.entries.as_ref()?.as_slice();
        Self::find_slot_in(control, entries, key, hash)
    }

    fn find_slot_in<Q>(
        control: &[u8],
        entries: &[MaybeUninit<(K, V)>],
        key: &Q,
        hash: u64,
    ) -> Option<(usize, bool)>
    where
        K: Borrow<Q>,
        Q: Eq + ?Sized,
    {
        let mask = control.len().checked_sub(1)?;
        let mut first_tombstone = None;
        let start = hash as usize & mask;
        let mut consumed = 0;
        while consumed < control.len() {
            let count = (control.len() - consumed).min(CONTROL_GROUP_WIDTH);
            let cursor = start.wrapping_add(consumed) & mask;
            let classes = classify_control_group(control, cursor, count);
            let active = lane_mask(count);
            let empty = classes.empty & active;
            let full = classes.full & active;
            let tombstone = classes.tombstone & active;
            if active & !(empty | full | tombstone) != 0 {
                unreachable!("control state is an internal two-bit invariant");
            }

            let first_empty = if empty == 0 {
                count
            } else {
                empty.trailing_zeros() as usize
            };
            let before_empty = lane_mask(first_empty);
            let mut full_candidates = full & before_empty;
            while full_candidates != 0 {
                let lane = full_candidates.trailing_zeros() as usize;
                full_candidates &= full_candidates - 1;
                let index = cursor.wrapping_add(lane) & mask;
                // SAFETY: FULL slots contain initialized key-value pairs.
                let pair = unsafe { entries[index].assume_init_ref() };
                if pair.0.borrow() == key {
                    return Some((index, true));
                }
            }
            let earlier_tombstones = tombstone & before_empty;
            if first_tombstone.is_none() && earlier_tombstones != 0 {
                let lane = earlier_tombstones.trailing_zeros() as usize;
                first_tombstone = Some(cursor.wrapping_add(lane) & mask);
            }
            if first_empty < count {
                let empty_index = cursor.wrapping_add(first_empty) & mask;
                return Some((first_tombstone.unwrap_or(empty_index), false));
            }
            consumed += count;
        }
        first_tombstone.map(|index| (index, false))
    }

    #[cfg(test)]
    fn find_slot_scalar<Q>(&self, key: &Q, hash: u64) -> Option<(usize, bool)>
    where
        K: Borrow<Q>,
        Q: Eq + ?Sized,
    {
        let control = self.control.as_ref()?.as_slice();
        let entries = self.entries.as_ref()?.as_slice();
        let mask = control.len().checked_sub(1)?;
        let mut first_tombstone = None;
        let start = hash as usize & mask;
        for step in 0..control.len() {
            let index = start.wrapping_add(step) & mask;
            match control[index] {
                EMPTY => return Some((first_tombstone.unwrap_or(index), false)),
                TOMBSTONE => {
                    first_tombstone.get_or_insert(index);
                }
                FULL => {
                    // SAFETY: FULL slots contain initialized key-value pairs.
                    let pair = unsafe { entries[index].assume_init_ref() };
                    if pair.0.borrow() == key {
                        return Some((index, true));
                    }
                }
                _ => unreachable!("control state is an internal two-bit invariant"),
            };
        }
        first_tombstone.map(|index| (index, false))
    }
    fn ensure_insert_capacity(&mut self) -> Result<()> {
        let capacity = self.capacity();
        if capacity == 0
            || ((self.len as usize + self.tombstones as usize + 1).saturating_mul(8))
                >= capacity.saturating_mul(7)
        {
            self.reserve(1)?;
        }
        Ok(())
    }
    fn rehash(&mut self, slots: usize) -> Result<()> {
        let slots = slots
            .max(8)
            .checked_next_power_of_two()
            .ok_or(CollectionError::CapacityOverflow)?;
        let (mut new_control, mut new_entries) = empty_table::<K, V>(slots)?;
        let old_control = self.control.as_ref().map_or(&[][..], |c| c.as_slice());
        let old_entries = self.entries.as_ref().map_or(&[][..], |e| e.as_slice());
        let mut destinations = Vec::<usize>::new();
        destinations
            .try_reserve_exact(old_control.len())
            .map_err(|_| CollectionError::Core(compact_core::Error::AllocationFailed))?;
        destinations.resize(old_control.len(), usize::MAX);
        let mask = slots - 1;
        let new_control_slice = new_control.as_mut_slice();
        for old_index in 0..old_control.len() {
            if old_control[old_index] != FULL {
                continue;
            }
            // SAFETY: FULL slots contain initialized pairs.
            let pair = unsafe { old_entries[old_index].assume_init_ref() };
            let hash = self.hash(&pair.0);
            let start = hash as usize & mask;
            let index = first_empty_slot(new_control_slice, start).ok_or(CollectionError::Core(
                compact_core::Error::AllocationExhausted,
            ))?;
            new_control_slice[index] = FULL;
            destinations[old_index] = index;
        }
        // No user code or fallible operation remains; transfer all pairs.
        let new_entries_slice = new_entries.as_mut_slice();
        if let Some(entries) = &mut self.entries {
            let entries = entries.as_mut_slice();
            for (old_index, target) in destinations.iter().copied().enumerate() {
                if target == usize::MAX {
                    continue;
                }
                // SAFETY: this pair is moved exactly once into an empty slot.
                let pair = unsafe { entries[old_index].assume_init_read() };
                new_entries_slice[target].write(pair);
            }
        }
        CompactRuntime::with_batched_releases(|| {
            drop(self.control.replace(new_control));
            drop(self.entries.replace(new_entries));
        });
        self.tombstones = 0;
        Ok(())
    }
}

type EntryStorage<K, V> = CageAllocation<MaybeUninit<(K, V)>>;
type TableStorage<K, V> = (CageAllocation<u8>, EntryStorage<K, V>);

fn empty_table<K: CompactValue, V: CompactValue>(slots: usize) -> Result<TableStorage<K, V>> {
    let mut control = CompactRuntime::alloc_owned_slice::<u8>(slots)?;
    control.extend_from_fn(slots, || EMPTY)?;
    let mut entries = CompactRuntime::alloc_owned_slice::<MaybeUninit<(K, V)>>(slots)?;
    entries.extend_from_fn(slots, MaybeUninit::uninit)?;
    Ok((control, entries))
}

struct MapDropGuard<K: CompactValue + Hash + Eq, V: CompactValue, S: BuildHasher + CompactValue> {
    map: *mut CompactHashMap<K, V, S>,
    armed: bool,
}
impl<K: CompactValue + Hash + Eq, V: CompactValue, S: BuildHasher + CompactValue> Drop
    for MapDropGuard<K, V, S>
{
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // SAFETY: guard is created from an exclusive borrow during map drop.
        let map = unsafe { &mut *self.map };
        if let Some(control) = &mut map.control {
            let controls = control.as_mut_slice();
            let entries = map
                .entries
                .as_mut()
                .expect("allocated control has entries")
                .as_mut_slice();
            for index in 0..controls.len() {
                if controls[index] == FULL {
                    controls[index] = EMPTY;
                    map.len -= 1;
                    // SAFETY: FULL marked an initialized pair and is cleared before its destructor.
                    let pair = unsafe { entries[index].assume_init_read() };
                    drop(pair);
                }
            }
        }
        map.len = 0;
        map.tombstones = 0;
        drop(map.entries.take());
        drop(map.control.take());
    }
}
impl<K: CompactValue + Hash + Eq, V: CompactValue, S: BuildHasher + CompactValue> Drop
    for CompactHashMap<K, V, S>
{
    fn drop(&mut self) {
        CompactRuntime::with_batched_releases(|| {
            let mut guard = MapDropGuard {
                map: self,
                armed: true,
            };
            self.clear();
            guard.armed = false;
            drop(self.entries.take());
            drop(self.control.take());
        });
    }
}
// SAFETY: keys and values obey CompactValue and the hasher is required to be a compact value.
unsafe impl<K: CompactValue + Hash + Eq, V: CompactValue, S: BuildHasher + CompactValue>
    CompactValue for CompactHashMap<K, V, S>
{
}

impl<K: CompactValue + Hash + Eq, V: CompactValue> CompactHashMap<K, V, CompactBuildHasher> {
    /// Construct an empty map with randomized hashing.
    pub fn new() -> Self {
        Self::with_hasher(CompactBuildHasher::new())
    }
    /// Construct a map with randomized hashing and initial capacity.
    pub fn with_capacity(capacity: usize) -> Result<Self> {
        Self::with_capacity_and_hasher(capacity, CompactBuildHasher::new())
    }
}
impl<K: CompactValue + Hash + Eq, V: CompactValue> Default
    for CompactHashMap<K, V, CompactBuildHasher>
{
    fn default() -> Self {
        Self::new()
    }
}

/// Immutable iterator over a compact hash map.
pub struct CompactHashMapIter<'a, K, V> {
    control: &'a [u8],
    entries: &'a [MaybeUninit<(K, V)>],
    index: usize,
    remaining: usize,
}
impl<'a, K, V> Iterator for CompactHashMapIter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.control.len() {
            let index = self.index;
            self.index += 1;
            if self.control[index] == FULL {
                self.remaining -= 1;
                // SAFETY: FULL entries are initialized and this iterator holds the map borrow.
                let pair = unsafe { self.entries[index].assume_init_ref() };
                return Some((&pair.0, &pair.1));
            }
        }
        None
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl<K, V> ExactSizeIterator for CompactHashMapIter<'_, K, V> {}
impl<K, V> core::iter::FusedIterator for CompactHashMapIter<'_, K, V> {}

/// Mutable iterator over key-value pairs.
pub struct CompactHashMapIterMut<'a, K, V> {
    control: &'a [u8],
    entries: *mut MaybeUninit<(K, V)>,
    slots: usize,
    index: usize,
    remaining: usize,
    marker: PhantomData<&'a mut (K, V)>,
}
impl<'a, K, V> Iterator for CompactHashMapIterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);
    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.slots {
            let index = self.index;
            self.index += 1;
            if self.control[index] == FULL {
                self.remaining -= 1;
                // SAFETY: the exclusive map borrow is held for 'a; each control slot is visited once.
                let pair: &'a mut (K, V) =
                    unsafe { (&mut *self.entries.add(index)).assume_init_mut() };
                return Some((&pair.0, &mut pair.1));
            }
        }
        None
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl<K, V> ExactSizeIterator for CompactHashMapIterMut<'_, K, V> {}
impl<K, V> core::iter::FusedIterator for CompactHashMapIterMut<'_, K, V> {}

/// Randomized cage-backed set.
pub struct CompactHashSet<
    T: CompactValue + Hash + Eq,
    S: BuildHasher + CompactValue = CompactBuildHasher,
> {
    map: CompactHashMap<T, (), S>,
}
impl<T: CompactValue + Hash + Eq, S: BuildHasher + CompactValue> CompactHashSet<T, S> {
    /// Create an empty set with a custom hasher.
    pub fn with_hasher(hash_builder: S) -> Self {
        Self {
            map: CompactHashMap::with_hasher(hash_builder),
        }
    }
    /// Create a set with initial capacity and a custom hasher.
    pub fn with_capacity_and_hasher(capacity: usize, hash_builder: S) -> Result<Self> {
        Ok(Self {
            map: CompactHashMap::with_capacity_and_hasher(capacity, hash_builder)?,
        })
    }
    /// Return the element count.
    pub const fn len(&self) -> usize {
        self.map.len()
    }
    /// Return slot capacity.
    pub fn capacity(&self) -> usize {
        self.map.capacity()
    }
    /// Return whether the set is empty.
    pub const fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    /// Insert a value and report whether it was new.
    pub fn insert(&mut self, value: T) -> Result<bool> {
        Ok(self.map.insert(value, ())?.is_none())
    }
    /// Return a stored value equivalent to `value`.
    pub fn get<Q>(&self, value: &Q) -> Option<&T>
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.get_key_value(value).map(|(key, _)| key)
    }
    /// Return whether the value is present.
    pub fn contains<Q>(&self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.contains_key(value)
    }
    /// Remove a value.
    pub fn remove<Q>(&mut self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.remove(value).is_some()
    }
    /// Reserve room for additional values.
    pub fn reserve(&mut self, additional: usize) -> Result<()> {
        self.map.reserve(additional)
    }
    /// Drop all values while retaining table capacity.
    pub fn clear(&mut self) {
        self.map.clear();
    }
    /// Iterate over stored values.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.map.keys()
    }
    /// Retain values accepted by `keep`.
    pub fn retain<F: FnMut(&T) -> bool>(&mut self, mut keep: F) {
        self.map.retain(|key, _| keep(key));
    }
}
impl<T: CompactValue + Hash + Eq> CompactHashSet<T, CompactBuildHasher> {
    /// Create an empty randomized set.
    pub fn new() -> Self {
        Self::with_hasher(CompactBuildHasher::new())
    }
    /// Create a randomized set with initial capacity.
    pub fn with_capacity(capacity: usize) -> Result<Self> {
        Self::with_capacity_and_hasher(capacity, CompactBuildHasher::new())
    }
}
impl<T: CompactValue + Hash + Eq> Default for CompactHashSet<T, CompactBuildHasher> {
    fn default() -> Self {
        Self::new()
    }
}
// SAFETY: all elements and the hasher are compact values.
unsafe impl<T: CompactValue + Hash + Eq, S: BuildHasher + CompactValue> CompactValue
    for CompactHashSet<T, S>
{
}

#[cfg(test)]
mod tests {
    use super::{classify_control_group, CompactHashMap, CompactValue, EMPTY, FULL, TOMBSTONE};
    use crate::hash_control::{self, ControlGroupMask, WIDTH};
    use compact_backend_std::{CageConfig, CompactRuntime};
    use core::hash::{BuildHasher, Hasher};
    use std::sync::OnceLock;

    static INIT: OnceLock<()> = OnceLock::new();

    fn init() {
        INIT.get_or_init(|| {
            CompactRuntime::init(CageConfig::new(32 * 1024 * 1024)).unwrap();
        });
    }

    #[derive(Clone, Copy, Default)]
    struct IdentityBuildHasher;
    struct IdentityHasher(u64);

    impl Hasher for IdentityHasher {
        fn finish(&self) -> u64 {
            self.0
        }
        fn write(&mut self, bytes: &[u8]) {
            self.0 = bytes
                .iter()
                .take(8)
                .enumerate()
                .fold(0_u64, |hash, (index, byte)| {
                    hash | (u64::from(*byte) << (index * 8))
                });
        }
        fn write_u32(&mut self, value: u32) {
            self.0 = u64::from(value);
        }
    }
    impl BuildHasher for IdentityBuildHasher {
        type Hasher = IdentityHasher;
        fn build_hasher(&self) -> Self::Hasher {
            IdentityHasher(0)
        }
    }
    // SAFETY: this is a zero-sized native hasher builder.
    unsafe impl CompactValue for IdentityBuildHasher {}

    #[test]
    fn control_group_access_matches_lane_reference_for_every_wrap_and_partial_range() {
        let states = [EMPTY, FULL, TOMBSTONE, 3];
        for length in [8_usize, 16, 23, 24, 32] {
            let control: Vec<u8> = (0..length)
                .map(|index| states[(index * 3 + index / 2) % states.len()])
                .collect();
            for start in 0..length {
                for count in 1..=length.min(WIDTH) {
                    let mut expected = [FULL; WIDTH];
                    for lane in 0..count {
                        expected[lane] = control[(start + lane) % length];
                    }
                    let expected = hash_control::classify(&expected);
                    let actual = classify_control_group(&control, start, count);
                    assert_eq!(
                        actual, expected,
                        "length={length}, start={start}, count={count}"
                    );
                }
            }
        }

        let mut all_states = [0_u8; WIDTH];
        for (lane, state) in all_states.iter_mut().enumerate() {
            *state = states[lane % states.len()];
        }
        assert_eq!(
            classify_control_group(&all_states, 0, WIDTH),
            ControlGroupMask {
                empty: 0x1111,
                full: 0x2222,
                tombstone: 0x4444,
            }
        );
    }

    fn assert_probe_matches_scalar(map: &CompactHashMap<u32, u32, IdentityBuildHasher>) {
        let capacity = map.capacity() as u32;
        for key in 0..capacity.saturating_mul(8).saturating_add(128) {
            let hash = map.hash(&key);
            assert_eq!(map.find_slot(&key, hash), map.find_slot_scalar(&key, hash));
        }
    }

    #[test]
    fn grouped_probe_matches_scalar_across_wrap_and_tombstones() {
        init();
        let mut small = CompactHashMap::with_capacity_and_hasher(3, IdentityBuildHasher).unwrap();
        let small_capacity = small.capacity() as u32;
        let small_start = small_capacity - 2;
        for index in 0..5_u32 {
            let key = small_start + index * small_capacity;
            small.insert(key, key).unwrap();
        }
        assert_probe_matches_scalar(&small);

        let mut map = CompactHashMap::with_capacity_and_hasher(20, IdentityBuildHasher).unwrap();
        let capacity = map.capacity() as u32;
        let start = capacity - 3;
        let keys: Vec<u32> = (0..20).map(|index| start + index * capacity).collect();
        for key in keys.iter().copied() {
            map.insert(key, key ^ 0x5a5a).unwrap();
        }
        assert_probe_matches_scalar(&map);

        for key in keys.iter().step_by(3).copied() {
            assert_eq!(map.remove(&key), Some(key ^ 0x5a5a));
        }
        assert_probe_matches_scalar(&map);

        for index in 0..8_u32 {
            let key = start + (100 + index) * capacity;
            map.insert(key, key ^ 0xa5a5).unwrap();
        }
        assert_probe_matches_scalar(&map);
    }
}
