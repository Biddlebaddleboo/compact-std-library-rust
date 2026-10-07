use compact_backend_std::StdArena;
use compact_collections::{
    CollectionError, CompactBitVec, CompactBox, CompactInterner, CompactOption, CompactSlab,
    CompactSmallVec, CompactString, CompactVec, InternId,
};
use compact_core::Offset32;

#[test]
fn compact_metadata_sizes_match_the_v2_targets() {
    assert_eq!(core::mem::size_of::<CompactBox<'static, u32>>(), 4);
    assert_eq!(core::mem::size_of::<CompactOption<'static, u32>>(), 4);
    assert_eq!(core::mem::size_of::<CompactVec<'static, u32>>(), 12);
    assert_eq!(core::mem::size_of::<CompactString<'static>>(), 16);
    assert_eq!(core::mem::size_of::<CompactSlab<'static, u32>>(), 16);
    assert_eq!(
        core::mem::size_of::<compact_core::ByteRange32<'static>>(),
        8
    );
    assert_eq!(core::mem::size_of::<CompactSmallVec<'static, u32, 2>>(), 16);
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
        let boxed = CompactBox::new_in(41_u32, arena).unwrap();
        *boxed.get_mut(arena).unwrap() += 1;
        assert_eq!(*boxed.get(arena).unwrap(), 42);

        let some = CompactOption::some(boxed.offset());
        let none = CompactOption::<u32>::none();
        assert_eq!(some.get(arena).unwrap(), Some(&42));
        assert_eq!(none.get(arena).unwrap(), None);
    })
    .unwrap();
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

    StdArena::with_capacity(3, |arena| {
        let mut values = CompactSmallVec::<u8, 2>::new_in(arena);
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

    StdArena::with_capacity(5, |arena| {
        let mut values = CompactVec::with_capacity_in(2, arena).unwrap();
        values.push_in(10_u8, arena).unwrap();
        values.push_in(20, arena).unwrap();
        assert!(values.push_in(30, arena).is_err());
        assert_eq!(values.as_slice(arena).unwrap(), &[10, 20]);
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
}

#[test]
fn interner_deduplicates_canonical_bytes_and_strings() {
    StdArena::with_capacity(512, |arena| {
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
    })
    .unwrap();
}
