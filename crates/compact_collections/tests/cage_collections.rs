use compact_backend_std::{CageAllocation, CageConfig, CompactRuntime, ScratchRegion};
use compact_collections::{
    CompactBitVec, CompactBox, CompactBytes, CompactHashMap, CompactHashSet, CompactInterner,
    CompactPathBuf, CompactRing, CompactSlab, CompactSmallVec, CompactString, CompactVec,
    CompactVecDeque,
};
use compact_core::{CompactValue, Offset32};
use core::mem::size_of;
use proptest::prelude::*;
use std::ffi::OsStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

static INIT: OnceLock<()> = OnceLock::new();
fn init() {
    INIT.get_or_init(|| CompactRuntime::init(CageConfig::new(64 * 1024 * 1024)).unwrap());
}

#[test]
fn compact_owners_and_offsets_have_their_v23_sizes() {
    assert_eq!(size_of::<Offset32<u64>>(), 4);
    assert_eq!(size_of::<CageAllocation<u64>>(), 4);
    assert_eq!(size_of::<Option<CageAllocation<u64>>>(), 4);
    assert_eq!(size_of::<CompactBox<u64>>(), 4);
    assert_eq!(size_of::<CompactVec<u64>>(), 4);
    assert_eq!(size_of::<CompactVecDeque<u64>>(), 12);
    assert!(size_of::<CompactString>() <= 16);
}

#[test]
fn vec_string_box_and_bytes_grow_and_release() {
    init();
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

    let mut interner = CompactInterner::new();
    let first = interner.intern_str("worker").unwrap();
    assert_eq!(interner.intern_str("worker").unwrap(), first);
    assert_eq!(interner.resolve_str(first).unwrap(), Some("worker"));
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
    fn compact_vec_matches_a_native_vector(operations in proptest::collection::vec((0_u8..4, any::<u32>()), 0..256)) {
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
                _ => if !native.is_empty() {
                    let index = value as usize % native.len();
                    compact[index] = compact[index].wrapping_add(1);
                    native[index] = native[index].wrapping_add(1);
                },
            }
            prop_assert_eq!(compact.as_slice(), native.as_slice());
        }
    }
}
