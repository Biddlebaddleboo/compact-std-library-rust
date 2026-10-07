use compact_backend_std::StdArena;
use compact_collections::{
    CloneIn, CompactBytes, CompactHashMap, CompactHashSet, CompactSmallVec, CompactString,
    CompactVec, CompactVecDeque, ExtendIn, FromIteratorIn, ToCompactStringIn,
};

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
