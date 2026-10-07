use compact_backend_std::{
    CoreError, Offset32, StableBacking, StdArena, StdBackendError, StdBacking, MAX_ARENA_BYTES,
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
    let mut backing = StdBacking::with_capacity(32).unwrap();
    assert_eq!(backing.capacity(), 32);
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
    let mut original = StdBacking::with_capacity(32).unwrap();
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
    let smallest = StdArena::with_capacity(2, |arena| {
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
    let result = StdArena::with_capacity(4, |arena| {
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
