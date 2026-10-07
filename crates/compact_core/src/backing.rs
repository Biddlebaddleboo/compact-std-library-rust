//! Backend contract for stable contiguous memory.

use core::mem::MaybeUninit;

/// A fixed contiguous memory region whose address remains stable while mutably
/// borrowed through this trait.
///
/// Implementations must return the full usable region, keep it alive and at
/// the same address for the returned borrow, and must not expose another safe
/// operation that relocates it while that borrow exists. The byte slice itself
/// is only required to have byte alignment; the arena accounts for any extra
/// padding needed by typed allocations.
///
/// # Safety
///
/// The returned slice must describe valid writable memory for its full length.
/// It must remain allocated and at a stable address until the returned borrow
/// ends. Implementors must not create aliases that permit concurrent access to
/// the bytes while the mutable slice is live.
pub unsafe trait StableBacking {
    /// Borrow the fixed backing region as uninitialized writable bytes.
    fn bytes_mut(&mut self) -> &mut [MaybeUninit<u8>];
}
