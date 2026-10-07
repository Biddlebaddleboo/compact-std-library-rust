//! Branded descriptors for initialized compact byte allocations.

use core::marker::PhantomData;

use crate::CompactValue;

/// An exact initialized byte range inside one arena.
///
/// The descriptor occupies eight bytes: a 32-bit byte offset and a 32-bit
/// byte length. Safe constructors only produce ranges backed by initialized
/// bytes. A range is a view descriptor, not an owning allocation handle.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteRange32<'arena> {
    pub(crate) offset: u32,
    pub(crate) len: u32,
    marker: PhantomData<fn(&'arena mut ()) -> &'arena mut ()>,
}

// SAFETY: ByteRange32 is a fixed offset/length descriptor with no native
// pointers and no destructor state.
unsafe impl CompactValue for ByteRange32<'_> {}

impl<'arena> ByteRange32<'arena> {
    pub(crate) const fn new(offset: u32, len: u32) -> Self {
        Self {
            offset,
            len,
            marker: PhantomData,
        }
    }

    /// Return the null, empty byte range.
    pub const fn empty() -> Self {
        Self::new(crate::NULL_OFFSET, 0)
    }

    /// Return the compact byte offset. Empty ranges use the null sentinel.
    pub const fn offset(self) -> u32 {
        self.offset
    }

    /// Return the exact number of initialized bytes in this range.
    pub const fn len(self) -> usize {
        self.len as usize
    }

    /// Return whether this range is empty.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Reconstruct a descriptor from raw parts.
    ///
    /// # Safety
    ///
    /// A non-empty range must name exactly `len` initialized bytes from a
    /// byte-valid allocation in the same arena scope. The allocation must
    /// remain byte-valid for arbitrary `u8` writes through safe mutable views;
    /// do not use this constructor for typed values, padding, or storage with
    /// live references. Empty ranges may use any offset; the canonical
    /// representation is [`empty`](Self::empty).
    pub const unsafe fn from_raw_parts_unchecked(offset: u32, len: u32) -> Self {
        Self::new(offset, len)
    }
}
