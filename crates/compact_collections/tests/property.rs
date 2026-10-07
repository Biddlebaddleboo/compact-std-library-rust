use compact_backend_std::StdArena;
use compact_collections::{
    CompactBitVec, CompactHashMap, CompactHashSet, CompactPathBuf, CompactString, CompactVec,
    CompactVecDeque,
};
use proptest::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    #[test]
    fn compact_vec_and_deque_track_standard_collections(
        operations in prop::collection::vec((any::<bool>(), any::<i32>()), 0..96)
    ) {
        StdArena::with_capacity(64 * 1024, |arena| {
            let mut compact_vec = CompactVec::new_in(arena);
            let mut native_vec = std::vec::Vec::new();
            let mut compact_deque = CompactVecDeque::new_in(arena);
            let mut native_deque = VecDeque::new();

            for (push, value) in operations {
                if push {
                    compact_vec.push_in(value, arena).unwrap();
                    native_vec.push(value);
                    if value & 1 == 0 {
                        compact_deque.push_front_in(value, arena).unwrap();
                        native_deque.push_front(value);
                    } else {
                        compact_deque.push_back_in(value, arena).unwrap();
                        native_deque.push_back(value);
                    }
                } else {
                    prop_assert_eq!(compact_vec.pop_in(arena).unwrap(), native_vec.pop());
                    if value & 1 == 0 {
                        prop_assert_eq!(compact_deque.pop_front_in(arena).unwrap(), native_deque.pop_front());
                    } else {
                        prop_assert_eq!(compact_deque.pop_back_in(arena).unwrap(), native_deque.pop_back());
                    }
                }
                prop_assert_eq!(compact_vec.as_slice(arena).unwrap(), native_vec.as_slice());
                prop_assert_eq!(
                    compact_deque.iter(arena).unwrap().copied().collect::<std::vec::Vec<_>>(),
                    native_deque.iter().copied().collect::<std::vec::Vec<_>>()
                );
            }
            Ok::<_, proptest::test_runner::TestCaseError>(())
        }).unwrap().unwrap();
    }

    #[test]
    fn compact_hash_map_and_set_track_standard_collections(
        operations in prop::collection::vec((any::<bool>(), -32_i16..32, any::<i32>()), 0..96)
    ) {
        StdArena::with_capacity(64 * 1024, |arena| {
            let mut compact_map = CompactHashMap::new();
            let mut native_map = HashMap::new();
            let mut compact_set = CompactHashSet::new();
            let mut native_set = HashSet::new();

            for (insert, key, value) in operations {
                if insert {
                    prop_assert_eq!(compact_map.insert(key, value, arena).unwrap(), native_map.insert(key, value));
                    prop_assert_eq!(compact_set.insert(key, arena).unwrap(), native_set.insert(key));
                } else {
                    prop_assert_eq!(compact_map.remove(&key, arena).unwrap(), native_map.remove(&key));
                    prop_assert_eq!(compact_set.remove(&key, arena).unwrap(), native_set.remove(&key));
                }
                for probe in -32_i16..32 {
                    prop_assert_eq!(compact_map.get(&probe, arena).unwrap(), native_map.get(&probe));
                    prop_assert_eq!(compact_set.contains(&probe, arena).unwrap(), native_set.contains(&probe));
                }
            }
            Ok::<_, proptest::test_runner::TestCaseError>(())
        }).unwrap().unwrap();
    }

    #[test]
    fn compact_strings_and_paths_preserve_generated_text(
        text in any::<String>(),
        components in prop::collection::vec("[a-z]{0,8}", 0..12)
    ) {
        StdArena::with_capacity(64 * 1024, |arena| {
            let mut compact_text = CompactString::from_str_in(&text, arena).unwrap();
            let mut native_text = text;
            compact_text.push_str_in("-tail", arena).unwrap();
            native_text.push_str("-tail");
            prop_assert_eq!(compact_text.as_str(arena).unwrap(), native_text.as_str());

            let mut compact_path = CompactPathBuf::new_in(arena);
            let mut native_path = PathBuf::new();
            for component in components {
                compact_path.push(&component, arena).unwrap();
                native_path.push(component);
            }
            prop_assert_eq!(compact_path.to_path_buf(), native_path);
            Ok::<_, proptest::test_runner::TestCaseError>(())
        }).unwrap().unwrap();
    }

    #[test]
    fn compact_packed_bits_match_a_boolean_vector(bits in prop::collection::vec(any::<bool>(), 0..384)) {
        StdArena::with_capacity(16 * 1024, |arena| {
            let mut compact = CompactBitVec::new_in(arena);
            for bit in bits.iter().copied() {
                compact.push_in(bit, arena).unwrap();
            }
            prop_assert_eq!(compact.len(), bits.len());
            for (index, bit) in bits.iter().copied().enumerate() {
                prop_assert_eq!(compact.get(index, arena).unwrap(), Some(bit));
            }
            Ok::<_, proptest::test_runner::TestCaseError>(())
        }).unwrap().unwrap();
    }
}
