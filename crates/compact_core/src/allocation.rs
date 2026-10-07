//! Values that may safely live in compact cage allocations.

use core::mem::MaybeUninit;

/// A value that may safely live in and move between compact cage slots.
///
/// # Safety
///
/// Implementors must satisfy all of the following:
///
/// - contain no retained native pointers or references, including pointers
///   disguised as integers when correctness depends on their address;
/// - contain no self-reference, address-sensitive state, or pinning requirement;
/// - remain valid when transferred with `ptr::read`/`ptr::write` to another
///   correctly aligned cage slot;
/// - use only cage-safe representations for nested ownership and links;
/// - remain safe to destroy while the process cage is alive.
///
/// Cage-relative offsets are resolved to native pointers only as temporary
/// views whose lifetimes are tied to a safe owner's borrow. Implementations do
/// not need to prove kernel-owned pointer arithmetic or initialization facts in
/// safe Rust; those are upheld by the unsafe cage kernel. Manual unsafe impls
/// remain the implementor's responsibility.
pub unsafe trait CompactValue {}

macro_rules! compact_values {
    ($($ty:ty),* $(,)?) => { $(unsafe impl CompactValue for $ty {})* };
}

compact_values!(
    (),
    bool,
    char,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    f32,
    f64
);

unsafe impl<T: CompactValue, const N: usize> CompactValue for [T; N] {}
unsafe impl<T: CompactValue> CompactValue for MaybeUninit<T> {}
unsafe impl<T: CompactValue> CompactValue for Option<T> {}
unsafe impl<T: CompactValue, E: CompactValue> CompactValue for core::result::Result<T, E> {}
unsafe impl<A: CompactValue, B: CompactValue> CompactValue for (A, B) {}
unsafe impl<A: CompactValue, B: CompactValue, C: CompactValue> CompactValue for (A, B, C) {}
unsafe impl<A: CompactValue, B: CompactValue, C: CompactValue, D: CompactValue> CompactValue
    for (A, B, C, D)
{
}
