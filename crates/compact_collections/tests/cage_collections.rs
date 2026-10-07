use compact_backend_std::{CageAllocation, CageConfig, CompactRuntime, ScratchRegion};
use compact_collections::{
    CompactBitVec, CompactBox, CompactBytes, CompactHashMap, CompactHashSet, CompactInterner,
    CompactPathBuf, CompactRing, CompactSlab, CompactSmallVec, CompactString, CompactVec,
    CompactVecDeque,
};
use compact_core::{ByteRange32, CompactValue, Offset32, OffsetSlice32, ABI_VERSION};
use core::mem::size_of;
use proptest::prelude::*;
use std::ffi::OsStr;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

static INIT: OnceLock<()> = OnceLock::new();
fn init() {
    INIT.get_or_init(|| CompactRuntime::init(CageConfig::new(64 * 1024 * 1024)).unwrap());
}

#[test]
fn compact_owners_and_offsets_have_their_v24_sizes() {
    assert_eq!(size_of::<Offset32<u64>>(), 4);
    assert_eq!(size_of::<OffsetSlice32<u64>>(), 8);
    assert_eq!(size_of::<ByteRange32>(), 8);
    assert_eq!(size_of::<CageAllocation<u64>>(), 4);
    assert_eq!(size_of::<Option<CageAllocation<u64>>>(), 4);
    assert_eq!(size_of::<CompactBox<u64>>(), 4);
    assert_eq!(size_of::<CompactVec<u64>>(), 4);
    assert_eq!(size_of::<CompactVecDeque<u64>>(), 12);
    assert!(size_of::<CompactString>() <= 16);
    assert_eq!(ABI_VERSION.major, 2);
    assert_eq!(ABI_VERSION.minor, 4);
}

#[test]
fn vec_string_box_and_bytes_grow_and_release() {
    init();
    let mut moved = CompactVec::with_capacity(2).unwrap();
    moved.push(41_u32).unwrap();
    let blocker = CompactRuntime::alloc_owned_slice::<u8>(128).unwrap();
    moved.reserve(20).unwrap();
    assert!(moved.capacity() >= 21);
    assert_eq!(moved.as_slice(), &[41]);
    drop(blocker);
    {
        let mut nested = CompactVec::new();
        nested
            .push(CompactString::from_str("a heap-backed nested compact string").unwrap())
            .unwrap();
        nested
            .push(CompactString::from_str("another independently owned string").unwrap())
            .unwrap();
        assert_eq!(nested[0].as_str(), "a heap-backed nested compact string");
        assert_eq!(nested[1].as_str(), "another independently owned string");
    }
    CompactRuntime::validate_allocator_state().unwrap();

    let mut values = CompactVec::new();
    for value in 0..2_000_u32 {
        values.push(value).unwrap();
    }
    assert_eq!(values.len(), 2_000);
    assert_eq!(values[1_337], 1_337);
    values.truncate(1_000);
    values.shrink_to_fit().unwrap();
    assert_eq!(values.capacity(), 1_000);

    let mut text = CompactString::from_str("cage").unwrap();
    text.push_str(" backed compact string").unwrap();
    assert_eq!(text.as_str(), "cage backed compact string");
    let boxed = CompactBox::new(42_u32).unwrap();
    assert_eq!(*boxed, 42);

    let mut bytes = CompactBytes::from_slice(b"first fragment").unwrap();
    bytes.extend_from_slice(b" + second fragment").unwrap();
    assert_eq!(bytes.as_slice(), b"first fragment + second fragment");
    bytes.truncate(5);
    assert_eq!(bytes.as_slice(), b"first");

    let mut fallible_bytes = CompactBytes::new();
    let mut emitted = 0;
    let mut source_error = None;
    fallible_bytes
        .try_extend_fallible(
            0,
            || {
                emitted += 1;
                if emitted <= 25 {
                    Ok(Some(emitted as u8))
                } else {
                    Err("source failed")
                }
            },
            &mut source_error,
        )
        .unwrap();
    assert_eq!(source_error, Some("source failed"));
    assert_eq!(fallible_bytes.len(), 25);
    assert_eq!(fallible_bytes.as_slice(), (1_u8..=25).collect::<Vec<_>>());
}

#[test]
fn drop_runs_exactly_once_when_a_vector_is_truncated_and_dropped() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct DropValue(u32);
    impl Drop for DropValue {
        fn drop(&mut self) {
            let _ = self.0;
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    unsafe impl CompactValue for DropValue {}

    init();
    DROPS.store(0, Ordering::SeqCst);
    let mut values = CompactVec::new();
    for value in 0..12 {
        values.push(DropValue(value)).unwrap();
    }
    values.truncate(5);
    assert_eq!(DROPS.load(Ordering::SeqCst), 7);
    drop(values);
    assert_eq!(DROPS.load(Ordering::SeqCst), 12);
}

#[test]
fn bulk_vector_extend_preserves_prefixes_on_iterator_and_clone_panics() {
    static CLONES: AtomicUsize = AtomicUsize::new(0);
    static DROPS: AtomicUsize = AtomicUsize::new(0);

    struct CloneProbe;
    impl Clone for CloneProbe {
        fn clone(&self) -> Self {
            let clone = CLONES.fetch_add(1, Ordering::SeqCst);
            assert_ne!(clone, 2, "requested clone panic");
            Self
        }
    }
    impl Drop for CloneProbe {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    unsafe impl CompactValue for CloneProbe {}

    init();
    let mut values = CompactVec::with_capacity(2).unwrap();
    values.try_extend([1_u32, 2, 3]).unwrap();

    struct LowHint {
        next: u32,
        end: u32,
    }
    impl Iterator for LowHint {
        type Item = u32;
        fn next(&mut self) -> Option<Self::Item> {
            if self.next == self.end {
                None
            } else {
                let value = self.next;
                self.next += 1;
                Some(value)
            }
        }
        fn size_hint(&self) -> (usize, Option<usize>) {
            (0, None)
        }
    }
    values.try_extend(LowHint { next: 4, end: 13 }).unwrap();
    assert_eq!(values.as_slice(), (1_u32..13).collect::<Vec<_>>());

    let mut yielded = 0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        values.try_extend(std::iter::from_fn(|| {
            yielded += 1;
            match yielded {
                1..=3 => Some(100 + yielded),
                _ => panic!("requested iterator panic"),
            }
        }))
    }));
    assert!(result.is_err());
    assert_eq!(&values.as_slice()[12..], &[101, 102, 103]);

    let mut probes = CompactVec::with_capacity(4).unwrap();
    probes.try_extend((0..4).map(|_| CloneProbe)).unwrap();
    CLONES.store(0, Ordering::SeqCst);
    DROPS.store(0, Ordering::SeqCst);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| probes.try_clone()));
    assert!(result.is_err());
    assert_eq!(DROPS.load(Ordering::SeqCst), 2);
    drop(probes);
    assert_eq!(DROPS.load(Ordering::SeqCst), 6);
}

#[test]
fn deque_ring_smallvec_and_bits_preserve_order_and_capacity_rules() {
    init();
    let mut deque = CompactVecDeque::with_capacity(4).unwrap();
    for value in 0..4 {
        deque.push_back(value).unwrap();
    }
    assert_eq!(deque.pop_front(), Some(0));
    deque.push_back(4).unwrap();
    assert_eq!(deque.iter().copied().collect::<Vec<_>>(), [1, 2, 3, 4]);
    deque.push_front(0).unwrap();
    assert_eq!(deque.pop_back(), Some(4));

    let mut ring = CompactRing::with_capacity(3).unwrap();
    for value in 0..5 {
        ring.push_back(value).unwrap();
    }
    assert_eq!(ring.iter().copied().collect::<Vec<_>>(), [2, 3, 4]);

    let mut small = CompactSmallVec::<u32, 2>::new();
    small.push(8).unwrap();
    small.push(9).unwrap();
    assert!(small.is_inline());
    small.push(10).unwrap();
    assert!(!small.is_inline());
    assert_eq!(small.as_slice(), [8, 9, 10]);

    let mut bits = CompactBitVec::new();
    for index in 0..33 {
        bits.push(index % 3 == 0).unwrap();
    }
    assert!(bits.get(30).unwrap());
    assert!(!bits.get(31).unwrap());
    bits.set(31, true).unwrap();
    bits.truncate(31);
    assert_eq!(bits.len(), 31);
}

#[test]
fn deque_iterator_handles_wrapping_and_alternating_ends() {
    init();
    let mut deque = CompactVecDeque::with_capacity(5).unwrap();
    for value in 0..5 {
        deque.push_back(value).unwrap();
    }
    assert_eq!(deque.pop_front(), Some(0));
    assert_eq!(deque.pop_front(), Some(1));
    deque.push_back(5).unwrap();
    deque.push_back(6).unwrap();
    assert_eq!(deque.iter().copied().collect::<Vec<_>>(), [2, 3, 4, 5, 6]);
    assert_eq!(
        deque.iter().rev().copied().collect::<Vec<_>>(),
        [6, 5, 4, 3, 2]
    );

    let mut iter = deque.iter();
    assert_eq!(iter.len(), 5);
    assert_eq!(iter.next(), Some(&2));
    assert_eq!(iter.next_back(), Some(&6));
    assert_eq!(iter.next_back(), Some(&5));
    assert_eq!(iter.next(), Some(&3));
    assert_eq!(iter.next(), Some(&4));
    assert_eq!(iter.next_back(), None);
    assert_eq!(iter.len(), 0);
}

#[test]
fn randomized_hash_collections_paths_slab_and_interner_work() {
    init();
    let mut map = CompactHashMap::new();
    let mut set = CompactHashSet::new();
    for value in 0..500_u32 {
        map.insert(value, value * 3).unwrap();
        assert!(set.insert(value).unwrap());
    }
    assert_eq!(map.get(&319), Some(&957));
    assert!(set.contains(&319));
    assert_eq!(map.remove(&319), Some(957));
    assert!(set.remove(&319));
    assert_eq!(map.len(), 499);

    let mut path = CompactPathBuf::from("var/log").unwrap();
    path.push("service.log").unwrap();
    assert_eq!(
        path.file_name().unwrap().to_os_string(),
        OsStr::new("service.log")
    );
    assert_eq!(path.to_path_buf().to_string_lossy(), "var/log/service.log");

    let mut slab = CompactSlab::with_capacity(1).unwrap();
    let old = slab.insert(17_u32).unwrap();
    assert_eq!(slab.remove(old), Some(17));
    let new = slab.insert(19).unwrap();
    assert_ne!(old.generation(), new.generation());
    assert_eq!(slab.get(old), None);
    assert_eq!(slab.get(new), Some(&19));
    let other_slab = CompactSlab::<u32>::with_capacity(2).unwrap();
    assert_eq!(other_slab.get(new), None);

    let mut interner = CompactInterner::new();
    let first = interner.intern_str("worker").unwrap();
    assert_eq!(interner.intern_str("worker").unwrap(), first);
    assert_ne!(interner.intern_str("scheduler").unwrap(), first);
    assert_eq!(interner.resolve_str(first).unwrap(), Some("worker"));
}

#[derive(Clone, Copy, Default)]
struct ConstantBuildHasher;
struct ConstantHasher;
impl Hasher for ConstantHasher {
    fn finish(&self) -> u64 {
        0
    }
    fn write(&mut self, _bytes: &[u8]) {}
}
impl BuildHasher for ConstantBuildHasher {
    type Hasher = ConstantHasher;
    fn build_hasher(&self) -> Self::Hasher {
        ConstantHasher
    }
}
unsafe impl CompactValue for ConstantBuildHasher {}

#[test]
fn hash_collisions_tombstones_and_panicking_hash_are_consistent() {
    init();
    let mut map = CompactHashMap::with_hasher(ConstantBuildHasher);
    for key in 0..96_u32 {
        assert_eq!(map.insert(key, key * 7).unwrap(), None);
    }
    for key in (0..96_u32).step_by(2) {
        assert_eq!(map.remove(&key), Some(key * 7));
    }
    for key in 96..144_u32 {
        assert_eq!(map.insert(key, key * 7).unwrap(), None);
    }
    assert_eq!(map.len(), 96);
    for key in 1..96_u32 {
        if key % 2 == 1 {
            assert_eq!(map.get(&key), Some(&(key * 7)));
        }
    }

    #[derive(Eq, PartialEq)]
    struct PanicKey(u8);
    impl Hash for PanicKey {
        fn hash<H: Hasher>(&self, state: &mut H) {
            assert_ne!(self.0, u8::MAX, "requested key hash panic");
            self.0.hash(state);
        }
    }
    unsafe impl CompactValue for PanicKey {}

    let mut panic_map = CompactHashMap::with_capacity(8).unwrap();
    panic_map.insert(PanicKey(3), 21_u32).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = panic_map.insert(PanicKey(u8::MAX), 77_u32);
    }));
    assert!(result.is_err());
    assert_eq!(panic_map.len(), 1);
    assert_eq!(panic_map.get(&PanicKey(3)), Some(&21));
}

#[test]
fn inline_transitions_utf8_boundaries_and_non_utf8_paths_round_trip() {
    init();
    let mut bytes =
        CompactBytes::from_slice(&[0xabu8; compact_collections::COMPACT_BYTES_INLINE_CAPACITY])
            .unwrap();
    assert_eq!(
        bytes.capacity(),
        compact_collections::COMPACT_BYTES_INLINE_CAPACITY
    );
    bytes.push(0xcd).unwrap();
    assert!(bytes.capacity() > compact_collections::COMPACT_BYTES_INLINE_CAPACITY);
    bytes.truncate(8);
    bytes.shrink_to_fit().unwrap();
    assert_eq!(
        bytes.capacity(),
        compact_collections::COMPACT_BYTES_INLINE_CAPACITY
    );
    assert_eq!(bytes.as_slice(), &[0xab; 8]);

    let mut text = CompactString::from_str("é🦀").unwrap();
    assert_eq!(text.as_str(), "é🦀");
    assert!(text.truncate(1).is_err());
    text.push_str("-worker-name-that-spills").unwrap();
    text.truncate("é".len()).unwrap();
    assert_eq!(text.as_str(), "é");

    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let raw = std::ffi::OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff, b'x']);
        let compact = CompactPathBuf::from_path(std::path::Path::new(&raw)).unwrap();
        assert_eq!(
            compact.as_os_str().as_encoded_bytes(),
            raw.as_os_str().as_bytes()
        );
        assert_eq!(
            compact.to_path_buf().as_os_str().as_bytes(),
            raw.as_os_str().as_bytes()
        );
    }
}

#[test]
fn deque_wrap_grow_contiguous_and_ring_eviction_drop_exactly_once() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct DropValue(u32);
    impl Drop for DropValue {
        fn drop(&mut self) {
            let _ = self.0;
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    unsafe impl CompactValue for DropValue {}

    init();
    let mut deque = CompactVecDeque::with_capacity(4).unwrap();
    for value in 0..4 {
        deque.push_back(value).unwrap();
    }
    assert_eq!(deque.pop_front(), Some(0));
    assert_eq!(deque.pop_front(), Some(1));
    deque.push_back(4).unwrap();
    deque.push_back(5).unwrap();
    deque.push_back(6).unwrap();
    assert_eq!(deque.iter().copied().collect::<Vec<_>>(), [2, 3, 4, 5, 6]);
    assert_eq!(deque.make_contiguous().unwrap(), &[2, 3, 4, 5, 6]);

    DROPS.store(0, Ordering::SeqCst);
    {
        let mut ring = CompactRing::with_capacity(2).unwrap();
        ring.push_back(DropValue(1)).unwrap();
        ring.push_back(DropValue(2)).unwrap();
        ring.push_back(DropValue(3)).unwrap();
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    }
    assert_eq!(DROPS.load(Ordering::SeqCst), 3);
}

#[test]
fn scratch_is_aligned_bounded_and_shared_cage_storage() {
    init();
    let mut scratch = ScratchRegion::new(128).unwrap();
    let bytes = scratch.alloc_bytes(9).unwrap();
    assert_eq!(bytes, &[0; 9]);
    let value = scratch.alloc_value(0x1234_u64).unwrap();
    assert_eq!(*value, 0x1234);
    assert_eq!(scratch.used(), 24);
    assert!(scratch.alloc_bytes(112).is_err());
}

#[test]
fn owners_can_be_released_on_other_threads_and_shared_for_reads() {
    init();
    let shared = std::sync::Arc::new(CompactVec::try_from_iter(0..100_u32).unwrap());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let shared = shared.clone();
            scope.spawn(move || {
                assert_eq!(shared[99], 99);
                for value in 0..100 {
                    let allocation = CompactRuntime::alloc_owned_value(value).unwrap();
                    drop(allocation);
                }
            });
        }
        scope.spawn(|| {
            let allocation = CompactRuntime::alloc_owned_value(77_u32).unwrap();
            drop(allocation);
        });
    });
    CompactRuntime::validate_allocator_state().unwrap();
}

#[test]
fn panic_during_a_value_destructor_still_releases_every_initialized_value() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct PanicDrop {
        panic: bool,
    }
    impl Drop for PanicDrop {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
            assert!(!self.panic, "requested destructor panic");
        }
    }
    unsafe impl CompactValue for PanicDrop {}

    init();
    DROPS.store(0, Ordering::SeqCst);
    let mut values = CompactVec::new();
    for index in 0..5 {
        values.push(PanicDrop { panic: index == 2 }).unwrap();
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(values)));
    assert!(result.is_err());
    assert_eq!(DROPS.load(Ordering::SeqCst), 5);
    CompactRuntime::validate_allocator_state().unwrap();
}

#[test]
fn map_drop_batches_nested_releases_and_finishes_after_one_value_panics() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct PanicDrop {
        bytes: CompactBytes,
        panic: bool,
    }
    unsafe impl CompactValue for PanicDrop {}
    impl Drop for PanicDrop {
        fn drop(&mut self) {
            let _ = self.bytes.len();
            DROPS.fetch_add(1, Ordering::SeqCst);
            assert!(!self.panic, "requested map value destructor panic");
        }
    }

    init();
    DROPS.store(0, Ordering::SeqCst);
    let mut map = CompactHashMap::with_capacity(8).unwrap();
    for index in 0..5 {
        map.insert(
            index,
            PanicDrop {
                bytes: CompactBytes::from_slice(&[index as u8; 64]).unwrap(),
                panic: index == 2,
            },
        )
        .unwrap();
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(map)));
    assert!(result.is_err());
    assert_eq!(DROPS.load(Ordering::SeqCst), 5);
    CompactRuntime::validate_allocator_state().unwrap();
}

#[test]
fn smallvec_drops_remaining_inline_values_after_one_destructor_panics() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct PanicDrop {
        panic: bool,
    }
    impl Drop for PanicDrop {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
            assert!(!self.panic, "requested destructor panic");
        }
    }
    unsafe impl CompactValue for PanicDrop {}

    init();
    DROPS.store(0, Ordering::SeqCst);
    let mut values = CompactSmallVec::<PanicDrop, 4>::new();
    for index in 0..4 {
        values.push(PanicDrop { panic: index == 1 }).unwrap();
    }
    assert!(values.is_inline());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(values)));
    assert!(result.is_err());
    assert_eq!(DROPS.load(Ordering::SeqCst), 4);
}

proptest::proptest! {
    #[test]
    fn compact_vec_matches_a_native_vector(operations in proptest::collection::vec((0_u8..6, any::<u32>()), 0..256)) {
        init();
        let mut compact = CompactVec::<u32>::new();
        let mut native = std::vec::Vec::<u32>::new();
        for (operation, value) in operations {
            match operation {
                0 => { compact.push(value).unwrap(); native.push(value); }
                1 => prop_assert_eq!(compact.pop(), native.pop()),
                2 => {
                    let new_len = value as usize % (native.len() + 1);
                    compact.truncate(new_len);
                    native.truncate(new_len);
                }
                3 => if !native.is_empty() {
                    let index = value as usize % native.len();
                    compact[index] = compact[index].wrapping_add(1);
                    native[index] = native[index].wrapping_add(1);
                },
                4 => compact.reserve(value as usize % 16).unwrap(),
                _ => compact.shrink_to_fit().unwrap(),
            }
            prop_assert_eq!(compact.as_slice(), native.as_slice());
            CompactRuntime::validate_allocator_state().unwrap();
        }
    }
}
