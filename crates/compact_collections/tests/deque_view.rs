use compact_backend_std::{CageConfig, CompactRuntime};
use compact_collections::{CollectionError, CompactVecDeque};
use compact_core::CompactValue;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
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
fn view_matches_vecdeque_through_wrap_and_mixed_ops() {
    init();

    let mut compact = CompactVecDeque::with_capacity(4).unwrap();
    let mut native: VecDeque<u32> = VecDeque::with_capacity(4);
    for value in 0..4 {
        compact.push_back(value).unwrap();
        native.push_back(value);
    }
    let steps = if cfg!(miri) { 256 } else { 20_000 };
    // Reserve enough headroom that the random walk never needs the view to grow.
    compact.reserve(steps + 8).unwrap();
    let mut state = 0x9e37_79b9u32;
    compact
        .with_view(|ring| -> Result<(), CollectionError> {
            for step in 0..steps {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let value = state;
                match state % 8 {
                    0 | 1 => {
                        ring.push_back(value)?;
                        native.push_back(value);
                    }
                    2 | 3 => {
                        ring.push_front(value)?;
                        native.push_front(value);
                    }
                    4 => assert_eq!(ring.pop_front(), native.pop_front()),
                    5 => assert_eq!(ring.pop_back(), native.pop_back()),
                    6 => {
                        if !native.is_empty() {
                            assert_eq!(ring.pop_front(), native.pop_front());
                            ring.push_back(value)?;
                            native.push_back(value);
                        }
                    }
                    _ => {
                        if !native.is_empty() {
                            assert_eq!(ring.pop_back(), native.pop_back());
                            ring.push_front(value)?;
                            native.push_front(value);
                        }
                    }
                }
                assert_eq!(ring.len(), native.len());
                assert_eq!(ring.front().copied(), native.front().copied());
                assert_eq!(ring.back().copied(), native.back().copied());
                if !native.is_empty() {
                    let index = step % native.len();
                    assert_eq!(ring.get(index).copied(), native.get(index).copied());
                }
            }
            Ok(())
        })
        .unwrap();

    assert_deques_match(&compact, &native);
}

#[test]
fn view_reports_full_state_and_leaves_valid_post_error_state() {
    init();

    let mut compact = CompactVecDeque::with_capacity(3).unwrap();
    for value in 0..3 {
        compact.push_back(value).unwrap();
    }

    compact.with_view(|ring| {
        assert_eq!(ring.len(), 3);
        assert!(matches!(ring.push_back(99), Err(CollectionError::Core(_))));
        assert!(matches!(ring.push_front(99), Err(CollectionError::Core(_))));
        // Failed pushes leave the ring unchanged and valid.
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.front().copied(), Some(0));
        assert_eq!(ring.back().copied(), Some(2));
        assert_eq!(ring.pop_front(), Some(0));
        // Now there is room again; the view can still push.
        assert!(ring.push_back(7).is_ok());
    });

    let seen: Vec<u32> = compact.iter().copied().collect();
    assert_eq!(seen, vec![1, 2, 7]);
}

#[test]
fn empty_view_has_no_storage_and_pushes_fail() {
    init();

    let mut compact = CompactVecDeque::<u32>::new();
    compact.with_view(|ring| {
        assert_eq!(ring.capacity(), 0);
        assert!(ring.is_empty());
        assert_eq!(ring.pop_front(), None);
        assert_eq!(ring.pop_back(), None);
        assert!(ring.push_back(1).is_err());
    });
    assert!(compact.is_empty());
}

#[test]
fn view_writes_back_metadata_when_the_closure_unwinds() {
    init();

    let mut compact = CompactVecDeque::with_capacity(4).unwrap();
    for value in 0..3 {
        compact.push_back(value).unwrap();
    }
    compact.reserve(4).unwrap();

    let result = catch_unwind(AssertUnwindSafe(|| {
        compact.with_view(|ring| {
            ring.push_back(10).unwrap();
            assert_eq!(ring.pop_front(), Some(0));
            panic!("intentional unwind inside view");
        });
    }));
    assert!(result.is_err());

    // One push and one pop completed before the unwind; the deque must reflect
    // both and stay internally consistent.
    let seen: Vec<u32> = compact.iter().copied().collect();
    assert_eq!(seen, vec![1, 2, 10]);
    assert_eq!(compact.pop_front(), Some(1));
}

#[test]
fn view_drops_each_value_exactly_once() {
    init();

    static DROPS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    struct Droppy(u32);

    impl Drop for Droppy {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    // SAFETY: `Droppy` is a plain `u32` wrapper moved by value into the cage.
    unsafe impl CompactValue for Droppy {}

    DROPS.store(0, Ordering::SeqCst);
    {
        let mut queue = CompactVecDeque::with_capacity(2).unwrap();
        queue.push_back(Droppy(0)).unwrap();
        queue.push_back(Droppy(1)).unwrap();
        queue.reserve(8).unwrap();
        queue.with_view(|ring| {
            ring.push_back(Droppy(2)).unwrap();
            let popped = ring.pop_front().expect("populated");
            assert_eq!(popped.0, 0);
            drop(popped);
        });
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(queue.len(), 2);
    }
    assert_eq!(DROPS.load(Ordering::SeqCst), 3);
}

#[test]
fn view_layout_does_not_change_the_deque_owner() {
    init();
    // The view is borrow-scoped and never stored, so the deque keeps its 12-byte
    // frozen layout.
    assert_eq!(core::mem::size_of::<CompactVecDeque<u64>>(), 12);
}

#[test]
fn view_handles_zero_sized_values() {
    init();

    let mut compact = CompactVecDeque::<()>::with_capacity(4).unwrap();
    compact.with_view(|ring| {
        assert_eq!(ring.capacity(), 4);
        for _ in 0..4 {
            ring.push_back(()).unwrap();
        }
        assert_eq!(ring.len(), 4);
        assert!(ring.push_back(()).is_err());
        assert_eq!(ring.front(), Some(&()));
        assert_eq!(ring.back(), Some(&()));
        assert_eq!(ring.pop_front(), Some(()));
        assert!(ring.push_back(()).is_ok());
    });
    assert_eq!(compact.len(), 4);
}

#[test]
fn view_writes_back_metadata_when_a_value_destructor_panics() {
    init();

    struct PanicOnDrop(bool);
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            if self.0 {
                panic!("value destructor panic");
            }
        }
    }
    // SAFETY: `PanicOnDrop` is a plain bool wrapper moved by value into the cage.
    unsafe impl CompactValue for PanicOnDrop {}

    let mut compact = CompactVecDeque::<PanicOnDrop>::with_capacity(4).unwrap();
    compact.push_back(PanicOnDrop(true)).unwrap();
    compact.push_back(PanicOnDrop(false)).unwrap();
    compact.push_back(PanicOnDrop(false)).unwrap();

    let result = catch_unwind(AssertUnwindSafe(|| {
        compact.with_view(|ring| {
            let doomed = ring.pop_front().expect("populated");
            drop(doomed);
            // Unreachable if the destructor panics as expected.
            ring.push_back(PanicOnDrop(false)).unwrap();
        });
    }));
    assert!(result.is_err());

    // The pop completed before the destructor panicked; the deque records it and
    // remains valid for further use.
    assert_eq!(compact.len(), 2);
    assert_eq!(compact.pop_back().map(|_| ()), Some(()));
}
