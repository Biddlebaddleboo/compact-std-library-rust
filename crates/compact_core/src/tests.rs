use core::mem::{align_of, MaybeUninit};
use std::cell::{Cell, RefCell};
use std::format;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use crate::{
    bits_required, checked_align_up, smallest_word, with_arena, with_arena_attached,
    with_arena_persistent, BitField, ByteRange32, Error, Offset32, PackedWord, StableBacking,
    StorageWord,
};

struct TestBacking {
    bytes: std::vec::Vec<MaybeUninit<u8>>,
}

impl TestBacking {
    fn new(capacity: usize) -> Self {
        Self {
            bytes: std::vec![MaybeUninit::uninit(); capacity],
        }
    }
}

// SAFETY: the Vec is not resized while the returned mutable slice is borrowed;
// its heap allocation remains stable for the borrow's duration.
unsafe impl StableBacking for TestBacking {
    fn bytes_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        &mut self.bytes
    }
}

#[test]
fn offsets_are_four_bytes_for_representative_types() {
    assert_eq!(core::mem::size_of::<Offset32<'static, u8>>(), 4);
    assert_eq!(core::mem::size_of::<Offset32<'static, u64>>(), 4);
}

#[test]
fn seeded_allocator_operations_match_a_reference_model() {
    const SEED: u64 = 0x7A91_3D52_C4E8_0B6F;
    const CAPACITY: usize = 32 * 1024;
    let operation_count = if cfg!(miri) { 128 } else { 1_500 };

    struct ModelAllocation {
        values: std::vec::Vec<u8>,
        capacity: usize,
    }

    struct SeededRng(u64);

    impl SeededRng {
        fn next(&mut self) -> u64 {
            let mut value = self.0;
            value ^= value << 13;
            value ^= value >> 7;
            value ^= value << 17;
            self.0 = value;
            value
        }
    }

    let mut backing = TestBacking::new(CAPACITY);
    with_arena(&mut backing, |arena| {
        let mut rng = SeededRng(SEED);
        let mut owners: std::vec::Vec<Option<crate::ArenaAllocation<'_, u8>>> =
            std::vec::Vec::new();
        let mut model: std::vec::Vec<Option<ModelAllocation>> = std::vec::Vec::new();
        let mut trace = std::vec::Vec::new();

        for step in 0..operation_count {
            let operation = (rng.next() % 6) as u8;
            if operation == 0 {
                let index = owners
                    .iter()
                    .position(Option::is_none)
                    .unwrap_or(owners.len());
                if index == owners.len() {
                    owners.push(None);
                    model.push(None);
                }
                let capacity = 1 + (rng.next() % 32) as usize;
                let len = (rng.next() as usize) % (capacity + 1);
                let first = rng.next() as u8;
                trace.push(format!(
                    "{step}: allocate slot={index} cap={capacity} len={len}"
                ));
                if let Ok(mut allocation) = arena.alloc_owned_slice::<u8>(capacity) {
                    let mut values = std::vec::Vec::with_capacity(len);
                    for offset in 0..len {
                        let value = first.wrapping_add((offset as u8).wrapping_mul(29));
                        allocation.push(value).unwrap();
                        values.push(value);
                    }
                    owners[index] = Some(allocation);
                    model[index] = Some(ModelAllocation { values, capacity });
                }
            } else if !owners.is_empty() {
                let index = (rng.next() as usize) % owners.len();
                match operation {
                    1 => {
                        trace.push(format!("{step}: release slot={index}"));
                        owners[index] = None;
                        model[index] = None;
                    }
                    2 => {
                        if let (Some(owner), Some(expected)) =
                            (owners[index].as_mut(), model[index].as_mut())
                        {
                            let requested = expected.capacity + 1 + (rng.next() % 8) as usize;
                            trace.push(format!("{step}: grow slot={index} cap={requested}"));
                            if arena.try_resize_owned(owner, requested).unwrap() {
                                expected.capacity = requested;
                            }
                        }
                    }
                    3 => {
                        if let (Some(owner), Some(expected)) =
                            (owners[index].as_mut(), model[index].as_mut())
                        {
                            let spare = expected.capacity - expected.values.len();
                            let requested = expected.values.len()
                                + if spare == 0 {
                                    0
                                } else {
                                    (rng.next() as usize) % (spare + 1)
                                };
                            trace.push(format!("{step}: shrink slot={index} cap={requested}"));
                            if arena.try_resize_owned(owner, requested).unwrap() {
                                expected.capacity = requested;
                            }
                        }
                    }
                    4 => {
                        if let (Some(owner), Some(expected)) =
                            (owners[index].as_mut(), model[index].as_mut())
                        {
                            if !expected.values.is_empty() {
                                let element = (rng.next() as usize) % expected.values.len();
                                let value = rng.next() as u8;
                                trace.push(format!(
                                    "{step}: write slot={index} element={element} value={value}"
                                ));
                                *owner.get_mut(element).unwrap() = value;
                                expected.values[element] = value;
                            }
                        }
                    }
                    _ => {
                        trace.push(format!("{step}: verify slot={index}"));
                    }
                }
            }

            let mut live_ranges: std::vec::Vec<_> = owners
                .iter()
                .filter_map(|owner| {
                    owner
                        .as_ref()
                        .map(crate::ArenaAllocation::debug_block_range)
                })
                .collect();
            live_ranges.sort_unstable();
            for pair in live_ranges.windows(2) {
                assert!(
                    pair[0].1 <= pair[1].0,
                    "overlapping live blocks; seed={SEED:#x}; trace={trace:?}"
                );
            }
            for (index, (owner, expected)) in owners.iter().zip(&model).enumerate() {
                if let (Some(owner), Some(expected)) = (owner, expected) {
                    assert_eq!(
                        owner.len(),
                        expected.values.len(),
                        "seed={SEED:#x}; trace={trace:?}"
                    );
                    assert_eq!(
                        owner.capacity(),
                        expected.capacity,
                        "seed={SEED:#x}; trace={trace:?}"
                    );
                    assert_eq!(
                        owner.as_slice(),
                        expected.values,
                        "slot={index}; seed={SEED:#x}; trace={trace:?}"
                    );
                } else {
                    assert!(
                        owner.is_none() && expected.is_none(),
                        "slot={index}; seed={SEED:#x}; trace={trace:?}"
                    );
                }
            }
            assert!(
                arena.used_bytes() <= CAPACITY,
                "cursor out of bounds; seed={SEED:#x}; trace={trace:?}"
            );
            assert!(
                arena.debug_validate_allocator(&live_ranges).is_ok(),
                "allocator structure invalid; seed={SEED:#x}; used={}; ranges={live_ranges:?}; trace={trace:?}",
                arena.used_bytes(),
            );
        }
    })
    .unwrap();
}

#[test]
fn owner_drop_releases_its_block_after_a_destructor_panics() {
    struct DropProbe {
        drops: Rc<Cell<usize>>,
        panic: bool,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
            assert!(!self.panic, "intentional destructor panic");
        }
    }

    // SAFETY: DropProbe has no address-sensitive state; moving it preserves
    // its Rc ownership, and its destructor is valid while the arena is alive.
    unsafe impl crate::CompactValue for DropProbe {}

    let mut backing = TestBacking::new(512);
    with_arena(&mut backing, |arena| {
        let baseline = arena.used_bytes();
        let drops = Rc::new(Cell::new(0));
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut allocation = arena.alloc_owned_slice::<DropProbe>(2).unwrap();
            allocation
                .push(DropProbe {
                    drops: Rc::clone(&drops),
                    panic: true,
                })
                .unwrap();
            allocation
                .push(DropProbe {
                    drops: Rc::clone(&drops),
                    panic: false,
                })
                .unwrap();
            drop(allocation);
        }));

        assert!(result.is_err());
        assert_eq!(drops.get(), 2);
        assert_eq!(arena.used_bytes(), baseline);
        let reused = arena.alloc_owned_slice::<u8>(64).unwrap();
        assert_eq!(reused.capacity(), 64);
    })
    .unwrap();
}

#[test]
fn persistent_arena_reattaches_without_resetting_allocator_state() {
    let mut backing = TestBacking::new(512);
    let (root, used_after_first_access) = with_arena_persistent(&mut backing, |arena| {
        let recyclable = arena.alloc_owned_slice::<u8>(32).unwrap();
        let root = arena.alloc_value(41_u32).unwrap();
        drop(recyclable);
        (root.as_u32(), arena.used_bytes())
    })
    .unwrap();

    // SAFETY: this is the exact backing initialized above; no other arena is
    // attached and the backing remains at the same address.
    let used_after_second_access = unsafe {
        with_arena_attached(&mut backing, |arena| {
            // SAFETY: `root` was returned by `alloc_value` in this persistent
            // backing and has not been released or overwritten.
            let root: Offset32<'_, u32> = Offset32::from_persistent_raw_unchecked(root);
            assert_eq!(*arena.get(root).unwrap(), 41);
            assert_eq!(arena.used_bytes(), used_after_first_access);
            let reused = arena.alloc_value(73_u32).unwrap();
            assert_eq!(*arena.get(reused).unwrap(), 73);
            assert_eq!(arena.used_bytes(), used_after_first_access);
            arena.alloc_value(73_u64).unwrap();
            arena.alloc_value(74_u64).unwrap();
            arena.used_bytes()
        })
    }
    .unwrap();
    assert!(used_after_second_access > used_after_first_access);

    // SAFETY: the same backing remains alive and only one attachment is active.
    unsafe {
        with_arena_attached(&mut backing, |arena| {
            assert_eq!(arena.used_bytes(), used_after_second_access);
        })
    }
    .unwrap();
}

#[test]
fn persistent_arena_rejects_a_corrupted_header() {
    let mut backing = TestBacking::new(256);
    with_arena_persistent(&mut backing, |arena| {
        arena.alloc_value(1_u8).unwrap();
    })
    .unwrap();
    // SAFETY: persistent initialization wrote the full header, including byte 0.
    let first = unsafe { backing.bytes[0].assume_init() };
    backing.bytes[0] = MaybeUninit::new(first ^ 0x80);

    // SAFETY: this is still the same initialized backing, with only its header
    // deliberately corrupted to exercise attach-time validation.
    let error = unsafe { with_arena_attached(&mut backing, |_| ()) }.unwrap_err();
    assert_eq!(error, Error::InitializationError);
}

#[test]
fn values_round_trip_mutate_and_align() {
    #[repr(align(32))]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Aligned(u8);

    let mut backing = TestBacking::new(256);
    with_arena(&mut backing, |arena| {
        let number = arena.alloc_value(17_u32).unwrap();
        assert_eq!(*arena.get(number).unwrap(), 17);
        *arena.get_mut(number).unwrap() = 28;
        assert_eq!(*arena.get(number).unwrap(), 28);

        let aligned = arena.alloc_value(Aligned(9)).unwrap();
        let view = arena.get(aligned).unwrap();
        assert_eq!(*view, Aligned(9));
        assert_eq!((view as *const Aligned as usize) % align_of::<Aligned>(), 0);
    })
    .unwrap();
}

#[test]
fn borrowed_copy_values_remain_bounded_by_their_referent() {
    let mut backing = TestBacking::new(128);
    let source = 73_u32;
    with_arena(&mut backing, |arena| {
        let borrowed = arena.alloc_value(&source).unwrap();
        assert_eq!(**arena.get(borrowed).unwrap(), 73);
    })
    .unwrap();
}

#[test]
fn compact_offsets_can_be_stored_inside_other_arena_values() {
    #[derive(Clone, Copy)]
    struct Link<'arena> {
        target: Offset32<'arena, u8>,
    }

    let mut backing = TestBacking::new(128);
    with_arena(&mut backing, |arena| {
        let target = arena.alloc_value(42_u8).unwrap();
        let link = arena.alloc_value(Link { target }).unwrap();
        let link = arena.get(link).unwrap();
        assert_eq!(*arena.get(link.target).unwrap(), 42);
    })
    .unwrap();
}

#[test]
fn slices_are_zero_copy_and_support_exclusive_mutation() {
    let mut backing = TestBacking::new(256);
    let base = backing.bytes.as_mut_ptr().cast::<u8>() as usize;
    with_arena(&mut backing, |arena| {
        let source = [3_u16, 5, 8, 13];
        let slice = arena.alloc_slice(&source).unwrap();
        let view = arena.get_slice(slice).unwrap();
        assert_eq!(view, &source);
        assert_eq!(
            view.as_ptr() as usize,
            base + slice.offset().as_u32() as usize
        );

        arena.get_slice_mut(slice).unwrap()[2] = 21;
        assert_eq!(arena.get_slice(slice).unwrap(), &[3, 5, 21, 13]);
    })
    .unwrap();
}

#[test]
fn uninitialized_storage_requires_matching_initialization() {
    let mut backing = TestBacking::new(128);
    with_arena(&mut backing, |arena| {
        let slot = arena.alloc_uninit::<u32>().unwrap();
        let initialized = arena.write_uninit(slot, 99).unwrap();
        assert_eq!(*arena.get(initialized).unwrap(), 99);

        let slots = arena.alloc_uninit_slice::<u16>(2).unwrap();
        assert_eq!(
            arena.write_uninit_slice(slots, &[1_u16]).unwrap_err(),
            Error::InitializationError
        );
        let values = arena.write_uninit_slice(slots, &[7_u16, 11]).unwrap();
        assert_eq!(arena.get_slice(values).unwrap(), &[7, 11]);
    })
    .unwrap();
}

#[test]
fn null_invalid_and_exhausted_offsets_are_rejected() {
    let mut backing = TestBacking::new(crate::MIN_ARENA_BYTES);
    with_arena(&mut backing, |arena| {
        assert!(arena.get(Offset32::<u8>::null()).is_err());
        let value = arena.alloc_value(1_u8).unwrap();
        // SAFETY: this deliberately supplies an out-of-bounds raw offset; the
        // checked resolver must reject it before constructing a pointer.
        let invalid = unsafe { Offset32::<u8>::from_raw_unchecked(100) };
        assert_eq!(arena.get(invalid).unwrap_err(), Error::OutOfBounds);
        assert_eq!(*arena.get(value).unwrap(), 1);
        assert_eq!(
            arena.alloc_slice(&[1_u8; 4]).unwrap_err(),
            Error::AllocationExhausted
        );
    })
    .unwrap();
}

#[test]
fn zero_sized_allocations_get_distinct_non_null_offsets() {
    #[derive(Clone, Copy)]
    struct Empty;

    let mut backing = TestBacking::new(128);
    with_arena(&mut backing, |arena| {
        let first = arena.alloc_value(Empty).unwrap();
        let second = arena.alloc_value(Empty).unwrap();
        assert!(!first.is_null());
        assert_ne!(first.as_u32(), second.as_u32());
        assert!(arena.get(first).is_ok());
        assert!(arena.get(second).is_ok());
    })
    .unwrap();
}

#[test]
fn scratch_scopes_nest_align_and_release_their_storage() {
    #[repr(align(64))]
    #[derive(Clone, Copy)]
    struct Aligned(u8);
    #[derive(Clone, Copy)]
    struct Empty;

    let mut backing = TestBacking::new(1024);
    with_arena(&mut backing, |arena| {
        let initial_used = arena.used_bytes();
        let result = arena
            .scratch(512, |scratch| {
                let aligned = scratch.alloc_value(Aligned(17)).unwrap();
                assert_eq!(scratch.get(aligned).unwrap().0, 17);
                assert_eq!(
                    (scratch.get(aligned).unwrap() as *const Aligned as usize)
                        % align_of::<Aligned>(),
                    0
                );
                let first_empty = scratch.alloc_value(Empty).unwrap();
                let second_empty = scratch.alloc_value(Empty).unwrap();
                assert_ne!(first_empty.as_u32(), second_empty.as_u32());

                scratch
                    .scratch(128, |nested| {
                        let value = nested.alloc_value(91_u32).unwrap();
                        assert_eq!(*nested.get(value).unwrap(), 91);
                    })
                    .unwrap();
                29
            })
            .unwrap();
        assert_eq!(result, 29);
        assert_eq!(arena.used_bytes(), initial_used);
    })
    .unwrap();
}

struct DropTrace {
    id: u8,
    drops: Rc<RefCell<std::vec::Vec<u8>>>,
    panic: bool,
}

// SAFETY: DropTrace is movable and its owned Rc remains valid until its
// destructor runs; it contains no references into the arena.
unsafe impl crate::CompactValue for DropTrace {}

impl Drop for DropTrace {
    fn drop(&mut self) {
        self.drops.borrow_mut().push(self.id);
        if self.panic {
            panic!("test destructor panic");
        }
    }
}

#[test]
fn scratch_drops_values_in_reverse_construction_order() {
    let drops = Rc::new(RefCell::new(std::vec::Vec::new()));
    let mut backing = TestBacking::new(512);
    with_arena(&mut backing, |arena| {
        arena
            .scratch(256, |scratch| {
                let mut values = scratch.alloc_owned_slice::<DropTrace>(3).unwrap();
                for id in 0..3 {
                    values
                        .push(DropTrace {
                            id,
                            drops: Rc::clone(&drops),
                            panic: false,
                        })
                        .unwrap();
                }
            })
            .unwrap();
    })
    .unwrap();
    assert_eq!(*drops.borrow(), [2, 1, 0]);
}

#[test]
fn scratch_cleanup_continues_after_one_destructor_panics() {
    let drops = Rc::new(RefCell::new(std::vec::Vec::new()));
    let mut backing = TestBacking::new(512);
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        with_arena(&mut backing, |arena| {
            let _ = arena.scratch(256, |scratch| {
                let mut values = scratch.alloc_owned_slice::<DropTrace>(3).unwrap();
                for id in 0..3 {
                    values
                        .push(DropTrace {
                            id,
                            drops: Rc::clone(&drops),
                            panic: id == 1,
                        })
                        .unwrap();
                }
                drop(values);
            });
        })
        .unwrap();
    }));
    assert!(outcome.is_err());
    assert_eq!(*drops.borrow(), [2, 1, 0]);
}

#[test]
fn scratch_reports_too_small_and_exhausted_regions() {
    let mut backing = TestBacking::new(128);
    with_arena(&mut backing, |arena| {
        assert_eq!(
            arena
                .scratch(crate::MIN_ARENA_BYTES - 1, |_| ())
                .unwrap_err(),
            Error::InvalidCapacity
        );
        assert_eq!(
            arena.scratch(128, |_| ()).unwrap_err(),
            Error::AllocationExhausted
        );
    })
    .unwrap();
}

#[test]
fn smallest_usable_backing_fits_one_byte() {
    let mut backing = TestBacking::new(crate::MIN_ARENA_BYTES);
    with_arena(&mut backing, |arena| {
        let only_value = arena.alloc_value(5_u8).unwrap();
        assert_eq!(*arena.get(only_value).unwrap(), 5);
        assert_eq!(
            arena.used_bytes() + arena.remaining_bytes(),
            arena.capacity()
        );
    })
    .unwrap();
}

#[test]
fn independent_arenas_have_independent_memory() {
    let mut first = TestBacking::new(128);
    let mut second = TestBacking::new(128);
    let first_value = with_arena(&mut first, |arena| {
        let offset = arena.alloc_value(10_u8).unwrap();
        *arena.get(offset).unwrap()
    })
    .unwrap();
    let second_value = with_arena(&mut second, |arena| {
        let offset = arena.alloc_value(20_u8).unwrap();
        *arena.get(offset).unwrap()
    })
    .unwrap();
    assert_eq!(first_value, 10);
    assert_eq!(second_value, 20);
}

#[test]
fn checked_layout_helpers_cover_boundaries() {
    assert_eq!(bits_required(0), 0);
    assert_eq!(bits_required(1), 1);
    assert_eq!(bits_required(3), 2);
    assert_eq!(bits_required(7), 3);
    assert_eq!(bits_required(255), 8);
    assert_eq!(bits_required(256), 9);
    assert_eq!(bits_required(65_535), 16);
    assert_eq!(bits_required(65_536), 17);
    assert_eq!(bits_required(u32::MAX as u64), 32);
    assert_eq!(bits_required(u64::MAX), 64);

    assert_eq!(smallest_word(1), Ok(StorageWord::U8));
    assert_eq!(smallest_word(8), Ok(StorageWord::U8));
    assert_eq!(smallest_word(9), Ok(StorageWord::U16));
    assert_eq!(smallest_word(16), Ok(StorageWord::U16));
    assert_eq!(smallest_word(17), Ok(StorageWord::U32));
    assert_eq!(smallest_word(32), Ok(StorageWord::U32));
    assert_eq!(smallest_word(33), Ok(StorageWord::U64));
    assert_eq!(smallest_word(64), Ok(StorageWord::U64));
    assert_eq!(smallest_word(0), Err(Error::InvalidBitRange));

    assert_eq!(checked_align_up(9, 8), Ok(16));
    assert_eq!(checked_align_up(16, 8), Ok(16));
    assert_eq!(checked_align_up(1, 3), Err(Error::AlignmentError));
    assert_eq!(checked_align_up(usize::MAX, 2), Err(Error::OffsetOverflow));
}

fn round_trip_word<W: PackedWord + core::fmt::Debug + Eq>(word: W, bits: u8) {
    let field = BitField::new(0, bits, W::BITS).unwrap();
    let maximum = if bits == 64 {
        u64::MAX
    } else {
        (1_u64 << bits) - 1
    };
    let inserted = field.insert(word, maximum).unwrap();
    assert_eq!(field.extract(inserted).unwrap(), maximum);
    if bits != 64 {
        assert_eq!(
            field.insert(inserted, maximum + 1).unwrap_err(),
            Error::ValueDoesNotFit
        );
    }
}

#[test]
fn packed_operations_cover_each_word_and_preserve_neighbors() {
    round_trip_word(0_u8, 8);
    round_trip_word(0_u16, 16);
    round_trip_word(0_u32, 32);
    round_trip_word(0_u64, 64);

    let byte_field = BitField::new(2, 3, 8).unwrap();
    let original = 0b1010_0011_u8;
    let updated = byte_field.insert(original, 0b101).unwrap();
    assert_eq!(byte_field.extract(updated), Ok(0b101));
    assert_eq!(updated & !0b0001_1100, original & !0b0001_1100);

    let first_bit = BitField::new(0, 1, 8).unwrap();
    let last_bit = BitField::new(7, 1, 8).unwrap();
    assert_eq!(first_bit.read_bool(0b1000_0001_u8), Ok(true));
    assert_eq!(last_bit.read_bool(0b0000_0001_u8), Ok(false));
    assert_eq!(last_bit.insert_bool(0b0000_0001_u8, true), Ok(0b1000_0001));
    assert_eq!(last_bit.insert_bool(0b1000_0001_u8, false), Ok(0b0000_0001));
    let middle_bit = BitField::new(3, 1, 8).unwrap();
    let mut grouped = 0_u8;
    grouped = first_bit.insert_bool(grouped, true).unwrap();
    grouped = middle_bit.insert_bool(grouped, true).unwrap();
    grouped = last_bit.insert_bool(grouped, true).unwrap();
    grouped = middle_bit.insert_bool(grouped, false).unwrap();
    assert_eq!(grouped, 0b1000_0001);
    assert_eq!(BitField::new(7, 2, 8), Err(Error::InvalidBitRange));
    assert_eq!(BitField::new(0, 0, 8), Err(Error::InvalidBitRange));
    assert_eq!(
        BitField::new(7, 2, 16).unwrap().extract(0_u8),
        Err(Error::InvalidBitRange)
    );
}

#[test]
fn initialized_byte_ranges_are_exact_zero_copy_views() {
    let mut backing = TestBacking::new(256);
    let base = backing.bytes.as_mut_ptr().cast::<u8>() as usize;
    with_arena(&mut backing, |arena| {
        let bytes = arena.alloc_bytes(b"abc\0tail").unwrap();
        let view = arena.get_bytes(bytes).unwrap();
        assert_eq!(view, b"abc\0tail");
        assert_eq!(view.as_ptr() as usize, base + bytes.offset() as usize);

        arena.get_bytes_mut(bytes).unwrap()[0] = b'A';
        assert_eq!(arena.get_bytes(bytes).unwrap(), b"Abc\0tail");

        let copied = arena.alloc_zeroed_bytes(bytes.len()).unwrap();
        arena.copy_bytes(bytes, copied, bytes.len()).unwrap();
        assert_eq!(arena.get_bytes(copied).unwrap(), b"Abc\0tail");
        assert_eq!(arena.get_bytes(ByteRange32::empty()).unwrap(), b"");
        // SAFETY: this view is immediately discarded before any allocator
        // mutation or owner drop can occur.
        assert!(unsafe { arena.used_uninit_bytes() }.len() >= arena.used_bytes());
    })
    .unwrap();
}

#[test]
fn owned_allocation_release_reuses_ranges_and_contracts_the_tail() {
    let mut backing = TestBacking::new(256);
    with_arena(&mut backing, |arena| {
        let baseline = arena.used_bytes();
        let first = arena.alloc_owned_slice::<u8>(8).unwrap();
        let middle = arena.alloc_owned_slice::<u8>(8).unwrap();
        let last = arena.alloc_owned_slice::<u8>(8).unwrap();
        let high_water = arena.used_bytes();

        drop(middle);
        assert_eq!(arena.used_bytes(), high_water);
        let reused = arena.alloc_owned_slice::<u8>(8).unwrap();
        assert_eq!(arena.used_bytes(), high_water);

        drop(last);
        let after_tail = arena.used_bytes();
        drop(first);
        drop(reused);
        assert!(after_tail < high_water);
        assert_eq!(arena.used_bytes(), baseline);
    })
    .unwrap();
}

#[test]
fn released_adjacent_ranges_coalesce_for_a_larger_allocation() {
    let mut backing = TestBacking::new(128);
    with_arena(&mut backing, |arena| {
        let first = arena.alloc_owned_slice::<u8>(4).unwrap();
        let middle = arena.alloc_owned_slice::<u8>(4).unwrap();
        let last = arena.alloc_owned_slice::<u8>(4).unwrap();
        drop(last);
        let before_coalesce = arena.used_bytes();
        drop(middle);
        assert!(arena.used_bytes() < before_coalesce);

        let larger = arena.alloc_owned_slice::<u8>(32).unwrap();
        assert_eq!(larger.capacity(), 32);
        drop(larger);
        drop(first);
    })
    .unwrap();
}

#[test]
fn owned_allocations_extend_in_place_and_report_fragmentation() {
    let mut backing = TestBacking::new(192);
    with_arena(&mut backing, |arena| {
        let mut tail = arena.alloc_owned_slice::<u8>(4).unwrap();
        let before = arena.used_bytes();
        assert!(arena.try_resize_owned(&mut tail, 8).unwrap());
        assert_eq!(tail.capacity(), 8);
        assert!(arena.used_bytes() >= before);

        let mut first = arena.alloc_owned_slice::<u8>(4).unwrap();
        let middle = arena.alloc_owned_slice::<u8>(4).unwrap();
        let _last = arena.alloc_owned_slice::<u8>(4).unwrap();
        drop(middle);
        let old_capacity = first.capacity();
        assert!(!arena.try_resize_owned(&mut first, 32).unwrap());
        assert_eq!(first.capacity(), old_capacity);
    })
    .unwrap();
}

#[test]
fn byte_bit_helpers_cross_byte_boundaries_and_preserve_neighbors() {
    let mut bytes = [0b1010_0101, 0b1100_0011, 0b0101_1010];
    let original = bytes;
    crate::write_bits(&mut bytes, 5, 9, 0b1_1010_1101).unwrap();
    assert_eq!(crate::read_bits(&bytes, 5, 9), Ok(0b1_1010_1101));
    for position in 0..24 {
        if !(5..14).contains(&position) {
            assert_eq!(
                bytes[position / 8] & (1 << (position % 8)),
                original[position / 8] & (1 << (position % 8))
            );
        }
    }
    assert_eq!(
        crate::write_bits(&mut bytes, 5, 3, 8),
        Err(Error::ValueDoesNotFit)
    );
    assert_eq!(crate::read_bits(&bytes, 24, 0), Ok(0));
    assert_eq!(
        crate::write_bits(&mut bytes, 24, 0, 1),
        Err(Error::ValueDoesNotFit)
    );
    assert_eq!(crate::read_bits(&bytes, 23, 2), Err(Error::InvalidBitRange));
}

#[test]
fn packed_fast_paths_match_reference_at_byte_and_word_boundaries() {
    fn reference_read(bytes: &[u8], offset: usize, width: u8) -> u64 {
        let mut value = 0;
        for index in 0..width as usize {
            if bytes[(offset + index) / 8] & (1 << ((offset + index) % 8)) != 0 {
                value |= 1_u64 << index;
            }
        }
        value
    }

    fn reference_write(bytes: &mut [u8], offset: usize, width: u8, value: u64) {
        for index in 0..width as usize {
            let position = offset + index;
            let bit = 1 << (position % 8);
            if value & (1_u64 << index) == 0 {
                bytes[position / 8] &= !bit;
            } else {
                bytes[position / 8] |= bit;
            }
        }
    }

    let starts = [0, 1, 7, 8, 15, 16, 31, 32, 63, 64, 71];
    let original = [
        0xA5_u8, 0x3C, 0xD2, 0x69, 0xF0, 0x1B, 0x87, 0x42, 0xE1, 0x55,
    ];
    for width in 0..=64 {
        for start in starts {
            if start + width as usize > original.len() * 8 {
                continue;
            }
            let expected = reference_read(&original, start, width);
            assert_eq!(crate::read_bits(&original, start, width), Ok(expected));

            let mask = if width == 64 {
                u64::MAX
            } else if width == 0 {
                0
            } else {
                (1_u64 << width) - 1
            };
            let value = 0xD6A5_39C7_81E2_4B0F & mask;
            let mut expected_bytes = original;
            reference_write(&mut expected_bytes, start, width, value);
            let mut actual = original;
            crate::write_bits(&mut actual, start, width, value).unwrap();
            assert_eq!(actual, expected_bytes, "width={width}, start={start}");
        }
    }

    let mut width64 = [0x5A_u8; 10];
    crate::write_bits(&mut width64, 0, 64, u64::MAX).unwrap();
    assert_eq!(crate::read_bits(&width64, 0, 64), Ok(u64::MAX));
    crate::write_bits(&mut width64, 1, 64, 0x0123_4567_89AB_CDEF).unwrap();
    assert_eq!(crate::read_bits(&width64, 1, 64), Ok(0x0123_4567_89AB_CDEF));
}
