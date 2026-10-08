use compact_backend_std::{CageConfig, CompactRuntime};
use compact_collections::{CompactVec, CompactVecDeque};
use std::collections::VecDeque;
use std::sync::OnceLock;

static INIT: OnceLock<()> = OnceLock::new();

fn init() {
    INIT.get_or_init(|| CompactRuntime::init(CageConfig::new(64 * 1024 * 1024)).unwrap());
}

fn assert_deques_match(compact: &CompactVecDeque<u32>, native: &VecDeque<u32>) {
    assert_eq!(compact.len(), native.len());
    assert_eq!(
        compact.iter().copied().collect::<Vec<_>>(),
        native.iter().copied().collect::<Vec<_>>()
    );
}

#[test]
fn deque_push_and_pop_fast_paths_match_vecdeque_through_wrap_and_growth() {
    init();

    let mut compact = CompactVecDeque::with_capacity(5).unwrap();
    let mut native = VecDeque::with_capacity(5);

    for value in 0..5 {
        compact.push_back(value).unwrap();
        native.push_back(value);
    }
    for _ in 0..3 {
        assert_eq!(compact.pop_front(), native.pop_front());
    }
    for value in 5..8 {
        compact.push_back(value).unwrap();
        native.push_back(value);
    }
    assert_deques_match(&compact, &native);

    compact.push_front(100).unwrap();
    native.push_front(100);
    compact.push_back(101).unwrap();
    native.push_back(101);
    assert!(compact.capacity() >= compact.len());
    assert_deques_match(&compact, &native);

    let steps = if cfg!(miri) { 128 } else { 8_192 };
    let mut state = 0x7a31_4d5bu32;
    for step in 0..steps {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        match state % 8 {
            0 | 1 => {
                compact.push_back(step).unwrap();
                native.push_back(step);
            }
            2 | 3 => {
                compact.push_front(step).unwrap();
                native.push_front(step);
            }
            4 => assert_eq!(compact.pop_front(), native.pop_front()),
            5 => assert_eq!(compact.pop_back(), native.pop_back()),
            6 => {
                if !native.is_empty() {
                    assert_eq!(compact.pop_front(), native.pop_front());
                    compact.push_back(step).unwrap();
                    native.push_back(step);
                }
            }
            _ => {
                if !native.is_empty() {
                    assert_eq!(compact.pop_back(), native.pop_back());
                    compact.push_front(step).unwrap();
                    native.push_front(step);
                }
            }
        }
        assert_deques_match(&compact, &native);
    }

    let mut zst = CompactVecDeque::new();
    zst.push_back(()).unwrap();
    zst.push_front(()).unwrap();
    assert_eq!(zst.pop_front(), Some(()));
    assert_eq!(zst.pop_back(), Some(()));
}

#[test]
fn try_clone_copy_copies_initialized_values_and_keeps_empty_capacity_semantics() {
    init();

    let empty = CompactVec::<u64>::with_capacity(8).unwrap();
    let empty_copy = empty.try_clone_copy().unwrap();
    assert!(empty_copy.is_empty());
    assert_eq!(empty_copy.capacity(), 0);
    assert_eq!(empty.capacity(), 8);

    let mut source = CompactVec::with_capacity(12).unwrap();
    source.try_extend_copy(&[3, 5, 8, 13, 21]).unwrap();
    let cloned = source.try_clone_copy().unwrap();

    assert_eq!(cloned.as_slice(), source.as_slice());
    assert_eq!(cloned.capacity(), source.len());
    assert_eq!(source.capacity(), 12);
}
