use compact_backend_std::StdArena;
use compact_collections::{
    CollectionError, CompactBitVec, CompactBox, CompactBytes, CompactInterner, CompactOption,
    CompactRing, CompactSlab, CompactSmallVec, CompactString, CompactVec, CompactVecDeque,
    InternId,
};
use compact_core::CompactValue;
use compact_core::Offset32;
use std::borrow::{Borrow, BorrowMut};
use std::cell::Cell;
use std::collections::VecDeque as StdVecDeque;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn hash_value(value: &impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

struct DropCounter(Rc<Cell<usize>>);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

// SAFETY: the Rc handle is movable and its destructor does not depend on the
// value's address or arena lifetime.
unsafe impl CompactValue for DropCounter {}

#[test]
fn compact_metadata_sizes_match_the_v2_targets() {
    assert_eq!(core::mem::size_of::<CompactBox<'static, u32>>(), 16);
    assert_eq!(core::mem::size_of::<CompactOption<'static, u32>>(), 4);
    assert_eq!(core::mem::size_of::<CompactVec<'static, u32>>(), 16);
    assert_eq!(core::mem::size_of::<CompactString<'static>>(), 24);
    assert_eq!(core::mem::size_of::<CompactSlab<'static, u32>>(), 24);
    assert_eq!(
        core::mem::size_of::<compact_core::ByteRange32<'static>>(),
        8
    );
    assert_eq!(core::mem::size_of::<CompactSmallVec<'static, u32, 2>>(), 24);
    assert_eq!(core::mem::size_of::<InternId<'static>>(), 8);
}

#[test]
fn compact_links_and_small_strings_have_deterministic_memory_costs() {
    #[repr(C)]
    #[allow(dead_code)]
    struct NativeLink {
        next: *const NativeLink,
        value: u32,
    }
    #[repr(C)]
    #[allow(dead_code)]
    struct CompactLink<'arena> {
        next: Offset32<'arena, ()>,
        value: u32,
    }
    assert!(core::mem::size_of::<NativeLink>() > core::mem::size_of::<CompactLink<'static>>());
    assert_eq!(core::mem::size_of::<CompactLink<'static>>(), 8);

    StdArena::with_capacity(256, |arena| {
        let before = arena.used_bytes();
        for _ in 0..64 {
            let text = CompactString::from_str_in("tiny", arena).unwrap();
            assert_eq!(text.as_str(arena).unwrap(), "tiny");
        }
        assert_eq!(arena.used_bytes(), before);
    })
    .unwrap();
}

#[test]
fn compact_box_and_nullable_offset_resolve_through_the_arena() {
    StdArena::with_capacity(128, |arena| {
        let mut boxed = CompactBox::new_in(41_u32, arena).unwrap();
        *boxed.get_mut(arena).unwrap() += 1;
        assert_eq!(*boxed.get(arena).unwrap(), 42);

        let offset = arena.alloc_value(42_u32).unwrap();
        let some = CompactOption::some(offset);
        let none = CompactOption::<u32>::none();
        assert_eq!(some.get(arena).unwrap(), Some(&42));
        assert_eq!(none.get(arena).unwrap(), None);
    })
    .unwrap();
}

#[test]
fn nested_scratch_scopes_host_temporary_compact_collections() {
    StdArena::with_capacity(2048, |arena| {
        let outer_used = arena.used_bytes();
        arena
            .scratch(1024, |scratch| {
                let mut values = CompactVec::new_in(scratch);
                values.push_in(3_u32, scratch).unwrap();
                values.push_in(5, scratch).unwrap();
                assert_eq!(values.as_slice(scratch).unwrap(), &[3, 5]);

                let mut text =
                    CompactString::from_str_in("temporary compact text", scratch).unwrap();
                text.push_str_in("!", scratch).unwrap();
                assert_eq!(text.as_str(scratch).unwrap(), "temporary compact text!");

                let bytes = scratch.alloc_bytes(b"payload").unwrap();
                assert_eq!(scratch.get_bytes(bytes).unwrap(), b"payload");
            })
            .unwrap();
        assert_eq!(arena.used_bytes(), outer_used);
    })
    .unwrap();
}

#[test]
fn compact_bytes_promotes_retains_capacity_and_returns_inline() {
    StdArena::with_capacity(512, |arena| {
        let baseline = arena.used_bytes();
        let mut bytes = CompactBytes::new_in(arena);
        assert_eq!(bytes.capacity(), 20);
        bytes
            .extend_from_slice_in(b"0123456789abcdefghij", arena)
            .unwrap();
        assert_eq!(arena.used_bytes(), baseline);
        bytes.push(b'!', arena).unwrap();
        assert!(bytes.capacity() >= 21);
        assert_eq!(bytes.as_slice(), b"0123456789abcdefghij!");

        let heap_capacity = bytes.capacity();
        bytes.truncate(7);
        assert_eq!(bytes.as_slice(), b"0123456");
        bytes.clear();
        assert_eq!(bytes.capacity(), heap_capacity);
        bytes.extend_from_slice_in(b"reuse", arena).unwrap();
        assert_eq!(bytes.as_slice(), b"reuse");

        bytes.reserve(25, arena).unwrap();
        bytes.extend_from_slice(&[9; 25], arena).unwrap();
        bytes.truncate(24);
        bytes.shrink_to_fit(arena).unwrap();
        assert_eq!(bytes.capacity(), 24);
        bytes.truncate(7);
        bytes.shrink_to_fit_in(arena).unwrap();
        assert_eq!(bytes.capacity(), 20);
        assert_eq!(arena.used_bytes(), baseline);
    })
    .unwrap();
}

#[test]
fn compact_bytes_split_and_traits_match_byte_slices() {
    StdArena::with_capacity(512, |arena| {
        let mut expected = (0_u8..40).collect::<std::vec::Vec<_>>();
        let mut bytes = CompactBytes::from_slice(&expected, arena).unwrap();
        assert_eq!(bytes.as_slice(), expected.as_slice());
        assert_eq!(bytes.as_ref(), expected.as_slice());
        assert_eq!(bytes[3], 3);
        bytes[3] = 99;
        expected[3] = 99;

        let right = bytes.split_off(10, arena).unwrap();
        assert_eq!(bytes.as_slice(), &expected[..10]);
        assert_eq!(right.as_slice(), &expected[10..]);
        assert_eq!(
            bytes.split_off_in(bytes.len() + 1, arena).unwrap_err(),
            CollectionError::Core(compact_core::Error::OutOfBounds)
        );
        let all = bytes.split_off_in(0, arena).unwrap();
        assert!(bytes.is_empty());
        assert_eq!(all.as_slice().len(), 10);
    })
    .unwrap();
}

#[test]
fn compact_bytes_growth_failure_preserves_contents() {
    StdArena::with_capacity(compact_core::MIN_ARENA_BYTES, |arena| {
        let mut bytes = CompactBytes::from_slice_in(b"kept inline", arena).unwrap();
        let old = bytes.as_slice().to_vec();
        let error = bytes.extend_from_slice_in(&[0xAA; 128], arena).unwrap_err();
        assert!(matches!(
            error,
            CollectionError::Core(compact_core::Error::AllocationExhausted)
        ));
        assert_eq!(bytes.as_slice(), old);
    })
    .unwrap();
}

#[test]
fn compact_vec_deque_matches_std_across_wrapping_and_reallocation() {
    StdArena::with_capacity(16 * 1024, |arena| {
        let mut compact = CompactVecDeque::with_capacity(3, arena).unwrap();
        let mut standard = StdVecDeque::with_capacity(3);
        for step in 0_i32..700 {
            match step % 7 {
                0 => {
                    compact.push_back(step, arena).unwrap();
                    standard.push_back(step);
                }
                1 => {
                    compact.push_front(-step, arena).unwrap();
                    standard.push_front(-step);
                }
                2 => assert_eq!(compact.pop_back(arena).unwrap(), standard.pop_back()),
                3 => {
                    compact.push_back(step * 3, arena).unwrap();
                    standard.push_back(step * 3);
                }
                4 => assert_eq!(compact.pop_front(arena).unwrap(), standard.pop_front()),
                5 => {
                    compact.push_front(step * 2, arena).unwrap();
                    standard.push_front(step * 2);
                }
                _ => {
                    if let Some(value) = compact.front_mut(arena).unwrap() {
                        *value += 1;
                    }
                    if let Some(value) = standard.front_mut() {
                        *value += 1;
                    }
                }
            }
            assert_eq!(compact.len(), standard.len());
            assert_eq!(
                compact.front(arena).unwrap().copied(),
                standard.front().copied()
            );
            assert_eq!(
                compact.back(arena).unwrap().copied(),
                standard.back().copied()
            );
            assert_eq!(
                compact
                    .iter(arena)
                    .unwrap()
                    .copied()
                    .collect::<std::vec::Vec<_>>(),
                standard.iter().copied().collect::<std::vec::Vec<_>>()
            );
            for (index, value) in standard.iter().enumerate() {
                assert_eq!(compact.get(index, arena).unwrap(), Some(value));
            }
            assert_eq!(
                compact
                    .iter(arena)
                    .unwrap()
                    .rev()
                    .copied()
                    .collect::<std::vec::Vec<_>>(),
                standard.iter().rev().copied().collect::<std::vec::Vec<_>>()
            );
        }

        compact.reserve(32, arena).unwrap();
        for value in compact.iter_mut(arena).unwrap() {
            *value += 5;
        }
        for value in &mut standard {
            *value += 5;
        }
        assert_eq!(
            compact.make_contiguous(arena).unwrap(),
            standard.make_contiguous()
        );

        compact
            .retain(arena, |value| value.rem_euclid(3) != 0)
            .unwrap();
        standard.retain(|value| value.rem_euclid(3) != 0);
        compact.truncate(4);
        standard.truncate(4);
        assert_eq!(
            compact
                .iter(arena)
                .unwrap()
                .copied()
                .collect::<std::vec::Vec<_>>(),
            standard.iter().copied().collect::<std::vec::Vec<_>>()
        );

        compact.shrink_to_fit(arena).unwrap();
        assert_eq!(compact.capacity(), compact.len());
        compact.clear();
        standard.clear();
        compact.push_front(71, arena).unwrap();
        standard.push_front(71);
        assert_eq!(
            compact
                .iter(arena)
                .unwrap()
                .copied()
                .collect::<std::vec::Vec<_>>(),
            standard.iter().copied().collect::<std::vec::Vec<_>>()
        );
    })
    .unwrap();
}

static ZERO_SIZED_DROPS: AtomicUsize = AtomicUsize::new(0);

struct ZeroSizedDrop;

impl Drop for ZeroSizedDrop {
    fn drop(&mut self) {
        ZERO_SIZED_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

// SAFETY: the zero-sized value has no address-dependent state, and its
// destructor is safe while the arena remains alive.
unsafe impl CompactValue for ZeroSizedDrop {}

#[test]
fn compact_vec_deque_counts_zero_sized_values_and_drops_them_once() {
    assert_eq!(core::mem::size_of::<ZeroSizedDrop>(), 0);
    ZERO_SIZED_DROPS.store(0, Ordering::SeqCst);
    StdArena::with_capacity(512, |arena| {
        let mut deque = CompactVecDeque::with_capacity(2, arena).unwrap();
        deque.push_back(ZeroSizedDrop, arena).unwrap();
        deque.push_front(ZeroSizedDrop, arena).unwrap();
        deque.push_back(ZeroSizedDrop, arena).unwrap();
        assert_eq!(deque.len(), 3);
        assert_eq!(deque.iter(arena).unwrap().count(), 3);
        drop(deque.pop_front(arena).unwrap().unwrap());
        deque.clear();
        assert_eq!(deque.len(), 0);
    })
    .unwrap();
    assert_eq!(ZERO_SIZED_DROPS.load(Ordering::SeqCst), 3);
}

#[test]
fn compact_vec_deque_makes_a_single_wrapped_entry_contiguous() {
    StdArena::with_capacity(512, |arena| {
        let mut deque = CompactVecDeque::with_capacity(4, arena).unwrap();
        deque.push_back(1_u32, arena).unwrap();
        assert_eq!(deque.pop_front(arena).unwrap(), Some(1));
        deque.push_front(7, arena).unwrap();
        assert_eq!(deque.make_contiguous(arena).unwrap(), &[7]);
        assert_eq!(
            deque
                .iter(arena)
                .unwrap()
                .copied()
                .collect::<std::vec::Vec<_>>(),
            [7]
        );
    })
    .unwrap();
}

#[test]
fn compact_ring_evicts_before_reusing_slots_and_drops_exactly_once() {
    let drops = Rc::new(Cell::new(0));
    StdArena::with_capacity(1024, |arena| {
        let mut ring = CompactRing::with_capacity(2, arena).unwrap();
        ring.push_back(DropCounter(Rc::clone(&drops)), arena)
            .unwrap();
        ring.push_back(DropCounter(Rc::clone(&drops)), arena)
            .unwrap();
        ring.push_back(DropCounter(Rc::clone(&drops)), arena)
            .unwrap();
        assert_eq!(ring.len(), 2);
        assert_eq!(drops.get(), 1);
        ring.clear();
        assert_eq!(drops.get(), 3);
        ring.push_back(DropCounter(Rc::clone(&drops)), arena)
            .unwrap();
        ring.clear();
    })
    .unwrap();
    assert_eq!(drops.get(), 4);
}

struct DropBomb {
    id: u8,
    drops: Rc<std::cell::RefCell<std::vec::Vec<u8>>>,
    panic_on_drop: bool,
}

impl Drop for DropBomb {
    fn drop(&mut self) {
        self.drops.as_ref().borrow_mut().push(self.id);
        if self.panic_on_drop {
            panic!("evicted value destructor panic");
        }
    }
}

// SAFETY: DropBomb is movable and owns its logging state; it contains no
// references into the arena.
unsafe impl CompactValue for DropBomb {}

#[test]
fn ring_eviction_panic_does_not_resurrect_the_removed_entry() {
    let drops = Rc::new(std::cell::RefCell::new(std::vec::Vec::new()));
    StdArena::with_capacity(1024, |arena| {
        let mut ring = CompactRing::with_capacity(2, arena).unwrap();
        ring.push_back(
            DropBomb {
                id: 1,
                drops: Rc::clone(&drops),
                panic_on_drop: true,
            },
            arena,
        )
        .unwrap();
        ring.push_back(
            DropBomb {
                id: 2,
                drops: Rc::clone(&drops),
                panic_on_drop: false,
            },
            arena,
        )
        .unwrap();

        let outcome = catch_unwind(AssertUnwindSafe(|| {
            ring.push_back(
                DropBomb {
                    id: 3,
                    drops: Rc::clone(&drops),
                    panic_on_drop: false,
                },
                arena,
            )
            .unwrap();
        }));
        assert!(outcome.is_err());
        assert_eq!(ring.len(), 1);
        assert_eq!(
            ring.iter(arena)
                .unwrap()
                .map(|value| value.id)
                .collect::<std::vec::Vec<_>>(),
            [2]
        );
        ring.push_back(
            DropBomb {
                id: 4,
                drops: Rc::clone(&drops),
                panic_on_drop: false,
            },
            arena,
        )
        .unwrap();
        ring.clear();
    })
    .unwrap();
    assert_eq!(*drops.as_ref().borrow(), [1, 3, 4, 2]);
}

#[test]
fn deque_drop_finishes_remaining_entries_after_a_destructor_panics() {
    let drops = Rc::new(std::cell::RefCell::new(std::vec::Vec::new()));
    StdArena::with_capacity(1024, |arena| {
        let mut deque = CompactVecDeque::with_capacity(3, arena).unwrap();
        for (id, panic_on_drop) in [(0, false), (1, true), (2, false)] {
            deque
                .push_back(
                    DropBomb {
                        id,
                        drops: Rc::clone(&drops),
                        panic_on_drop,
                    },
                    arena,
                )
                .unwrap();
        }
        let outcome = catch_unwind(AssertUnwindSafe(|| drop(deque)));
        assert!(outcome.is_err());
    })
    .unwrap();
    assert_eq!(*drops.as_ref().borrow(), [2, 1, 0]);
}

#[test]
fn small_vector_stays_inline_then_promotes_failure_safely() {
    StdArena::with_capacity(128, |arena| {
        let before = arena.used_bytes();
        let mut values = CompactSmallVec::<u16, 2>::new_in(arena);
        values.push_in(4, arena).unwrap();
        values.push_in(8, arena).unwrap();
        assert!(values.is_inline());
        assert_eq!(arena.used_bytes(), before);
        values.push_in(12, arena).unwrap();
        assert!(!values.is_inline());
        assert_eq!(values.as_slice(arena).unwrap(), &[4, 8, 12]);
    })
    .unwrap();

    StdArena::with_capacity(64, |arena| {
        let mut values = CompactSmallVec::<u32, 2>::new_in(arena);
        values.push_in(1, arena).unwrap();
        values.push_in(2, arena).unwrap();
        assert!(values.push_in(3, arena).is_err());
        assert!(values.is_inline());
        assert_eq!(values.as_slice(arena).unwrap(), &[1, 2]);
    })
    .unwrap();
}

#[test]
fn vector_grows_and_allocation_failure_preserves_existing_values() {
    StdArena::with_capacity(256, |arena| {
        let mut values = CompactVec::new_in(arena);
        values.push_in(3_u32, arena).unwrap();
        values.push_in(5, arena).unwrap();
        values.push_in(8, arena).unwrap();
        values.push_in(13, arena).unwrap();
        assert_eq!(values.capacity(), 4);
        assert_eq!(values.as_slice(arena).unwrap(), &[3, 5, 8, 13]);
        assert_eq!(values.pop_in(arena).unwrap(), Some(13));
        if let Some(value) = values.get_mut(0, arena).unwrap() {
            *value = 2;
        }
        assert_eq!(values.as_slice(arena).unwrap(), &[2, 5, 8]);
    })
    .unwrap();

    StdArena::with_capacity(64, |arena| {
        let mut values = CompactVec::with_capacity_in(2, arena).unwrap();
        values.push_in(10_u8, arena).unwrap();
        values.push_in(20, arena).unwrap();
        values.push_in(30, arena).unwrap();
        values.push_in(40, arena).unwrap();
        values.push_in(50, arena).unwrap();
        values.push_in(60, arena).unwrap();
        values.push_in(70, arena).unwrap();
        values.push_in(80, arena).unwrap();
        assert!(values.push_in(90, arena).is_err());
        assert_eq!(
            values.as_slice(arena).unwrap(),
            &[10, 20, 30, 40, 50, 60, 70, 80]
        );
    })
    .unwrap();
}

#[test]
fn compact_string_transitions_between_inline_and_arena_storage() {
    StdArena::with_capacity(256, |arena| {
        let mut text = CompactString::from_str_in("abcdefghijkl", arena).unwrap();
        assert_eq!(text.capacity(), 12);
        assert_eq!(text.as_str(arena).unwrap(), "abcdefghijkl");
        text.push_char_in('é', arena).unwrap();
        let text_view = text.as_str(arena).unwrap();
        assert_eq!(text_view, "abcdefghijklé");
        assert_eq!(text_view.as_ptr(), text.as_bytes(arena).unwrap().as_ptr());
        assert!(text.truncate_in(13, arena).is_err());
        text.truncate_in(12, arena).unwrap();
        assert_eq!(text.as_str(arena).unwrap(), "abcdefghijkl");
        assert_eq!(text.capacity(), 12);
        text.clear();
        assert!(text.is_empty());
    })
    .unwrap();

    StdArena::with_capacity(80, |arena| {
        let mut text = CompactString::from_str_in("abcdefghijklmnopqrst", arena).unwrap();
        assert!(text
            .push_str_in(" this addition will not fit", arena)
            .is_err());
        assert_eq!(text.as_str(arena).unwrap(), "abcdefghijklmnopqrst");
    })
    .unwrap();
}

#[test]
fn packed_boolean_vector_preserves_bits_when_growing_and_mutating() {
    StdArena::with_capacity(128, |arena| {
        let mut bits = CompactBitVec::new_in(arena);
        for index in 0..19 {
            bits.push_in(index % 3 == 1, arena).unwrap();
        }
        assert_eq!(bits.len(), 19);
        for index in 0..19 {
            assert_eq!(bits.get(index, arena).unwrap(), Some(index % 3 == 1));
        }
        bits.set(7, true, arena).unwrap();
        assert_eq!(bits.get(7, arena).unwrap(), Some(true));
        bits.clear(arena).unwrap();
        assert!(bits.is_empty());
    })
    .unwrap();
}

#[test]
fn slab_reuses_slots_and_rejects_stale_or_foreign_handles() {
    StdArena::with_capacity(256, |arena| {
        let mut slab = CompactSlab::with_capacity_in(1, arena).unwrap();
        let mut other = CompactSlab::with_capacity_in(1, arena).unwrap();
        let first = slab.insert(7_u32, arena).unwrap().unwrap();
        assert_eq!(*slab.get(first, arena).unwrap(), 7);
        assert_eq!(slab.remove(first, arena).unwrap(), 7);
        assert_eq!(
            slab.get(first, arena).unwrap_err(),
            CollectionError::StaleHandle
        );

        let second = slab.insert(9, arena).unwrap().unwrap();
        assert_ne!(first.generation(), second.generation());
        assert_eq!(*slab.get(second, arena).unwrap(), 9);

        let foreign = other.insert(11, arena).unwrap().unwrap();
        assert_eq!(
            slab.get(foreign, arena).unwrap_err(),
            CollectionError::StaleHandle
        );
        assert_eq!(slab.insert(12, arena).unwrap(), None);
    })
    .unwrap();

    StdArena::with_capacity(512, |arena| {
        let mut old = CompactSlab::with_capacity_in(1, arena).unwrap();
        let stale = old.insert(1_u32, arena).unwrap().unwrap();
        drop(old);

        let mut replacement = CompactSlab::with_capacity_in(1, arena).unwrap();
        let current = replacement.insert(2_u32, arena).unwrap().unwrap();
        assert_ne!(stale, current);
        assert_eq!(
            replacement.get(stale, arena).unwrap_err(),
            CollectionError::StaleHandle
        );
    })
    .unwrap();
}

#[test]
fn interner_deduplicates_canonical_bytes_and_strings() {
    StdArena::with_capacity(512, |arena| {
        let baseline = arena.used_bytes();
        let mut interner = CompactInterner::new_in(arena).unwrap();
        let first = interner.intern_str("repeated", arena).unwrap();
        let second = interner.intern_bytes(b"repeated", arena).unwrap();
        let third = interner.intern_str("other", arena).unwrap();
        assert_eq!(first, second);
        assert_ne!(first, third);
        assert_eq!(interner.len(), 2);
        assert_eq!(interner.resolve_str(first, arena).unwrap(), "repeated");
        assert_eq!(interner.resolve_bytes(third, arena).unwrap(), b"other");
        assert_eq!(
            interner.intern_bytes(b"", arena).unwrap(),
            interner.intern_str("", arena).unwrap()
        );
        drop(interner);
        assert_eq!(arena.used_bytes(), baseline);
    })
    .unwrap();
}

#[test]
fn vector_moves_and_drops_non_copy_values_exactly_once() {
    let drops = Rc::new(Cell::new(0));
    StdArena::with_capacity(1024, |arena| {
        let mut values = CompactVec::with_capacity_in(1, arena).unwrap();
        for _ in 0..6 {
            values
                .push_in(DropCounter(Rc::clone(&drops)), arena)
                .unwrap();
        }
        assert_eq!(drops.get(), 0, "growth moves values without dropping them");

        let popped = values.pop_in(arena).unwrap().unwrap();
        assert_eq!(drops.get(), 0);
        drop(popped);
        assert_eq!(drops.get(), 1);

        values.truncate(2);
        assert_eq!(drops.get(), 4);
        values
            .push_in(DropCounter(Rc::clone(&drops)), arena)
            .unwrap();
        values.clear();
        assert_eq!(drops.get(), 7);
    })
    .unwrap();
    assert_eq!(drops.get(), 7);
}

#[test]
fn box_smallvec_and_slab_drop_owned_values_once() {
    let drops = Rc::new(Cell::new(0));
    StdArena::with_capacity(2048, |arena| {
        let boxed = CompactBox::new_in(DropCounter(Rc::clone(&drops)), arena).unwrap();
        assert_eq!(drops.get(), 0);
        drop(boxed);
        assert_eq!(drops.get(), 1);

        let mut small = CompactSmallVec::<DropCounter, 2>::new_in(arena);
        for _ in 0..3 {
            small
                .push_in(DropCounter(Rc::clone(&drops)), arena)
                .unwrap();
        }
        assert!(!small.is_inline());
        assert_eq!(drops.get(), 1, "promotion transfers each value");
        drop(small);
        assert_eq!(drops.get(), 4);

        let mut slab = CompactSlab::with_capacity_in(3, arena).unwrap();
        let handles = (0..3)
            .map(|_| {
                slab.insert(DropCounter(Rc::clone(&drops)), arena)
                    .unwrap()
                    .unwrap()
            })
            .collect::<std::vec::Vec<_>>();
        let removed = slab.remove(handles[0], arena).unwrap();
        assert_eq!(drops.get(), 4);
        drop(removed);
        assert_eq!(drops.get(), 5);
        drop(slab);
        assert_eq!(drops.get(), 7);
    })
    .unwrap();
    assert_eq!(drops.get(), 7);
}

#[test]
fn vector_growth_and_drop_reuse_bounded_arena_storage() {
    StdArena::with_capacity(1024, |arena| {
        let baseline = arena.used_bytes();
        {
            let mut values = CompactVec::new_in(arena);
            for value in 0..32_u32 {
                values.push_in(value, arena).unwrap();
            }
            assert_eq!(values.capacity(), 32);
            assert!(arena.used_bytes() < baseline + 20 + 32 * 4 + 32);
        }
        assert_eq!(arena.used_bytes(), baseline);

        let high_water = arena.used_bytes();
        for _ in 0..8 {
            let mut values = CompactVec::with_capacity_in(32, arena).unwrap();
            for value in 0..32_u32 {
                values.push_in(value, arena).unwrap();
            }
            drop(values);
            assert_eq!(arena.used_bytes(), high_water);
        }
    })
    .unwrap();
}

#[test]
fn string_growth_and_clear_reclaim_or_reuse_heap_storage() {
    StdArena::with_capacity(1024, |arena| {
        let baseline = arena.used_bytes();
        let mut text = CompactString::from_str_in("a string longer than inline", arena).unwrap();
        assert!(text.capacity() > 12);
        arena.alloc_bytes(b"allocation barrier").unwrap();
        text.push_str_in(" plus additional text that grows again", arena)
            .unwrap();
        assert_eq!(
            text.as_str(arena).unwrap(),
            "a string longer than inline plus additional text that grows again"
        );
        let retained = text.capacity();
        text.clear();
        assert_eq!(text.capacity(), retained);
        assert!(text.is_empty());
        drop(text);
        assert!(arena.used_bytes() >= baseline);

        let tail_baseline = arena.used_bytes();
        let text = CompactString::from_str_in("a long heap-backed string", arena).unwrap();
        drop(text);
        assert_eq!(arena.used_bytes(), tail_baseline);
    })
    .unwrap();
}

#[test]
fn owner_backed_vector_string_and_box_traits_match_std() {
    let native_values = std::vec![3_u32, 5, 8, 13];
    StdArena::with_capacity(4096, |arena| {
        let mut values = CompactVec::new_in(arena);
        for value in &native_values {
            values.push_in(*value, arena).unwrap();
        }

        assert_eq!(&*values, native_values.as_slice());
        let as_ref: &[u32] = values.as_ref();
        let borrowed: &[u32] = Borrow::borrow(&values);
        assert_eq!(as_ref, native_values.as_slice());
        assert_eq!(borrowed, native_values.as_slice());
        assert_eq!(values[2], native_values[2]);
        assert_eq!(values.as_slice(arena).unwrap(), native_values.as_slice());
        assert_eq!(values, values);
        assert_eq!(
            values.partial_cmp(&values),
            Some(core::cmp::Ordering::Equal)
        );
        assert_eq!(values.cmp(&values), core::cmp::Ordering::Equal);
        assert_eq!(format!("{values:?}"), format!("{native_values:?}"));
        assert_eq!(hash_value(&values), hash_value(&native_values));
        assert_eq!(
            (&values).into_iter().copied().collect::<std::vec::Vec<_>>(),
            native_values
        );

        for value in &mut values {
            *value += 1;
        }
        let mut incremented = native_values.clone();
        for value in &mut incremented {
            *value += 1;
        }
        let as_mut: &mut [u32] = values.as_mut();
        as_mut[0] += 1;
        let borrowed_mut: &mut [u32] = BorrowMut::borrow_mut(&mut values);
        borrowed_mut[0] += 1;
        values[1] += 1;
        incremented[0] += 1;
        incremented[0] += 1;
        incremented[1] += 1;
        assert_eq!(&*values, incremented.as_slice());

        let mut boxed = CompactBox::new_in(41_u32, arena).unwrap();
        *boxed += 1;
        *boxed.as_mut() += 1;
        let as_ref: &u32 = boxed.as_ref();
        assert_eq!(*boxed, 43);
        assert_eq!(*as_ref, *std::boxed::Box::new(43));
        assert_eq!(boxed, boxed);
        assert_eq!(boxed.partial_cmp(&boxed), Some(core::cmp::Ordering::Equal));
        assert_eq!(boxed.cmp(&boxed), core::cmp::Ordering::Equal);
        assert_eq!(
            format!("{boxed:?}"),
            format!("{:?}", std::boxed::Box::new(43))
        );
        assert_eq!(
            hash_value(&boxed),
            hash_value(&std::boxed::Box::new(43_u32))
        );

        let mut text = CompactString::from_str_in("héllo", arena).unwrap();
        let as_ref: &str = text.as_ref();
        let borrowed: &str = Borrow::borrow(&text);
        assert_eq!(as_ref, "héllo");
        assert_eq!(borrowed, "héllo");
        assert_eq!(&*text, "héllo");
        assert!(text == "héllo");
        assert!(<CompactString as PartialEq<str>>::eq(&text, "héllo"));
        assert_eq!(format!("{text}"), "héllo");
        assert_eq!(format!("{text:?}"), format!("{:?}", String::from("héllo")));
        assert_eq!(hash_value(&text), hash_value(&String::from("héllo")));
        assert_eq!(text, text);

        {
            let mut writer = text.writer(arena);
            write!(&mut writer, " compact/{}", 2).unwrap();
        }
        let native_text = String::from("héllo compact/2");
        assert_eq!(&*text, native_text);
        assert_eq!(text.as_str(arena).unwrap(), native_text);
        assert!(text > CompactString::from_str_in("héllo", arena).unwrap());
    })
    .unwrap();
}
