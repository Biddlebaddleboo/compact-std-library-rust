use compact_backend_std::{CageConfig, CompactRuntime};
use compact_core::CompactValue;
use std::sync::{Arc, Barrier};
use std::thread;

static DROPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static PANIC_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const BENCHMARK_POLICY_A: bool = cfg!(feature = "benchmark-allocator-a")
    && !cfg!(feature = "benchmark-allocator-b")
    && !cfg!(feature = "benchmark-allocator-c");
#[derive(Debug)]
struct DropCount(u32);
unsafe impl CompactValue for DropCount {}
impl Drop for DropCount {
    fn drop(&mut self) {
        let _ = self.0;
        DROPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

struct PanicOnce;
unsafe impl CompactValue for PanicOnce {}
impl Drop for PanicOnce {
    fn drop(&mut self) {
        if !PANIC_ONCE.swap(true, std::sync::atomic::Ordering::SeqCst) {
            panic!("intentional one-time destructor panic for allocator test");
        }
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
    let aligned_offset = aligned.offset().as_u32();
    assert!(aligned.try_resize(4).unwrap());
    assert_eq!(aligned.capacity(), 4);
    aligned.truncate(0);
    assert!(aligned.try_resize(0).unwrap());
    assert_eq!(aligned.capacity(), 0);
    assert!(aligned.try_resize(4).unwrap());
    assert_eq!(aligned.offset().as_u32(), aligned_offset);

    let mut reused_aligned = CompactRuntime::with_batched_releases(|| {
        drop(aligned);
        CompactRuntime::alloc_owned_slice::<Aligned>(4).unwrap()
    });
    if BENCHMARK_POLICY_A {
        assert_ne!(reused_aligned.offset().as_u32(), aligned_offset);
    } else {
        assert_eq!(reused_aligned.offset().as_u32(), aligned_offset);
    }
    assert_eq!(reused_aligned.len(), 0);
    reused_aligned.push(Aligned([9; 64])).unwrap();
    assert_eq!(reused_aligned.as_slice().as_ptr() as usize % 64, 0);
    assert_eq!(reused_aligned.as_slice()[0].0[0], 9);
    drop(reused_aligned);

    let mut zero_sized = CompactRuntime::alloc_owned_slice::<()>(3).unwrap();
    zero_sized.push(()).unwrap();
    zero_sized.push(()).unwrap();
    assert_eq!(zero_sized.len(), 2);
    drop(zero_sized);

    // Repeated small allocate/drop cycles reuse the same range without
    // changing the four-byte owner representation.
    let first_cycle = CompactRuntime::alloc_owned_slice::<u8>(16).unwrap();
    let cycle_offset = first_cycle.offset().as_u32();
    drop(first_cycle);
    for _ in 0..128 {
        let cycle = CompactRuntime::alloc_owned_slice::<u8>(16).unwrap();
        assert_eq!(cycle.offset().as_u32(), cycle_offset);
        drop(cycle);
    }
    assert_eq!(CompactRuntime::used_bytes().unwrap(), 0);
    CompactRuntime::validate_allocator_state().unwrap();

    for _ in 0..32 {
        thread::spawn(|| {
            let allocation = CompactRuntime::alloc_owned_slice::<u8>(16).unwrap();
            drop(allocation);
        })
        .join()
        .unwrap();
    }
    assert_eq!(CompactRuntime::used_bytes().unwrap(), 0);
    CompactRuntime::validate_allocator_state().unwrap();

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

    let before_recycle = CompactRuntime::used_bytes().unwrap();
    let mut old_header_owner = CompactRuntime::alloc_owned_slice::<u32>(6).unwrap();
    old_header_owner.push(7).unwrap();
    old_header_owner.push(9).unwrap();
    let old_header_offset = old_header_owner.offset().as_u32();
    let live_before_recycle = CompactRuntime::used_bytes().unwrap();
    #[cfg(feature = "allocator-telemetry")]
    let pending_hits_before_recycle = CompactRuntime::allocator_stats()
        .unwrap()
        .pending_reuse_hits;
    let fresh_header_owner = CompactRuntime::with_batched_releases(|| {
        CompactRuntime::with_batched_releases(|| drop(old_header_owner));
        assert_eq!(CompactRuntime::used_bytes().unwrap(), live_before_recycle);
        let fresh = CompactRuntime::alloc_owned_slice::<u64>(3).unwrap();
        if BENCHMARK_POLICY_A {
            assert_ne!(fresh.offset().as_u32(), old_header_offset);
        } else {
            assert_eq!(fresh.offset().as_u32(), old_header_offset);
        }
        assert_eq!(fresh.capacity(), 3);
        assert_eq!(fresh.len(), 0);
        if !BENCHMARK_POLICY_A {
            assert_eq!(CompactRuntime::used_bytes().unwrap(), live_before_recycle);
        }
        fresh
    });
    #[cfg(feature = "allocator-telemetry")]
    assert_eq!(
        CompactRuntime::allocator_stats()
            .unwrap()
            .pending_reuse_hits,
        pending_hits_before_recycle + (!BENCHMARK_POLICY_A) as u64
    );
    drop(fresh_header_owner);
    assert_eq!(CompactRuntime::used_bytes().unwrap(), before_recycle);
    CompactRuntime::validate_allocator_state().unwrap();

    let cross_thread_owner = CompactRuntime::alloc_owned_slice::<u64>(3).unwrap();
    let pending_offset = cross_thread_owner.offset().as_u32();
    let pending_ready = Arc::new(Barrier::new(2));
    let allocation_done = Arc::new(Barrier::new(2));
    let worker_ready = pending_ready.clone();
    let worker_done = allocation_done.clone();
    let worker = thread::spawn(move || {
        CompactRuntime::with_batched_releases(|| {
            drop(cross_thread_owner);
            worker_ready.wait();
            worker_done.wait();
        });
    });
    pending_ready.wait();
    let other_thread_owner = CompactRuntime::alloc_owned_slice::<u64>(3).unwrap();
    assert_ne!(other_thread_owner.offset().as_u32(), pending_offset);
    CompactRuntime::validate_allocator_state().unwrap();
    allocation_done.wait();
    worker.join().unwrap();
    drop(other_thread_owner);
    // Flush this thread's bounded cache so the next allocation exercises the
    // cross-thread release published by the worker.
    let _ = CompactRuntime::used_bytes().unwrap();
    let globally_released_owner = CompactRuntime::alloc_owned_slice::<u64>(3).unwrap();
    assert_eq!(globally_released_owner.offset().as_u32(), pending_offset);
    drop(globally_released_owner);
    CompactRuntime::validate_allocator_state().unwrap();

    PANIC_ONCE.store(false, std::sync::atomic::Ordering::SeqCst);
    let before_panic_batch = CompactRuntime::used_bytes().unwrap();
    let mut panic_owner = CompactRuntime::alloc_owned_slice::<PanicOnce>(1).unwrap();
    panic_owner.push(PanicOnce).unwrap();
    let panic_offset = panic_owner.offset().as_u32();
    CompactRuntime::with_batched_releases(|| {
        CompactRuntime::with_batched_releases(|| {
            let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                drop(panic_owner);
            }));
            assert!(panic_result.is_err());
        });
        let mut replacement = CompactRuntime::alloc_owned_slice::<PanicOnce>(1).unwrap();
        if BENCHMARK_POLICY_A {
            assert_ne!(replacement.offset().as_u32(), panic_offset);
        } else {
            assert_eq!(replacement.offset().as_u32(), panic_offset);
        }
        assert_eq!(replacement.len(), 0);
        replacement.push(PanicOnce).unwrap();
        drop(replacement);
    });
    assert_eq!(CompactRuntime::used_bytes().unwrap(), before_panic_batch);
    CompactRuntime::validate_allocator_state().unwrap();

    thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                CompactRuntime::with_batched_releases(|| {
                    let mut allocations = Vec::with_capacity(32);
                    for value in 0..32_u32 {
                        allocations.push(CompactRuntime::alloc_owned_value(value).unwrap());
                    }
                    drop(allocations);
                });
            });
        }
    });
    CompactRuntime::validate_allocator_state().unwrap();

    let first = CompactRuntime::alloc_owned_slice::<u64>(16).unwrap();
    let first_offset = first.offset().as_u32();
    drop(first);
    let second = CompactRuntime::alloc_owned_slice::<u64>(16).unwrap();
    assert_eq!(second.offset().as_u32(), first_offset);

    let (owner_sender, owner_receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        owner_sender
            .send(CompactRuntime::alloc_owned_slice::<u8>(16).unwrap())
            .unwrap();
    })
    .join()
    .unwrap();
    drop(owner_receiver.recv().unwrap());
    drop(second);
    assert_eq!(CompactRuntime::used_bytes().unwrap(), 0);
    assert!(matches!(
        CompactRuntime::init(CageConfig::new(1024)),
        Err(compact_core::Error::RuntimeAlreadyInitialized)
    ));
    CompactRuntime::validate_allocator_state().unwrap();
}
