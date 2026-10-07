use compact_backend_std::{CageConfig, CompactRuntime};
use compact_core::CompactValue;
use std::sync::{Arc, Barrier};
use std::thread;

static DROPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[derive(Debug)]
struct DropCount(u32);
unsafe impl CompactValue for DropCount {}
impl Drop for DropCount {
    fn drop(&mut self) {
        let _ = self.0;
        DROPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn process_cage_owners_layout_drop_and_threaded_release() {
    let outcomes = thread::scope(|scope| {
        (0..4)
            .map(|_| scope.spawn(|| CompactRuntime::init(CageConfig::new(1 << 20))))
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(outcomes.iter().all(|result| {
        result.is_ok() || *result == Err(compact_core::Error::RuntimeAlreadyInitialized)
    }));
    assert!(CompactRuntime::is_initialized());
    assert_eq!(CompactRuntime::capacity().unwrap(), 1 << 20);
    assert!(CompactRuntime::remaining_bytes().unwrap() > 0);

    #[repr(align(64))]
    struct Aligned([u8; 64]);
    unsafe impl CompactValue for Aligned {}
    let mut aligned = CompactRuntime::alloc_owned_slice::<Aligned>(1).unwrap();
    aligned.push(Aligned([7; 64])).unwrap();
    assert_eq!(aligned.as_slice().as_ptr() as usize % 64, 0);
    assert_eq!(aligned.as_slice()[0].0[0], 7);
    drop(aligned);

    let mut zero_sized = CompactRuntime::alloc_owned_slice::<()>(3).unwrap();
    zero_sized.push(()).unwrap();
    zero_sized.push(()).unwrap();
    assert_eq!(zero_sized.len(), 2);
    drop(zero_sized);

    let mut tail = CompactRuntime::alloc_owned_slice::<u32>(2).unwrap();
    tail.push(11).unwrap();
    let tail_offset = tail.offset().as_u32();
    assert!(tail.try_resize(8).unwrap());
    assert!(tail.try_resize(1).unwrap());
    assert_eq!(tail.offset().as_u32(), tail_offset);
    assert_eq!(tail.as_slice(), &[11]);
    let previous_capacity = tail.capacity();
    assert!(!tail.try_resize(2 * 1024 * 1024).unwrap());
    assert_eq!(tail.capacity(), previous_capacity);
    assert_eq!(tail.as_slice(), &[11]);
    drop(tail);

    let mut left = CompactRuntime::alloc_owned_slice::<u32>(2).unwrap();
    left.push(19).unwrap();
    let left_offset = left.offset().as_u32();
    let right = CompactRuntime::alloc_owned_slice::<u32>(4).unwrap();
    let keeper = CompactRuntime::alloc_owned_slice::<u32>(1).unwrap();
    drop(right);
    assert!(left.try_resize(8).unwrap());
    assert_eq!(left.offset().as_u32(), left_offset);
    assert_eq!(left.as_slice(), &[19]);
    drop(keeper);
    drop(left);

    let overflow = CompactRuntime::alloc_owned_slice::<u8>(u32::MAX as usize + 1);
    assert!(matches!(overflow, Err(compact_core::Error::OffsetOverflow)));
    let exhaustion = CompactRuntime::alloc_owned_slice::<u8>(2 * 1024 * 1024);
    assert!(matches!(
        exhaustion,
        Err(compact_core::Error::AllocationExhausted)
    ));

    DROPS.store(0, std::sync::atomic::Ordering::SeqCst);
    let mut owner = CompactRuntime::alloc_owned_slice::<DropCount>(2).unwrap();
    owner.push(DropCount(1)).unwrap();
    owner.push(DropCount(2)).unwrap();
    assert_eq!(std::mem::size_of_val(&owner), 4);
    assert_eq!(owner.len(), 2);

    let barrier = Arc::new(Barrier::new(2));
    let thread_barrier = barrier.clone();
    let thread = thread::spawn(move || {
        assert_eq!(owner.len(), 2);
        thread_barrier.wait();
        drop(owner);
    });
    barrier.wait();
    thread.join().unwrap();
    assert_eq!(DROPS.load(std::sync::atomic::Ordering::SeqCst), 2);
    CompactRuntime::validate_allocator_state().unwrap();

    let first = CompactRuntime::alloc_owned_slice::<u64>(16).unwrap();
    let first_offset = first.offset().as_u32();
    drop(first);
    let second = CompactRuntime::alloc_owned_slice::<u64>(16).unwrap();
    assert_eq!(second.offset().as_u32(), first_offset);
    assert!(matches!(
        CompactRuntime::init(CageConfig::new(1024)),
        Err(compact_core::Error::RuntimeAlreadyInitialized)
    ));
    CompactRuntime::validate_allocator_state().unwrap();
}
