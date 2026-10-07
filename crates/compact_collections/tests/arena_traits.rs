use compact_backend_std::StdArena;
use compact_collections::{
    CloneIn, CompactBox, CompactBytes, CompactHashMap, CompactHashSet, CompactRing, CompactSlab,
    CompactSmallVec, CompactString, CompactVec, CompactVecDeque, ExtendIn, FromIteratorIn,
    ToCompactStringIn,
};
use compact_core::{Arena, CompactValue};
use std::cell::Cell;
use std::fmt;
use std::hash::Hash;
use std::panic::{catch_unwind, AssertUnwindSafe};

#[test]
fn explicit_collection_construction_extension_and_clone_work() {
    StdArena::with_capacity(64 * 1024, |arena| {
        let mut values = CompactVec::from_iter_in(0_u32..5, arena).unwrap();
        values.extend_in([5, 6, 7], arena).unwrap();
        assert_eq!(values.as_slice(arena).unwrap(), &[0, 1, 2, 3, 4, 5, 6, 7]);
        let copied = values.clone_in(arena).unwrap();
        assert_eq!(
            copied.as_slice(arena).unwrap(),
            values.as_slice(arena).unwrap()
        );

        let bytes = CompactBytes::from_iter_in(b"compact".iter().copied(), arena).unwrap();
        assert_eq!(bytes.as_slice(), b"compact");
        assert_eq!(bytes.clone_in(arena).unwrap().as_slice(), b"compact");

        let mut text = CompactString::from_iter_in("compact".chars(), arena).unwrap();
        text.extend_in(" std".chars(), arena).unwrap();
        assert_eq!(text.as_str(arena).unwrap(), "compact std");
        assert_eq!(
            text.clone_in(arena).unwrap().as_str(arena).unwrap(),
            "compact std"
        );
        assert_eq!(
            42_u32
                .to_compact_string_in(arena)
                .unwrap()
                .as_str(arena)
                .unwrap(),
            "42"
        );

        let small: CompactSmallVec<'_, u32, 2> =
            CompactSmallVec::from_iter_in([10_u32, 11, 12], arena).unwrap();
        assert_eq!(small.as_slice(arena).unwrap(), &[10, 11, 12]);
        assert_eq!(
            small.clone_in(arena).unwrap().as_slice(arena).unwrap(),
            &[10, 11, 12]
        );

        let deque = CompactVecDeque::from_iter_in([20_u32, 21, 22], arena).unwrap();
        assert_eq!(
            deque.iter(arena).unwrap().copied().collect::<Vec<_>>(),
            [20, 21, 22]
        );
        assert_eq!(
            deque
                .clone_in(arena)
                .unwrap()
                .iter(arena)
                .unwrap()
                .copied()
                .collect::<Vec<_>>(),
            [20, 21, 22]
        );

        let compact_key = CompactString::from_str_in("key", arena).unwrap();
        let compact_value = CompactString::from_str_in("value", arena).unwrap();
        let map: CompactHashMap<'_, _, _> =
            CompactHashMap::from_iter_in([(compact_key, compact_value)], arena).unwrap();
        let cloned_map = map.clone_in(arena).unwrap();
        assert_eq!(
            cloned_map
                .iter(arena)
                .unwrap()
                .map(|(key, value)| (key.as_ref(), value.as_ref()))
                .collect::<Vec<_>>(),
            [("key", "value")]
        );

        let set: CompactHashSet<'_, u32> =
            CompactHashSet::from_iter_in([1_u32, 2, 2], arena).unwrap();
        assert_eq!(set.len(), 2);
        assert_eq!(set.clone_in(arena).unwrap().len(), 2);
    })
    .unwrap();
}

#[test]
fn compact_string_writer_preserves_valid_prefix_on_arena_error() {
    StdArena::with_capacity(compact_core::MIN_ARENA_BYTES, |arena| {
        let mut text = CompactString::from_str_in("prefix", arena).unwrap();
        let result = text
            .writer(arena)
            .write_fmt_in(format_args!("{}", "x".repeat(128)));
        assert!(result.is_err());
        assert_eq!(text.as_str(arena).unwrap(), "prefix");
    })
    .unwrap();
}

struct PanickingDisplay;

impl fmt::Display for PanickingDisplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("partial")?;
        panic!("intentional Display panic");
    }
}

#[test]
fn panicking_display_leaves_a_valid_compact_string_and_reusable_arena() {
    StdArena::with_capacity(4096, |arena| {
        let mut text = CompactString::from_str_in("prefix:", arena).unwrap();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            text.writer(arena)
                .write_fmt_in(format_args!("{}!", PanickingDisplay))
        }));
        assert!(outcome.is_err());
        assert_eq!(text.as_str(arena).unwrap(), "prefix:partial");

        text.push_str_in(" tail", arena).unwrap();
        assert_eq!(text.as_str(arena).unwrap(), "prefix:partial tail");

        let another = CompactString::from_str_in("still usable", arena).unwrap();
        assert_eq!(another.as_str(arena).unwrap(), "still usable");
    })
    .unwrap();
}

fn assert_aligned<T>(value: &T) {
    assert_eq!(value as *const T as usize % core::mem::align_of::<T>(), 0);
}

fn exercise_generic_owners<T>(
    arena: &mut Arena<'_, '_>,
    make_value: impl FnMut() -> T,
) -> compact_collections::Result<usize>
where
    T: CompactValue + Eq + Hash,
{
    let constructed = Cell::new(0_usize);
    let mut make_value = make_value;
    let mut make = || {
        constructed.set(constructed.get() + 1);
        make_value()
    };

    let mut values = CompactVec::new_in(arena);
    values.push_in(make(), arena)?;
    values.push_in(make(), arena)?;
    assert_aligned(&values.as_slice(arena)?[0]);

    let mut deque = CompactVecDeque::new_in(arena);
    deque.push_back(make(), arena)?;
    deque.push_back(make(), arena)?;
    assert_aligned(deque.get(0, arena)?.unwrap());

    let mut ring = CompactRing::with_capacity(1, arena)?;
    ring.push_back(make(), arena)?;
    ring.push_back(make(), arena)?;
    assert_aligned(ring.front(arena)?.unwrap());

    let mut small = CompactSmallVec::<T, 1>::new_in(arena);
    small.push_in(make(), arena)?;
    small.push_in(make(), arena)?;
    assert_aligned(&small.as_slice(arena)?[0]);

    let boxed = CompactBox::new_in(make(), arena)?;
    assert_aligned(boxed.get(arena)?);

    let mut slab = CompactSlab::with_capacity_in(1, arena)?;
    let handle = slab.insert(make(), arena)?.unwrap();
    assert_aligned(slab.get(handle, arena)?);

    let mut map = CompactHashMap::new();
    map.insert(make(), make(), arena)?;
    let (key, value) = map.iter(arena)?.next().unwrap();
    assert_aligned(key);
    assert_aligned(value);

    let mut set = CompactHashSet::new();
    set.insert(make(), arena)?;
    assert_aligned(set.iter(arena)?.next().unwrap());

    Ok(constructed.get())
}

struct ZeroSizedDrop;

static ZERO_SIZED_OWNER_DROPS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

impl Drop for ZeroSizedDrop {
    fn drop(&mut self) {
        ZERO_SIZED_OWNER_DROPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl PartialEq for ZeroSizedDrop {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for ZeroSizedDrop {}

impl Hash for ZeroSizedDrop {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        0_u8.hash(state);
    }
}

// SAFETY: this zero-sized value has no address-dependent state and can be
// moved between collection storage locations before its destructor runs.
unsafe impl CompactValue for ZeroSizedDrop {}

#[repr(align(16))]
#[derive(Eq, Hash, PartialEq)]
struct Aligned16(u8);

#[repr(align(32))]
#[derive(Eq, Hash, PartialEq)]
struct Aligned32(u8);

#[repr(align(64))]
#[derive(Eq, Hash, PartialEq)]
struct Aligned64(u8);

// SAFETY: each wrapper contains one copyable scalar and has no address-bound
// state; its explicit alignment remains valid when moved by compact owners.
unsafe impl CompactValue for Aligned16 {}
unsafe impl CompactValue for Aligned32 {}
unsafe impl CompactValue for Aligned64 {}

#[test]
fn generic_owning_collections_cover_zst_and_overaligned_values() {
    ZERO_SIZED_OWNER_DROPS.store(0, std::sync::atomic::Ordering::SeqCst);
    StdArena::with_capacity(256 * 1024, |arena| -> compact_collections::Result<()> {
        let unit_count = exercise_generic_owners(arena, || ())?;
        assert_eq!(unit_count, 13);

        let zst_count = exercise_generic_owners(arena, || ZeroSizedDrop)?;
        assert_eq!(zst_count, 13);
        assert_eq!(
            ZERO_SIZED_OWNER_DROPS.load(std::sync::atomic::Ordering::SeqCst),
            zst_count
        );

        exercise_generic_owners(arena, || Aligned16(1))?;
        exercise_generic_owners(arena, || Aligned32(2))?;
        exercise_generic_owners(arena, || Aligned64(3))?;
        Ok(())
    })
    .unwrap()
    .unwrap();
}
