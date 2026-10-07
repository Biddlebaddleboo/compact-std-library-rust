use core::mem::{align_of, MaybeUninit};

use crate::{
    bits_required, checked_align_up, smallest_word, with_arena, BitField, Error, Offset32,
    PackedWord, StableBacking, StorageWord,
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
fn values_round_trip_mutate_and_align() {
    #[repr(align(32))]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Aligned(u8);

    let mut backing = TestBacking::new(96);
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
    let mut backing = TestBacking::new(32);
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

    let mut backing = TestBacking::new(32);
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
    let mut backing = TestBacking::new(64);
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
    let mut backing = TestBacking::new(48);
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
    let mut backing = TestBacking::new(5);
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

    let mut backing = TestBacking::new(8);
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
fn smallest_usable_backing_fits_one_byte() {
    let mut backing = TestBacking::new(crate::MIN_ARENA_BYTES);
    with_arena(&mut backing, |arena| {
        let only_value = arena.alloc_value(5_u8).unwrap();
        assert_eq!(*arena.get(only_value).unwrap(), 5);
        assert_eq!(arena.remaining_bytes(), 0);
    })
    .unwrap();
}

#[test]
fn independent_arenas_have_independent_memory() {
    let mut first = TestBacking::new(8);
    let mut second = TestBacking::new(8);
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
