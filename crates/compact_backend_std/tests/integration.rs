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
    CompactRuntime::init(CageConfig::new(1 << 20)).expect("initialize process cage");
    assert!(CompactRuntime::is_initialized());
    assert_eq!(CompactRuntime::capacity().unwrap(), 1 << 20);
    assert!(CompactRuntime::remaining_bytes().unwrap() > 0);

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

    let first = CompactRuntime::alloc_owned_slice::<u64>(16).unwrap();
    let first_offset = first.offset().as_u32();
    drop(first);
    let second = CompactRuntime::alloc_owned_slice::<u64>(16).unwrap();
    assert_eq!(second.offset().as_u32(), first_offset);
    assert!(matches!(
        CompactRuntime::init(CageConfig::new(1024)),
        Err(compact_core::Error::RuntimeAlreadyInitialized)
    ));
}
