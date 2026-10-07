//! Lifetime-free compact offset descriptors.

use core::marker::PhantomData;

use crate::CompactValue;

/// A typed, non-owning 32-bit byte offset into the process cage.
///
/// Offset zero is reserved as null. This descriptor does not keep an
/// allocation alive; use an owning compact collection or graph to retain its
/// target.
#[repr(transparent)]
pub struct Offset32<T> {
    pub(crate) raw: u32,
    marker: PhantomData<fn() -> T>,
}

unsafe impl<T> CompactValue for Offset32<T> {}
impl<T> Copy for Offset32<T> {}
impl<T> Clone for Offset32<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> core::fmt::Debug for Offset32<T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_tuple("Offset32").field(&self.raw).finish()
    }
}
impl<T> PartialEq for Offset32<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}
impl<T> Eq for Offset32<T> {}

impl<T> Offset32<T> {
    pub(crate) const fn new(raw: u32) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }
    /// Construct the null offset.
    pub const fn null() -> Self {
        Self::new(crate::NULL_OFFSET)
    }
    /// Return whether this offset is null.
    pub const fn is_null(self) -> bool {
        self.raw == crate::NULL_OFFSET
    }
    /// Return the raw byte offset for diagnostics or compact storage.
    pub const fn as_u32(self) -> u32 {
        self.raw
    }
    /// Reconstruct a typed offset from a raw cage offset.
    ///
    /// # Safety
    ///
    /// A non-null value must identify a live, initialized allocation of `T`
    /// that remains owned for every access through this descriptor.
    pub const unsafe fn from_raw_unchecked(raw: u32) -> Self {
        Self::new(raw)
    }
}

/// A compact offset and element count for a contiguous typed allocation.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OffsetSlice32<T> {
    pub(crate) offset: Offset32<T>,
    pub(crate) len: u32,
}

unsafe impl<T> CompactValue for OffsetSlice32<T> {}

impl<T> OffsetSlice32<T> {
    pub(crate) const fn new(offset: Offset32<T>, len: u32) -> Self {
        Self { offset, len }
    }
    /// Return the first element offset.
    pub const fn offset(self) -> Offset32<T> {
        self.offset
    }
    /// Return the element count.
    pub const fn len(self) -> usize {
        self.len as usize
    }
    /// Return whether the slice is empty.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
    /// Construct the canonical empty descriptor.
    pub const fn empty() -> Self {
        Self::new(Offset32::null(), 0)
    }
    /// Reconstruct an offset slice from raw parts.
    ///
    /// # Safety
    ///
    /// The raw parts must describe `len` initialized values in a live cage
    /// allocation that outlives all uses of the descriptor.
    pub const unsafe fn from_raw_parts_unchecked(offset: u32, len: u32) -> Self {
        Self::new(unsafe { Offset32::from_raw_unchecked(offset) }, len)
    }
}
