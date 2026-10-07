//! Arena-backed open-addressed hash map.

use compact_core::{Arena, ArenaAllocation, CompactValue};
use core::borrow::Borrow;
use core::hash::Hash;
use core::mem::{self, MaybeUninit};
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

use crate::{CollectionError, Result};

const EMPTY: u8 = 0;
const OCCUPIED: u8 = 1;
const DELETED: u8 = 2;
const MIN_SLOTS: usize = 8;

/// An arena-backed open-addressed map with randomized hashing by default.
///
/// Control bytes and entry slots use separate arena allocations. Removed
/// entries become tombstones until a later rehash clears them.
pub struct CompactHashMap<'arena, K: CompactValue, V: CompactValue, S: BuildHasher = RandomState> {
    control: Option<ArenaAllocation<'arena, u8>>,
    entries: Option<ArenaAllocation<'arena, MaybeUninit<(K, V)>>>,
    len: usize,
    deleted: usize,
    hash_builder: S,
}

impl<'arena, K: CompactValue, V: CompactValue, S: BuildHasher> CompactHashMap<'arena, K, V, S> {
    /// Construct an empty map with the supplied hash builder.
    pub fn with_hasher(hash_builder: S) -> Self {
        Self {
            control: None,
            entries: None,
            len: 0,
            deleted: 0,
            hash_builder,
        }
    }

    /// Construct a map with room for at least `capacity` entries.
    pub fn with_capacity_and_hasher(
        capacity: usize,
        hash_builder: S,
        arena: &mut Arena<'arena, '_>,
    ) -> Result<Self> {
        let slots = slots_for_entries(capacity)?;
        let mut map = Self::with_hasher(hash_builder);
        if slots != 0 {
            let allocations = allocate_table(slots, arena)?;
            map.control = Some(allocations.control);
            map.entries = Some(allocations.entries);
        }
        Ok(map)
    }

    /// Return the number of stored key-value pairs.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return the number of entries the current table can hold before growth.
    pub fn capacity(&self) -> usize {
        max_entries(self.slot_count())
    }

    /// Return whether the map contains no pairs.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Borrow the active hash builder.
    pub const fn hasher(&self) -> &S {
        &self.hash_builder
    }

    /// Insert a key-value pair and return the previous value, if the key was
    /// already present.
    pub fn insert(&mut self, key: K, value: V, arena: &mut Arena<'arena, '_>) -> Result<Option<V>>
    where
        K: Hash + Eq,
    {
        self.insert_with_index(key, value, arena)
            .map(|(old, _index)| old)
    }

    /// Borrow the value stored for `key`.
    pub fn get<Q>(&self, key: &Q, arena: &Arena<'arena, '_>) -> Result<Option<&V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let Some(index) = self.find_index(key, arena)? else {
            return Ok(None);
        };
        // SAFETY: find_index returns only a currently occupied table slot.
        let pair = unsafe {
            self.entries
                .as_ref()
                .expect("occupied map has entry storage")
                .as_slice()[index]
                .assume_init_ref()
        };
        Ok(Some(&pair.1))
    }

    /// Borrow both the stored key and value for `key`.
    pub fn get_key_value<Q>(&self, key: &Q, arena: &Arena<'arena, '_>) -> Result<Option<(&K, &V)>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let Some(index) = self.find_index(key, arena)? else {
            return Ok(None);
        };
        // SAFETY: find_index returns only a currently occupied table slot.
        let pair = unsafe {
            self.entries
                .as_ref()
                .expect("occupied map has entry storage")
                .as_slice()[index]
                .assume_init_ref()
        };
        Ok(Some((&pair.0, &pair.1)))
    }

    /// Mutably borrow the value stored for `key`.
    pub fn get_mut<'view, Q>(
        &'view mut self,
        key: &Q,
        arena: &Arena<'arena, '_>,
    ) -> Result<Option<&'view mut V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let Some(index) = self.find_index(key, arena)? else {
            return Ok(None);
        };
        // SAFETY: find_index identified an occupied slot before the map was
        // mutably borrowed, and this method returns its unique value borrow.
        let pair = unsafe {
            self.entries
                .as_mut()
                .expect("occupied map has entry storage")
                .as_mut_slice()[index]
                .assume_init_mut()
        };
        Ok(Some(&mut pair.1))
    }

    /// Return whether `key` is present.
    pub fn contains_key<Q>(&self, key: &Q, arena: &Arena<'arena, '_>) -> Result<bool>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        Ok(self.find_index(key, arena)?.is_some())
    }

    /// Remove a key-value pair and return both owned values.
    pub fn remove_entry<Q>(&mut self, key: &Q, arena: &Arena<'arena, '_>) -> Result<Option<(K, V)>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let Some(index) = self.find_index(key, arena)? else {
            return Ok(None);
        };
        Ok(Some(self.remove_index(index)))
    }

    /// Remove a key and return its value.
    pub fn remove<Q>(&mut self, key: &Q, arena: &Arena<'arena, '_>) -> Result<Option<V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let Some((key, value)) = self.remove_entry(key, arena)? else {
            return Ok(None);
        };
        drop(key);
        Ok(Some(value))
    }

    /// Drop all stored pairs and retain the table allocation.
    pub fn clear(&mut self) {
        self.drop_entries();
    }

    /// Ensure room for at least `additional` more entries.
    pub fn reserve(&mut self, additional: usize, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        K: Hash,
    {
        let required = self
            .len
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        u32::try_from(required).map_err(|_| CollectionError::CapacityOverflow)?;
        if required <= self.capacity() {
            return Ok(());
        }
        self.rehash(slots_for_entries(required)?, arena)
    }

    /// Reduce storage to the smallest table that can hold the current entries.
    pub fn shrink_to_fit(&mut self, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        K: Hash,
    {
        self.validate_storage(arena)?;
        let target_slots = slots_for_entries(self.len)?;
        if target_slots == self.slot_count() && self.deleted == 0 {
            return Ok(());
        }
        self.rehash(target_slots, arena)
    }

    /// Return an iterator over all key-value pairs.
    pub fn iter<'view>(
        &'view self,
        arena: &Arena<'arena, '_>,
    ) -> Result<CompactHashMapIter<'view, K, V>> {
        self.validate_storage(arena)?;
        Ok(CompactHashMapIter {
            control: self.control.as_ref().map_or(&[], ArenaAllocation::as_slice),
            entries: self.entries.as_ref().map_or(&[], ArenaAllocation::as_slice),
            next: 0,
            remaining: self.len,
        })
    }

    /// Return a mutable iterator over all key-value pairs.
    pub fn iter_mut<'view>(
        &'view mut self,
        arena: &Arena<'arena, '_>,
    ) -> Result<CompactHashMapIterMut<'view, K, V>> {
        self.validate_storage(arena)?;
        Ok(CompactHashMapIterMut {
            control: self.control.as_ref().map_or(&[], ArenaAllocation::as_slice),
            entries: self
                .entries
                .as_mut()
                .map_or(core::ptr::null_mut(), |entries| {
                    entries.as_mut_slice().as_mut_ptr()
                }),
            next: 0,
            remaining: self.len,
            marker: core::marker::PhantomData,
        })
    }

    /// Return an iterator over stored keys.
    pub fn keys<'view>(
        &'view self,
        arena: &Arena<'arena, '_>,
    ) -> Result<impl ExactSizeIterator<Item = &'view K>> {
        Ok(self.iter(arena)?.map(|(key, _)| key))
    }

    /// Return an iterator over stored values.
    pub fn values<'view>(
        &'view self,
        arena: &Arena<'arena, '_>,
    ) -> Result<impl ExactSizeIterator<Item = &'view V>> {
        Ok(self.iter(arena)?.map(|(_, value)| value))
    }

    /// Return a mutable iterator over stored values.
    pub fn values_mut<'view>(
        &'view mut self,
        arena: &Arena<'arena, '_>,
    ) -> Result<impl ExactSizeIterator<Item = &'view mut V>> {
        Ok(self.iter_mut(arena)?.map(|(_, value)| value))
    }

    /// Find or insert `key`, returning an entry handle for `or_insert` APIs.
    pub fn entry<'view, 'memory>(
        &'view mut self,
        key: K,
        arena: &'view mut Arena<'arena, 'memory>,
    ) -> Result<CompactHashMapEntry<'view, 'arena, 'memory, K, V, S>>
    where
        K: Hash + Eq,
    {
        self.validate_storage(arena)?;
        let hash = self.hash(&key);
        let probe = self.probe_with_hash(&key, hash);
        let index = probe.found;
        let key = if index.is_some() { None } else { Some(key) };
        Ok(CompactHashMapEntry {
            map: self,
            arena,
            key,
            index,
        })
    }

    /// Keep only pairs for which `keep` returns true.
    ///
    /// Hashing and equality are not called during retention. A pair is marked
    /// deleted before its values are dropped, so a panicking destructor cannot
    /// make the removed pair visible again.
    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&K, &mut V) -> bool,
    {
        let slots = self.slot_count();
        for index in 0..slots {
            if self
                .control
                .as_ref()
                .expect("nonempty table has control bytes")
                .as_slice()[index]
                != OCCUPIED
            {
                continue;
            }
            let should_keep = {
                // SAFETY: an occupied control byte identifies an initialized
                // pair, and the map is mutably borrowed for this operation.
                let pair = unsafe {
                    self.entries
                        .as_mut()
                        .expect("occupied table has entries")
                        .as_mut_slice()[index]
                        .assume_init_mut()
                };
                keep(&pair.0, &mut pair.1)
            };
            if !should_keep {
                let pair = self.remove_index(index);
                drop(pair);
            }
        }
    }

    fn insert_with_index(
        &mut self,
        key: K,
        value: V,
        arena: &mut Arena<'arena, '_>,
    ) -> Result<(Option<V>, usize)>
    where
        K: Hash + Eq,
    {
        self.validate_storage(arena)?;
        let hash = self.hash(&key);
        let initial = self.probe_with_hash(&key, hash);
        if let Some(index) = initial.found {
            return Ok((Some(self.replace_value(index, key, value)), index));
        }

        let required = self
            .len
            .checked_add(1)
            .ok_or(CollectionError::CapacityOverflow)?;
        u32::try_from(required).map_err(|_| CollectionError::CapacityOverflow)?;
        let mut probe = initial;
        if required > self.capacity() {
            self.rehash(slots_for_entries(required)?, arena)?;
            probe = self.probe_with_hash(&key, hash);
        }
        let Some(index) = probe.vacant else {
            return Err(CollectionError::Core(
                compact_core::Error::AllocationExhausted,
            ));
        };

        // The pair is fully initialized before the control byte publishes it.
        self.entries
            .as_mut()
            .expect("insertion table has entry storage")
            .as_mut_slice()[index]
            .write((key, value));
        if probe.vacant_is_deleted {
            self.deleted -= 1;
        }
        self.control
            .as_mut()
            .expect("insertion table has control bytes")
            .as_mut_slice()[index] = OCCUPIED;
        self.len += 1;
        Ok((None, index))
    }

    fn replace_value(&mut self, index: usize, key: K, value: V) -> V {
        // SAFETY: index was returned by a completed immutable probe, and no
        // table mutation occurs before this unique access.
        let pair = unsafe {
            self.entries
                .as_mut()
                .expect("occupied table has entries")
                .as_mut_slice()[index]
                .assume_init_mut()
        };
        let old_value = mem::replace(&mut pair.1, value);
        drop(key);
        old_value
    }

    fn find_index<Q>(&self, key: &Q, arena: &Arena<'arena, '_>) -> Result<Option<usize>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.validate_storage(arena)?;
        if self.slot_count() == 0 {
            return Ok(None);
        }
        let hash = self.hash(key);
        Ok(self.probe_with_hash(key, hash).found)
    }

    fn validate_storage(&self, arena: &Arena<'arena, '_>) -> Result<()> {
        if let Some(control) = &self.control {
            arena.validate_owned(control)?;
        }
        if let Some(entries) = &self.entries {
            arena.validate_owned(entries)?;
        }
        Ok(())
    }

    fn hash<Q: Hash + ?Sized>(&self, key: &Q) -> u64 {
        self.hash_builder.hash_one(key)
    }

    fn probe_with_hash<Q>(&self, key: &Q, hash: u64) -> Probe
    where
        K: Borrow<Q>,
        Q: Eq + ?Sized,
    {
        let slots = self.slot_count();
        if slots == 0 {
            return Probe::empty();
        }
        let control = self
            .control
            .as_ref()
            .expect("allocated table has control")
            .as_slice();
        let entries = self
            .entries
            .as_ref()
            .expect("allocated table has entries")
            .as_slice();
        let mask = slots - 1;
        let start = hash as usize & mask;
        let mut first_deleted = None;
        for offset in 0..slots {
            let index = (start + offset) & mask;
            match control[index] {
                EMPTY => {
                    return match first_deleted {
                        Some(deleted) => Probe::vacant(deleted, true),
                        None => Probe::vacant(index, false),
                    };
                }
                DELETED => {
                    if first_deleted.is_none() {
                        first_deleted = Some(index);
                    }
                }
                OCCUPIED => {
                    // SAFETY: OCCUPIED is written only after the entry pair
                    // has been initialized and remains set until it is moved.
                    let pair = unsafe { entries[index].assume_init_ref() };
                    if pair.0.borrow() == key {
                        return Probe::found(index);
                    }
                }
                _ => unreachable!("control byte has a valid table state"),
            }
        }
        first_deleted.map_or_else(Probe::empty, |index| Probe::vacant(index, true))
    }

    fn remove_index(&mut self, index: usize) -> (K, V) {
        self.control
            .as_mut()
            .expect("occupied table has control bytes")
            .as_mut_slice()[index] = DELETED;
        self.len -= 1;
        self.deleted += 1;
        // SAFETY: the occupied pair is moved out after its state is changed to
        // a tombstone, so it cannot be observed or dropped again by the map.
        unsafe {
            self.entries
                .as_mut()
                .expect("occupied table has entries")
                .as_mut_slice()[index]
                .assume_init_read()
        }
    }

    fn rehash(&mut self, new_slots: usize, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        K: Hash,
    {
        if new_slots == 0 {
            if self.len != 0 {
                return Err(CollectionError::CapacityOverflow);
            }
            self.control = None;
            self.entries = None;
            self.deleted = 0;
            return Ok(());
        }
        if self.len == 0 && new_slots == self.slot_count() {
            if let Some(control) = &mut self.control {
                control.as_mut_slice().fill(EMPTY);
            }
            self.deleted = 0;
            return Ok(());
        }

        let TableAllocations {
            control: mut new_control,
            entries: mut new_entries,
        } = allocate_table(new_slots, arena)?;
        if self.len == 0 {
            self.control = Some(new_control);
            self.entries = Some(new_entries);
            self.deleted = 0;
            return Ok(());
        }

        // Compute every destination before moving a key or value. Hash may
        // panic here; the old table remains untouched until planning succeeds.
        let mut destinations = arena.alloc_owned_slice::<usize>(self.len)?;
        let old_control = self
            .control
            .as_ref()
            .expect("nonempty map has control")
            .as_slice();
        let old_entries = self
            .entries
            .as_ref()
            .expect("nonempty map has entries")
            .as_slice();
        let mask = new_slots - 1;
        for old_index in 0..old_control.len() {
            if old_control[old_index] != OCCUPIED {
                continue;
            }
            // SAFETY: occupied source slots always contain an initialized pair.
            let key = unsafe { &old_entries[old_index].assume_init_ref().0 };
            let start = self.hash(key) as usize & mask;
            let mut destination = None;
            for offset in 0..new_slots {
                let candidate = (start + offset) & mask;
                if new_control.as_slice()[candidate] == EMPTY {
                    destination = Some(candidate);
                    break;
                }
            }
            let destination = destination.ok_or(CollectionError::CapacityOverflow)?;
            new_control.as_mut_slice()[destination] = OCCUPIED;
            destinations.push(destination)?;
        }

        // No user code runs after the plan is complete; all ownership moves
        // finish before the map publishes the replacement allocations.
        let destinations = destinations.as_slice();
        let mut destination_index = 0;
        let old_control = self.control.as_mut().expect("nonempty map has control");
        let old_entries = self.entries.as_mut().expect("nonempty map has entries");
        for old_index in 0..old_control.capacity() {
            if old_control.as_slice()[old_index] != OCCUPIED {
                continue;
            }
            let new_index = destinations[destination_index];
            destination_index += 1;
            old_control.as_mut_slice()[old_index] = EMPTY;
            // SAFETY: the source pair is initialized and the destination slot
            // is an initialized MaybeUninit wrapper reserved above.
            let pair = unsafe { old_entries.as_mut_slice()[old_index].assume_init_read() };
            new_entries.as_mut_slice()[new_index].write(pair);
        }
        self.control = Some(new_control);
        self.entries = Some(new_entries);
        self.deleted = 0;
        Ok(())
    }

    fn slot_count(&self) -> usize {
        self.control.as_ref().map_or(0, ArenaAllocation::capacity)
    }

    fn value_mut_at(&mut self, index: usize) -> &mut V {
        // SAFETY: the index is either an occupied slot from entry lookup or a
        // slot just committed by insert_with_index.
        let pair = unsafe {
            self.entries
                .as_mut()
                .expect("occupied map has entry storage")
                .as_mut_slice()[index]
                .assume_init_mut()
        };
        &mut pair.1
    }

    fn key_at(&self, index: usize) -> &K {
        // SAFETY: the index was captured from a completed occupied-slot probe.
        unsafe {
            &self
                .entries
                .as_ref()
                .expect("occupied map has entry storage")
                .as_slice()[index]
                .assume_init_ref()
                .0
        }
    }

    fn drop_entries(&mut self) {
        let mut guard = HashMapDropGuard {
            map: self,
            armed: true,
        };
        let slots = self.slot_count();
        for index in 0..slots {
            if self
                .control
                .as_ref()
                .expect("allocated table has control")
                .as_slice()[index]
                != OCCUPIED
            {
                continue;
            }
            let pair = self.remove_index(index);
            drop(pair);
        }
        if let Some(control) = &mut self.control {
            control.as_mut_slice().fill(EMPTY);
        }
        self.len = 0;
        self.deleted = 0;
        guard.armed = false;
    }
}

impl<'arena, K: CompactValue, V: CompactValue, S: BuildHasher> Drop
    for CompactHashMap<'arena, K, V, S>
{
    fn drop(&mut self) {
        self.drop_entries();
    }
}

impl<'arena, K: CompactValue, V: CompactValue> CompactHashMap<'arena, K, V, RandomState> {
    /// Construct an empty map with a fresh randomized hash builder.
    pub fn new() -> Self {
        Self::with_hasher(RandomState::new())
    }

    /// Construct a map with room for at least `capacity` entries and a fresh
    /// randomized hash builder.
    pub fn with_capacity(capacity: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        Self::with_capacity_and_hasher(capacity, RandomState::new(), arena)
    }
}

impl<K: CompactValue, V: CompactValue> Default for CompactHashMap<'_, K, V, RandomState> {
    fn default() -> Self {
        Self::new()
    }
}

/// An entry handle for conditional insertion into a compact hash map.
pub struct CompactHashMapEntry<
    'view,
    'arena,
    'memory,
    K: CompactValue,
    V: CompactValue,
    S: BuildHasher,
> {
    map: &'view mut CompactHashMap<'arena, K, V, S>,
    arena: &'view mut Arena<'arena, 'memory>,
    key: Option<K>,
    index: Option<usize>,
}

impl<'view, 'arena, 'memory, K: CompactValue, V: CompactValue, S: BuildHasher>
    CompactHashMapEntry<'view, 'arena, 'memory, K, V, S>
{
    /// Return whether the map already contained this key.
    pub const fn is_occupied(&self) -> bool {
        self.index.is_some()
    }

    /// Borrow the occupied or vacant key.
    pub fn key(&self) -> &K {
        match self.index {
            Some(index) => self.map.key_at(index),
            None => self.key.as_ref().expect("vacant entry retains its key"),
        }
    }

    /// Apply `action` to an existing value, then retain this entry handle.
    pub fn and_modify<F>(self, action: F) -> Self
    where
        F: FnOnce(&mut V),
    {
        if let Some(index) = self.index {
            action(self.map.value_mut_at(index));
        }
        self
    }

    /// Insert `default` if vacant, returning a mutable reference to the value.
    pub fn or_insert(self, default: V) -> Result<&'view mut V>
    where
        K: Hash + Eq,
    {
        self.or_insert_with(|| default)
    }

    /// Call `default` and insert its value only when the key is vacant.
    pub fn or_insert_with<F>(self, default: F) -> Result<&'view mut V>
    where
        K: Hash + Eq,
        F: FnOnce() -> V,
    {
        let Self {
            map,
            arena,
            key,
            index,
        } = self;
        if let Some(index) = index {
            drop(key);
            return Ok(map.value_mut_at(index));
        }
        let key = key.expect("vacant entry retains its key");
        let value = default();
        let (_old, index) = map.insert_with_index(key, value, arena)?;
        Ok(map.value_mut_at(index))
    }
}

/// An iterator over compact hash-map key-value pairs.
pub struct CompactHashMapIter<'view, K, V> {
    control: &'view [u8],
    entries: &'view [MaybeUninit<(K, V)>],
    next: usize,
    remaining: usize,
}

impl<'view, K, V> Iterator for CompactHashMapIter<'view, K, V> {
    type Item = (&'view K, &'view V);

    fn next(&mut self) -> Option<Self::Item> {
        while self.next < self.control.len() {
            let index = self.next;
            self.next += 1;
            if self.control[index] == OCCUPIED {
                self.remaining -= 1;
                // SAFETY: the occupied control byte identifies an initialized
                // entry, and the iterator is tied to an immutable map borrow.
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

/// A mutable iterator over compact hash-map key-value pairs.
pub struct CompactHashMapIterMut<'view, K, V> {
    control: &'view [u8],
    entries: *mut MaybeUninit<(K, V)>,
    next: usize,
    remaining: usize,
    marker: core::marker::PhantomData<&'view mut (K, V)>,
}

impl<'view, K, V> Iterator for CompactHashMapIterMut<'view, K, V> {
    type Item = (&'view K, &'view mut V);

    fn next(&mut self) -> Option<Self::Item> {
        while self.next < self.control.len() {
            let index = self.next;
            self.next += 1;
            if self.control[index] == OCCUPIED {
                self.remaining -= 1;
                // SAFETY: each slot is yielded at most once; the table is
                // exclusively borrowed and the control byte marks a live pair.
                let pair = unsafe { (&mut *self.entries.add(index)).assume_init_mut() };
                let (key, value) = pair;
                return Some((&*key, value));
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

struct HashMapDropGuard<'arena, K: CompactValue, V: CompactValue, S: BuildHasher> {
    map: *mut CompactHashMap<'arena, K, V, S>,
    armed: bool,
}

impl<K: CompactValue, V: CompactValue, S: BuildHasher> Drop for HashMapDropGuard<'_, K, V, S> {
    fn drop(&mut self) {
        if self.armed {
            // SAFETY: this pointer comes from the exclusive map borrow in
            // drop_entries and is used only to finish cleanup during unwind.
            unsafe { (*self.map).drop_entries() };
        }
    }
}

#[derive(Clone, Copy)]
struct Probe {
    found: Option<usize>,
    vacant: Option<usize>,
    vacant_is_deleted: bool,
}

impl Probe {
    const fn empty() -> Self {
        Self {
            found: None,
            vacant: None,
            vacant_is_deleted: false,
        }
    }

    const fn found(index: usize) -> Self {
        Self {
            found: Some(index),
            vacant: None,
            vacant_is_deleted: false,
        }
    }

    const fn vacant(index: usize, is_deleted: bool) -> Self {
        Self {
            found: None,
            vacant: Some(index),
            vacant_is_deleted: is_deleted,
        }
    }
}

struct TableAllocations<'arena, K: CompactValue, V: CompactValue> {
    control: ArenaAllocation<'arena, u8>,
    entries: ArenaAllocation<'arena, MaybeUninit<(K, V)>>,
}

fn allocate_table<'arena, K: CompactValue, V: CompactValue>(
    slots: usize,
    arena: &mut Arena<'arena, '_>,
) -> Result<TableAllocations<'arena, K, V>> {
    let mut control = arena.alloc_owned_slice::<u8>(slots)?;
    let mut entries = arena.alloc_owned_slice::<MaybeUninit<(K, V)>>(slots)?;
    for _ in 0..slots {
        control.push(EMPTY)?;
        entries.push(MaybeUninit::uninit())?;
    }
    Ok(TableAllocations { control, entries })
}

fn slots_for_entries(entries: usize) -> Result<usize> {
    if entries == 0 {
        return Ok(0);
    }
    u32::try_from(entries).map_err(|_| CollectionError::CapacityOverflow)?;
    let mut slots = MIN_SLOTS;
    while max_entries(slots) < entries {
        slots = slots
            .checked_mul(2)
            .filter(|slots| *slots <= u32::MAX as usize)
            .ok_or(CollectionError::CapacityOverflow)?;
    }
    Ok(slots)
}

fn max_entries(slots: usize) -> usize {
    slots.saturating_sub(slots / 8)
}

/// An arena-backed hash set implemented as a thin map-backed abstraction.
pub struct CompactHashSet<'arena, T: CompactValue, S: BuildHasher = RandomState> {
    map: CompactHashMap<'arena, T, (), S>,
}

impl<'arena, T: CompactValue, S: BuildHasher> CompactHashSet<'arena, T, S> {
    /// Construct an empty set with the supplied hash builder.
    pub fn with_hasher(hash_builder: S) -> Self {
        Self {
            map: CompactHashMap::with_hasher(hash_builder),
        }
    }

    /// Construct a set with room for at least `capacity` values.
    pub fn with_capacity_and_hasher(
        capacity: usize,
        hash_builder: S,
        arena: &mut Arena<'arena, '_>,
    ) -> Result<Self> {
        Ok(Self {
            map: CompactHashMap::with_capacity_and_hasher(capacity, hash_builder, arena)?,
        })
    }

    /// Return the number of stored values.
    pub const fn len(&self) -> usize {
        self.map.len()
    }

    /// Return the number of values the current table can hold before growth.
    pub fn capacity(&self) -> usize {
        self.map.capacity()
    }

    /// Return whether the set contains no values.
    pub const fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Borrow the active hash builder.
    pub const fn hasher(&self) -> &S {
        self.map.hasher()
    }

    /// Insert `value`, returning whether it was newly added.
    pub fn insert(&mut self, value: T, arena: &mut Arena<'arena, '_>) -> Result<bool>
    where
        T: Hash + Eq,
    {
        Ok(self.map.insert(value, (), arena)?.is_none())
    }

    /// Borrow the stored value equivalent to `value`.
    pub fn get<Q>(&self, value: &Q, arena: &Arena<'arena, '_>) -> Result<Option<&T>>
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        Ok(self
            .map
            .get_key_value(value, arena)?
            .map(|(stored, _)| stored))
    }

    /// Return whether an equivalent value is present.
    pub fn contains<Q>(&self, value: &Q, arena: &Arena<'arena, '_>) -> Result<bool>
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.contains_key(value, arena)
    }

    /// Remove an equivalent value and report whether one was present.
    pub fn remove<Q>(&mut self, value: &Q, arena: &Arena<'arena, '_>) -> Result<bool>
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        Ok(self.map.remove(value, arena)?.is_some())
    }

    /// Drop all stored values while retaining the table allocation.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Ensure room for at least `additional` more values.
    pub fn reserve(&mut self, additional: usize, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        T: Hash,
    {
        self.map.reserve(additional, arena)
    }

    /// Reduce storage to the smallest table that can hold the current values.
    pub fn shrink_to_fit(&mut self, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        T: Hash,
    {
        self.map.shrink_to_fit(arena)
    }

    /// Return an iterator over stored values.
    pub fn iter<'view>(
        &'view self,
        arena: &Arena<'arena, '_>,
    ) -> Result<impl ExactSizeIterator<Item = &'view T>> {
        self.map.keys(arena)
    }

    /// Keep only values for which `keep` returns true.
    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&T) -> bool,
    {
        self.map.retain(|value, _unit| keep(value));
    }
}

impl<'arena, T: CompactValue> CompactHashSet<'arena, T, RandomState> {
    /// Construct an empty set with a fresh randomized hash builder.
    pub fn new() -> Self {
        Self::with_hasher(RandomState::new())
    }

    /// Construct a set with room for at least `capacity` values and a fresh
    /// randomized hash builder.
    pub fn with_capacity(capacity: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        Self::with_capacity_and_hasher(capacity, RandomState::new(), arena)
    }
}

impl<T: CompactValue> Default for CompactHashSet<'_, T, RandomState> {
    fn default() -> Self {
        Self::new()
    }
}
