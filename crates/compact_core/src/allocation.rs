//! Values that may safely live in compact cage allocations.

use core::mem::MaybeUninit;

/// A value that may safely live in and move between compact cage slots.
///
/// # Safety
///
/// Implementors must be valid at their ordinary Rust alignment in cage
/// storage, and moving a value with `ptr::read`/`ptr::write` to another slot
/// must preserve its invariants. A value must not depend on its own address,
/// require pinning, or contain native pointers or references. Cage references
/// must be represented as compact offsets and resolved only while their owner
/// is borrowed. Its destructor must be safe to run while the process cage
/// remains alive.
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
