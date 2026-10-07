//! Compact byte-range descriptors.

use crate::CompactValue;

/// An exact byte range in a live process-cage allocation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteRange32 {
    pub(crate) offset: u32,
    pub(crate) len: u32,
}

unsafe impl CompactValue for ByteRange32 {}

impl ByteRange32 {
    pub(crate) const fn new(offset: u32, len: u32) -> Self {
        Self { offset, len }
    }
    /// Return the canonical empty range.
    pub const fn empty() -> Self {
        Self::new(crate::NULL_OFFSET, 0)
    }
    /// Return the byte offset.
    pub const fn offset(self) -> u32 {
        self.offset
    }
    /// Return the exact byte length.
    pub const fn len(self) -> usize {
        self.len as usize
    }
    /// Return whether the range is empty.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
    /// Reconstruct a byte range from raw parts.
    ///
    /// # Safety
    ///
    /// A non-empty range must name initialized bytes in a live allocation.
    pub const unsafe fn from_raw_parts_unchecked(offset: u32, len: u32) -> Self {
        Self::new(offset, len)
    }
}
