use compact_backend_std::StdArena;
use compact_collections::{CompactHashMap, CompactHashSet};
use compact_core::CompactValue;
use std::cell::Cell;
use std::collections::{HashMap as StdHashMap, HashSet as StdHashSet};
use std::hash::{BuildHasher, Hash, Hasher};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

#[derive(Clone, Copy, Default)]
struct CollisionBuildHasher;

struct CollisionHasher;

impl Hasher for CollisionHasher {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, _bytes: &[u8]) {}
}

impl BuildHasher for CollisionBuildHasher {
    type Hasher = CollisionHasher;

    fn build_hasher(&self) -> Self::Hasher {
        CollisionHasher
    }
}

#[test]
fn collision_heavy_operations_match_std_hash_map() {
    StdArena::with_capacity(256 * 1024, |arena| {
        let mut compact = CompactHashMap::with_hasher(CollisionBuildHasher);
        let mut standard = StdHashMap::new();
        let mut state = 0x5eed_u64;
        let steps = if cfg!(miri) { 256 } else { 2_000 };

        for step in 0..steps {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let key = ((state >> 32) % 113) as u32;
            match state as u8 % 7 {
                0 | 1 => {
                    let value = (state >> 17) as i64;
                    assert_eq!(
                        compact.insert(key, value, arena).unwrap(),
                        standard.insert(key, value)
                    );
                }
                2 => {
                    assert_eq!(
                        compact.remove_entry(&key, arena).unwrap(),
                        standard.remove_entry(&key)
                    );
                }
                3 => {
                    assert_eq!(compact.get(&key, arena).unwrap(), standard.get(&key));
                }
                4 => {
                    if let Some(value) = compact.get_mut(&key, arena).unwrap() {
                        *value = value.wrapping_add(9);
                    }
                    if let Some(value) = standard.get_mut(&key) {
                        *value = value.wrapping_add(9);
                    }
                }
                5 => compact.reserve((state as usize >> 8) % 5, arena).unwrap(),
                _ if step % 31 == 0 => {
                    compact.shrink_to_fit(arena).unwrap();
                    standard.shrink_to_fit();
                }
                _ => {}
            }

            let compact_snapshot: StdHashMap<_, _> = compact
                .iter(arena)
                .unwrap()
                .map(|(key, value)| (*key, *value))
                .collect();
            assert_eq!(compact_snapshot, standard);
            assert_eq!(compact.len(), standard.len());
            assert_eq!(compact.is_empty(), standard.is_empty());
        }
    })
    .unwrap();
}

#[test]
fn tombstones_rehash_entry_and_iterators_preserve_values() {
    StdArena::with_capacity(32 * 1024, |arena| {
        let mut map =
            CompactHashMap::with_capacity_and_hasher(12, CollisionBuildHasher, arena).unwrap();
        for key in 0..9_u32 {
            assert_eq!(map.insert(key, key * 10, arena).unwrap(), None);
        }
        assert_eq!(map.remove(&4, arena).unwrap(), Some(40));
        assert_eq!(map.get(&8, arena).unwrap(), Some(&80));
        assert_eq!(map.insert(14, 140, arena).unwrap(), None);

        *map.entry(8, arena)
            .unwrap()
            .and_modify(|value| *value += 1)
            .or_insert(0)
            .unwrap() += 1;
        assert_eq!(map.get(&8, arena).unwrap(), Some(&82));
        *map.entry(10, arena)
            .unwrap()
            .or_insert_with(|| 100)
            .unwrap() += 1;
        assert_eq!(map.get(&10, arena).unwrap(), Some(&101));

        for (key, value) in map.iter_mut(arena).unwrap() {
            if *key % 2 == 0 {
                *value += 5;
            }
        }
        let mut snapshot: Vec<_> = map
            .iter(arena)
            .unwrap()
            .map(|(key, value)| (*key, *value))
            .collect();
        snapshot.sort_unstable();
        assert_eq!(snapshot.len(), map.len());
        assert!(snapshot.contains(&(8, 87)));
        assert!(snapshot.contains(&(10, 106)));
        assert_eq!(map.keys(arena).unwrap().count(), map.len());
        assert_eq!(map.values(arena).unwrap().count(), map.len());
        assert_eq!(map.values_mut(arena).unwrap().count(), map.len());

        map.retain(|key, _| key % 2 == 0);
        assert!(map.iter(arena).unwrap().all(|(key, _)| *key % 2 == 0));
        map.shrink_to_fit(arena).unwrap();
        assert!(map.capacity() >= map.len());
        map.clear();
        assert!(map.is_empty());
        assert_eq!(map.iter(arena).unwrap().count(), 0);
    })
    .unwrap();
}

#[test]
fn collision_heavy_set_matches_std_hash_set() {
    StdArena::with_capacity(32 * 1024, |arena| {
        let mut compact = CompactHashSet::with_hasher(CollisionBuildHasher);
        let mut standard = StdHashSet::new();
        let value_count = if cfg!(miri) { 48_u32 } else { 200_u32 };
        for value in (0..value_count).chain((0..value_count / 2).rev()) {
            assert_eq!(
                compact.insert(value, arena).unwrap(),
                standard.insert(value)
            );
        }
        for value in (0..value_count).step_by(3) {
            assert_eq!(
                compact.remove(&value, arena).unwrap(),
                standard.remove(&value)
            );
        }
        compact.reserve(75, arena).unwrap();
        compact.shrink_to_fit(arena).unwrap();
        assert_eq!(compact.len(), standard.len());
        assert_eq!(
            compact
                .iter(arena)
                .unwrap()
                .copied()
                .collect::<StdHashSet<_>>(),
            standard
        );
        compact.retain(|value| value % 2 == 0);
        standard.retain(|value| value % 2 == 0);
        assert_eq!(
            compact
                .iter(arena)
                .unwrap()
                .copied()
                .collect::<StdHashSet<_>>(),
            standard
        );
        compact.clear();
        assert!(compact.is_empty());
    })
    .unwrap();
}

struct PanicKey {
    id: u32,
    panic_hash: Rc<Cell<bool>>,
    panic_eq: Rc<Cell<bool>>,
}

impl PanicKey {
    fn new(id: u32) -> Self {
        Self {
            id,
            panic_hash: Rc::new(Cell::new(false)),
            panic_eq: Rc::new(Cell::new(false)),
        }
    }
}

impl Hash for PanicKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        assert!(!self.panic_hash.get(), "intentional Hash panic");
        self.id.hash(state);
    }
}

impl PartialEq for PanicKey {
    fn eq(&self, other: &Self) -> bool {
        assert!(!self.panic_eq.get(), "intentional Eq panic");
        self.id == other.id
    }
}

impl Eq for PanicKey {}

// SAFETY: both Rc handles and all scalar fields can be moved and dropped
// independently of their arena storage.
unsafe impl CompactValue for PanicKey {}

#[test]
fn panicking_hash_and_equality_leave_the_table_usable() {
    StdArena::with_capacity(64 * 1024, |arena| {
        let mut map = CompactHashMap::with_hasher(CollisionBuildHasher);
        let stored = PanicKey::new(1);
        let stored_eq_panic = Rc::clone(&stored.panic_eq);
        map.insert(stored, 10_u32, arena).unwrap();
        map.insert(PanicKey::new(2), 20, arena).unwrap();

        let query = PanicKey::new(1);
        query.panic_hash.set(true);
        assert!(catch_unwind(AssertUnwindSafe(|| map.get(&query, arena))).is_err());
        query.panic_hash.set(false);

        stored_eq_panic.set(true);
        assert!(catch_unwind(AssertUnwindSafe(|| {
            map.insert(PanicKey::new(1), 99, arena)
        }))
        .is_err());
        stored_eq_panic.set(false);

        assert_eq!(map.get(&PanicKey::new(1), arena).unwrap(), Some(&10));
        assert_eq!(map.get(&PanicKey::new(2), arena).unwrap(), Some(&20));
    })
    .unwrap();
}

#[test]
fn panic_while_hashing_for_growth_keeps_old_entries() {
    StdArena::with_capacity(64 * 1024, |arena| {
        let mut map = CompactHashMap::with_hasher(CollisionBuildHasher);
        let first = PanicKey::new(0);
        let panic_on_rehash = Rc::clone(&first.panic_hash);
        map.insert(first, 0_u32, arena).unwrap();
        for id in 1..7 {
            map.insert(PanicKey::new(id), id, arena).unwrap();
        }

        panic_on_rehash.set(true);
        assert!(catch_unwind(AssertUnwindSafe(|| {
            map.insert(PanicKey::new(7), 7, arena)
        }))
        .is_err());
        panic_on_rehash.set(false);

        assert_eq!(map.len(), 7);
        for id in 0..7 {
            assert_eq!(map.get(&PanicKey::new(id), arena).unwrap(), Some(&id));
        }
    })
    .unwrap();
}

struct DropKey {
    id: u32,
    drops: Rc<Cell<usize>>,
}

impl Hash for DropKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl PartialEq for DropKey {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for DropKey {}

impl Drop for DropKey {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

// SAFETY: the Rc handle is movable, and Drop does not depend on the key's
// address or on arena storage.
unsafe impl CompactValue for DropKey {}

struct DropValue(Rc<Cell<usize>>);

impl Drop for DropValue {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

// SAFETY: the Rc handle is movable, and Drop does not depend on arena storage.
unsafe impl CompactValue for DropValue {}

#[test]
fn removals_replacements_clear_and_drop_destroy_owned_values_once() {
    let key_drops = Rc::new(Cell::new(0));
    let value_drops = Rc::new(Cell::new(0));
    StdArena::with_capacity(32 * 1024, |arena| {
        let mut map = CompactHashMap::with_hasher(CollisionBuildHasher);
        for id in 0..5 {
            map.insert(
                DropKey {
                    id,
                    drops: Rc::clone(&key_drops),
                },
                DropValue(Rc::clone(&value_drops)),
                arena,
            )
            .unwrap();
        }
        let old = map
            .insert(
                DropKey {
                    id: 2,
                    drops: Rc::clone(&key_drops),
                },
                DropValue(Rc::clone(&value_drops)),
                arena,
            )
            .unwrap()
            .unwrap();
        drop(old);
        assert_eq!(key_drops.get(), 1); // duplicate incoming key
        assert_eq!(value_drops.get(), 1); // replaced value

        let query = DropKey {
            id: 0,
            drops: Rc::new(Cell::new(0)),
        };
        let removed = map.remove_entry(&query, arena).unwrap();
        drop(removed);
        assert_eq!(key_drops.get(), 2);
        assert_eq!(value_drops.get(), 2);

        map.clear();
        assert_eq!(map.len(), 0);
        assert_eq!(key_drops.get(), 6);
        assert_eq!(value_drops.get(), 6);
    })
    .unwrap();
}

#[test]
fn allocation_exhaustion_during_growth_preserves_existing_entries() {
    StdArena::with_capacity(512, |arena| {
        let mut map = CompactHashMap::with_hasher(CollisionBuildHasher);
        let mut standard = StdHashMap::new();
        let mut saw_exhaustion = false;
        for key in 0..100_u32 {
            match map.insert(key, key * 2, arena) {
                Ok(old) => {
                    assert_eq!(old, standard.insert(key, key * 2));
                }
                Err(_) => {
                    saw_exhaustion = true;
                    break;
                }
            }
        }
        assert!(saw_exhaustion);
        let snapshot: StdHashMap<_, _> = map
            .iter(arena)
            .unwrap()
            .map(|(key, value)| (*key, *value))
            .collect();
        assert_eq!(snapshot, standard);
    })
    .unwrap();
}
