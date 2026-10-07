//! Compact arena-relative references.

use core::marker::PhantomData;

/// A typed arena-relative byte offset with a four-byte representation.
///
/// The arena lifetime is a generative scope supplied by [`with_arena`](crate::with_arena).
/// It prevents safe code from resolving a reference through a different arena.
/// The marker carries no bytes; `T` must be sized and arena allocations currently
/// require `Copy` values so the arena can be discarded without running drops.
/// Use [`null`](Self::null) for the V1 null sentinel; no `Option<Offset32<T>>`
/// size or niche optimization is part of the ABI guarantee.
#[repr(transparent)]
pub struct Offset32<'arena, T> {
    pub(crate) raw: u32,
    marker: PhantomData<fn(&'arena mut ()) -> &'arena mut ()>,
    type_marker: PhantomData<fn(T) -> T>,
}

impl<T> Copy for Offset32<'_, T> {}

impl<T> Clone for Offset32<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> core::fmt::Debug for Offset32<'_, T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_tuple("Offset32").field(&self.raw).finish()
    }
}

impl<'arena, T> Offset32<'arena, T> {
    pub(crate) const fn new(raw: u32) -> Self {
        Self {
            raw,
            marker: PhantomData,
            type_marker: PhantomData,
        }
    }

    /// Construct the null offset. It cannot be resolved to a value.
    pub const fn null() -> Self {
        Self::new(crate::NULL_OFFSET)
    }

    /// Return whether this is the reserved null offset.
    pub const fn is_null(self) -> bool {
        self.raw == crate::NULL_OFFSET
    }

    /// Return the byte offset payload.
    ///
    /// This value is useful for diagnostics and compact storage. Reconstructing
    /// an offset from it requires [`from_raw_unchecked`](Self::from_raw_unchecked).
    pub const fn as_u32(self) -> u32 {
        self.raw
    }

    /// Reconstruct an offset from a raw byte offset.
    ///
    /// # Safety
    ///
    /// Unless `raw` is [`NULL_OFFSET`](crate::NULL_OFFSET), it must be the
    /// start of a live allocation of `T` created by the same arena scope, with
    /// `T` initialized and still within that arena's allocated prefix. The
    /// caller must not use this to bypass the arena lifetime brand.
    pub const unsafe fn from_raw_unchecked(raw: u32) -> Self {
        Self::new(raw)
    }
}

/// A compact offset plus the element count for a contiguous typed allocation.
///
/// The offset itself remains four bytes; this view descriptor occupies eight
/// bytes and keeps the length tied to the allocation that created it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OffsetSlice32<'arena, T> {
    pub(crate) offset: Offset32<'arena, T>,
    pub(crate) len: u32,
}

impl<'arena, T> OffsetSlice32<'arena, T> {
    pub(crate) const fn new(offset: Offset32<'arena, T>, len: u32) -> Self {
        Self { offset, len }
    }

    /// Return the first element's compact offset.
    pub const fn offset(self) -> Offset32<'arena, T> {
        self.offset
    }

    /// Return the element count.
    pub const fn len(self) -> usize {
        self.len as usize
    }

    /// Return whether the allocation contains no elements.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Construct the canonical empty descriptor. It cannot be resolved as an
    /// allocation, but is useful for containers with zero capacity.
    pub const fn empty() -> Self {
        Self::new(Offset32::null(), 0)
    }

    /// Reconstruct a slice descriptor from raw parts.
    ///
    /// # Safety
    ///
    /// The non-null offset and `len` must describe a live initialized slice of
    /// `T` created in the same arena scope. Its total byte length must be valid
    /// for native slice references. Null is only allowed when `len == 0`, but
    /// such a descriptor cannot be resolved.
    pub const unsafe fn from_raw_parts_unchecked(offset: u32, len: u32) -> Self {
        // SAFETY: this method carries the same allocation-provenance contract
        // as the returned slice descriptor; callers uphold it when resolving.
        Self::new(unsafe { Offset32::from_raw_unchecked(offset) }, len)
    }
}
