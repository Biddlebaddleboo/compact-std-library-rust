use compact_backend_std::{
    CompactStore, CoreError, Offset32, StableBacking, StdArena, StdBackendError, StdBacking,
    MAX_ARENA_BYTES, MIN_ARENA_BYTES,
};

#[test]
fn hosted_arena_round_trips_mutable_and_native_values() {
    let sum = StdArena::with_capacity(128, |arena| -> compact_backend_std::CoreResult<u32> {
        let number: Offset32<'_, u32> = arena.alloc_value(40_u32)?;
        *arena.get_mut(number)? += 2;
        Ok(*arena.get(number)?)
    })
    .unwrap()
    .unwrap();
    assert_eq!(sum, 42);
}

#[test]
fn fixed_backing_can_be_reused_as_separate_scoped_arenas() {
    let mut backing = StdBacking::with_capacity(MIN_ARENA_BYTES).unwrap();
    assert_eq!(backing.capacity(), MIN_ARENA_BYTES);
    let first = backing
        .with_arena(|arena| {
            let offset = arena.alloc_value(7_u8).unwrap();
            *arena.get(offset).unwrap()
        })
        .unwrap();
    let second = backing
        .with_arena(|arena| {
            let offset = arena.alloc_value(9_u8).unwrap();
            *arena.get(offset).unwrap()
        })
        .unwrap();
    assert_eq!((first, second), (7, 9));
}

#[test]
fn moving_the_owner_does_not_move_the_backing_allocation() {
    let mut original = StdBacking::with_capacity(MIN_ARENA_BYTES).unwrap();
    let original_address = original.bytes_mut().as_mut_ptr() as usize;
    let mut moved = original;
    let moved_address = moved.bytes_mut().as_mut_ptr() as usize;
    assert_eq!(original_address, moved_address);
}

#[test]
fn capacity_errors_are_reported_without_large_allocations() {
    assert!(matches!(
        StdBacking::with_capacity(0),
        Err(StdBackendError::Core(CoreError::InvalidCapacity))
    ));
    assert!(matches!(
        StdBacking::with_capacity(1),
        Err(StdBackendError::Core(CoreError::InvalidCapacity))
    ));
    assert!(matches!(
        StdBacking::with_capacity(MIN_ARENA_BYTES - 1),
        Err(StdBackendError::Core(CoreError::InvalidCapacity))
    ));
    let smallest = StdArena::with_capacity(MIN_ARENA_BYTES, |arena| {
        let offset = arena.alloc_value(3_u8).unwrap();
        *arena.get(offset).unwrap()
    })
    .unwrap();
    assert_eq!(smallest, 3);
    #[cfg(target_pointer_width = "64")]
    assert!(matches!(
        StdBacking::with_capacity(MAX_ARENA_BYTES as usize + 1),
        Err(StdBackendError::Core(CoreError::BackingTooLarge))
    ));
}

#[test]
fn allocation_exhaustion_is_deterministic() {
    let result = StdArena::with_capacity(MIN_ARENA_BYTES, |arena| {
        arena.alloc_value(1_u8).unwrap();
        arena.alloc_slice(&[0_u8; 4]).unwrap_err()
    })
    .unwrap();
    assert_eq!(result, CoreError::AllocationExhausted);
}

#[test]
fn v1_limit_is_four_gibibytes() {
    assert_eq!(MAX_ARENA_BYTES, 1_u64 << 32);
}

#[test]
fn compact_store_rebrands_roots_across_repeated_access_and_moves() {
    let store = CompactStore::<u32>::build(512, |arena| arena.alloc_value(41_u32)).unwrap();
    let mut store = store;
    assert_eq!(
        store.with(|arena, root| *root.get(arena).unwrap()).unwrap(),
        41
    );

    store
        .with_mut(|arena, root| {
            *root.get_mut(arena).unwrap() += 1;
        })
        .unwrap();
    let used_before = store.with(|arena, _| arena.used_bytes()).unwrap();
    store
        .with_mut(|arena, root| {
            arena.alloc_value(0xfeed_u64).unwrap();
            *root.get_mut(arena).unwrap() += 1;
        })
        .unwrap();
    let used_after = store
        .with(|arena, root| {
            assert_eq!(*root.get(arena).unwrap(), 43);
            arena.used_bytes()
        })
        .unwrap();
    assert!(used_after > used_before);
}
