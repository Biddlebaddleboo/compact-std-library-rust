//! Compact arena-backed storage for arbitrary byte payloads.

use compact_core::{Arena, ArenaAllocation, CompactValue};
use core::borrow::{Borrow, BorrowMut};
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::{Deref, DerefMut, Index, IndexMut};

use crate::{CollectionError, Result};

/// Inline byte count selected from local layout and payload benchmarks.
pub const COMPACT_BYTES_INLINE_CAPACITY: usize = 20;

enum BytesRepr<'arena, const INLINE: usize> {
    Inline { len: u8, bytes: [u8; INLINE] },
    Heap(ArenaAllocation<'arena, u8>),
}

/// A compact byte buffer with twenty inline bytes and reclaimable arena
/// storage for longer payloads.
///
/// Methods that may grow the buffer take an explicit arena. Borrowed slices
/// are available directly from the unique owner and remain bounded by `self`.
pub struct CompactBytes<'arena> {
    repr: BytesRepr<'arena, COMPACT_BYTES_INLINE_CAPACITY>,
}

impl<'arena> CompactBytes<'arena> {
    /// Construct an empty inline byte buffer.
    pub const fn new() -> Self {
        Self {
            repr: BytesRepr::Inline {
                len: 0,
                bytes: [0; COMPACT_BYTES_INLINE_CAPACITY],
            },
        }
    }

    /// Construct an empty inline byte buffer tied to `arena`.
    pub fn new_in(_arena: &Arena<'arena, '_>) -> Self {
        Self::new()
    }

    /// Construct an empty buffer with room for at least `capacity` bytes.
    pub fn with_capacity_in(capacity: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        u32::try_from(capacity).map_err(|_| CollectionError::CapacityOverflow)?;
        if capacity <= COMPACT_BYTES_INLINE_CAPACITY {
            return Ok(Self::new());
        }
        let storage = arena.alloc_owned_slice::<u8>(capacity)?;
        Ok(Self {
            repr: BytesRepr::Heap(storage),
        })
    }

    /// Construct an empty buffer with room for at least `capacity` bytes.
    pub fn with_capacity(capacity: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        Self::with_capacity_in(capacity, arena)
    }

    /// Copy a byte slice into compact storage.
    pub fn from_slice_in(bytes: &[u8], arena: &mut Arena<'arena, '_>) -> Result<Self> {
        u32::try_from(bytes.len()).map_err(|_| CollectionError::CapacityOverflow)?;
        if bytes.len() <= COMPACT_BYTES_INLINE_CAPACITY {
            let mut inline = [0; COMPACT_BYTES_INLINE_CAPACITY];
            inline[..bytes.len()].copy_from_slice(bytes);
            return Ok(Self {
                repr: BytesRepr::Inline {
                    len: bytes.len() as u8,
                    bytes: inline,
                },
            });
        }
        let mut storage = arena.alloc_owned_slice::<u8>(bytes.len())?;
        storage.extend_copy(bytes)?;
        Ok(Self {
            repr: BytesRepr::Heap(storage),
        })
    }

    /// Copy a byte slice into compact storage.
    pub fn from_slice(bytes: &[u8], arena: &mut Arena<'arena, '_>) -> Result<Self> {
        Self::from_slice_in(bytes, arena)
    }

    /// Return the initialized byte count.
    pub fn len(&self) -> usize {
        match &self.repr {
            BytesRepr::Inline { len, .. } => *len as usize,
            BytesRepr::Heap(storage) => storage.len(),
        }
    }

    /// Return the current byte capacity.
    pub fn capacity(&self) -> usize {
        match &self.repr {
            BytesRepr::Inline { .. } => COMPACT_BYTES_INLINE_CAPACITY,
            BytesRepr::Heap(storage) => storage.capacity(),
        }
    }

    /// Return whether the buffer contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrow the initialized bytes.
    pub fn as_slice(&self) -> &[u8] {
        match &self.repr {
            BytesRepr::Inline { len, bytes } => &bytes[..*len as usize],
            BytesRepr::Heap(storage) => storage.as_slice(),
        }
    }

    /// Return a pointer to the initialized bytes.
    ///
    /// The pointer is valid only while this buffer remains alive and is not
    /// mutated or reallocated. Prefer [`with_ffi_bytes`](Self::with_ffi_bytes)
    /// for a synchronous native call.
    pub fn as_ptr(&self) -> *const u8 {
        self.as_slice().as_ptr()
    }

    /// Expose the byte slice for the duration of a synchronous native call.
    ///
    /// A native callee must not retain the pointer after `call` returns. Use
    /// `compact_std::FfiByteBuffer` when the bytes must outlive this borrow.
    pub fn with_ffi_bytes<R>(&self, call: impl FnOnce(&[u8]) -> R) -> R {
        call(self.as_slice())
    }

    /// Mutably borrow the initialized bytes.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        match &mut self.repr {
            BytesRepr::Inline { len, bytes } => &mut bytes[..*len as usize],
            BytesRepr::Heap(storage) => storage.as_mut_slice(),
        }
    }

    /// Append one byte, growing through `arena` when required.
    pub fn push_in(&mut self, byte: u8, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.reserve_in(1, arena)?;
        match &mut self.repr {
            BytesRepr::Inline { len, bytes } => {
                bytes[*len as usize] = byte;
                *len += 1;
            }
            BytesRepr::Heap(storage) => storage.push(byte)?,
        }
        Ok(())
    }

    /// Append one byte, growing through `arena` when required.
    pub fn push(&mut self, byte: u8, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.push_in(byte, arena)
    }

    /// Append a slice of bytes, growing through `arena` when required.
    pub fn extend_from_slice_in(
        &mut self,
        bytes: &[u8],
        arena: &mut Arena<'arena, '_>,
    ) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.reserve_in(bytes.len(), arena)?;
        match &mut self.repr {
            BytesRepr::Inline { len, bytes: inline } => {
                let start = *len as usize;
                let end = start + bytes.len();
                inline[start..end].copy_from_slice(bytes);
                *len = end as u8;
            }
            BytesRepr::Heap(storage) => storage.extend_copy(bytes)?,
        }
        Ok(())
    }

    /// Append a byte slice, growing through `arena` when required.
    pub fn extend_from_slice(&mut self, bytes: &[u8], arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.extend_from_slice_in(bytes, arena)
    }

    /// Shorten the buffer to `new_len`, doing nothing when it is already
    /// shorter. Heap capacity is retained for reuse.
    pub fn truncate(&mut self, new_len: usize) {
        match &mut self.repr {
            BytesRepr::Inline { len, .. } => *len = (*len as usize).min(new_len) as u8,
            BytesRepr::Heap(storage) => storage.truncate(new_len),
        }
    }

    /// Clear the initialized bytes while retaining any heap capacity.
    pub fn clear(&mut self) {
        self.truncate(0);
    }

    /// Ensure room for at least `additional` more bytes.
    pub fn reserve_in(&mut self, additional: usize, arena: &mut Arena<'arena, '_>) -> Result<()> {
        let required = self
            .len()
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        u32::try_from(required).map_err(|_| CollectionError::CapacityOverflow)?;
        if required <= self.capacity() {
            return Ok(());
        }

        if let BytesRepr::Heap(storage) = &mut self.repr {
            arena.validate_owned(storage)?;
            let grown_capacity = storage.capacity().saturating_mul(2).min(u32::MAX as usize);
            if arena.try_resize_owned(storage, required.max(grown_capacity))? {
                return Ok(());
            }
        }

        let doubled_capacity = self
            .capacity()
            .saturating_mul(2)
            .min(u32::MAX as usize)
            .max(32);
        let new_capacity = required.max(doubled_capacity);
        let mut replacement = arena.alloc_owned_slice::<u8>(new_capacity)?;
        replacement.extend_copy(self.as_slice())?;
        self.repr = BytesRepr::Heap(replacement);
        Ok(())
    }

    /// Ensure room for at least `additional` more bytes.
    pub fn reserve(&mut self, additional: usize, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.reserve_in(additional, arena)
    }

    /// Reduce the buffer to its current length, returning to inline storage
    /// when possible.
    pub fn shrink_to_fit_in(&mut self, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if self.len() <= COMPACT_BYTES_INLINE_CAPACITY {
            if let BytesRepr::Heap(storage) = &self.repr {
                arena.validate_owned(storage)?;
                let mut inline = [0; COMPACT_BYTES_INLINE_CAPACITY];
                inline[..storage.len()].copy_from_slice(storage.as_slice());
                let len = storage.len() as u8;
                self.repr = BytesRepr::Inline { len, bytes: inline };
            }
            return Ok(());
        }

        let BytesRepr::Heap(storage) = &mut self.repr else {
            return Ok(());
        };
        arena.validate_owned(storage)?;
        let len = storage.len();
        if arena.try_resize_owned(storage, len)? {
            return Ok(());
        }
        let mut replacement = arena.alloc_owned_slice::<u8>(len)?;
        replacement.extend_copy(storage.as_slice())?;
        self.repr = BytesRepr::Heap(replacement);
        Ok(())
    }

    /// Reduce the buffer to its current length, returning to inline storage
    /// when possible.
    pub fn shrink_to_fit(&mut self, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.shrink_to_fit_in(arena)
    }

    /// Split off bytes from `at` onward and return them as a new buffer.
    ///
    /// On allocation failure, `self` is unchanged.
    pub fn split_off_in(&mut self, at: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        if at > self.len() {
            return Err(CollectionError::Core(compact_core::Error::OutOfBounds));
        }
        if let BytesRepr::Heap(storage) = &self.repr {
            arena.validate_owned(storage)?;
        }
        if at == 0 {
            return Ok(core::mem::take(self));
        }
        if at == self.len() {
            return Ok(Self::new());
        }
        let right = Self::from_slice_in(&self.as_slice()[at..], arena)?;
        self.truncate(at);
        Ok(right)
    }

    /// Split off bytes from `at` onward and return them as a new buffer.
    pub fn split_off(&mut self, at: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        self.split_off_in(at, arena)
    }
}

impl Default for CompactBytes<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for CompactBytes<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl DerefMut for CompactBytes<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut_slice()
    }
}

impl AsRef<[u8]> for CompactBytes<'_> {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl AsMut<[u8]> for CompactBytes<'_> {
    fn as_mut(&mut self) -> &mut [u8] {
        self
    }
}

impl Borrow<[u8]> for CompactBytes<'_> {
    fn borrow(&self) -> &[u8] {
        self
    }
}

impl BorrowMut<[u8]> for CompactBytes<'_> {
    fn borrow_mut(&mut self) -> &mut [u8] {
        self
    }
}

impl<I> Index<I> for CompactBytes<'_>
where
    [u8]: Index<I>,
{
    type Output = <[u8] as Index<I>>::Output;

    fn index(&self, index: I) -> &Self::Output {
        <[u8] as Index<I>>::index(self, index)
    }
}

impl<I> IndexMut<I> for CompactBytes<'_>
where
    [u8]: IndexMut<I>,
{
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        <[u8] as IndexMut<I>>::index_mut(self, index)
    }
}

impl<'view, 'arena> IntoIterator for &'view CompactBytes<'arena> {
    type Item = &'view u8;
    type IntoIter = core::slice::Iter<'view, u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl<'view, 'arena> IntoIterator for &'view mut CompactBytes<'arena> {
    type Item = &'view mut u8;
    type IntoIter = core::slice::IterMut<'view, u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_mut_slice().iter_mut()
    }
}

impl fmt::Debug for CompactBytes<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_slice(), formatter)
    }
}

impl PartialEq for CompactBytes<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for CompactBytes<'_> {}

impl PartialEq<[u8]> for CompactBytes<'_> {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_slice() == other
    }
}

impl PartialEq<&[u8]> for CompactBytes<'_> {
    fn eq(&self, other: &&[u8]) -> bool {
        self.as_slice() == *other
    }
}

impl PartialOrd for CompactBytes<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CompactBytes<'_> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}

impl Hash for CompactBytes<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

// SAFETY: inline bytes move with the wrapper, while heap storage remains under
// the unique movable ArenaAllocation owner.
unsafe impl CompactValue for CompactBytes<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;

    #[test]
    fn inline_capacity_candidates_keep_wrapper_growth_visible() {
        assert!(size_of::<BytesRepr<'static, 12>>() <= size_of::<BytesRepr<'static, 20>>());
        assert!(size_of::<BytesRepr<'static, 16>>() <= size_of::<BytesRepr<'static, 20>>());
        assert!(size_of::<BytesRepr<'static, 24>>() > size_of::<BytesRepr<'static, 20>>());
        assert_eq!(
            size_of::<CompactBytes<'static>>(),
            size_of::<BytesRepr<'static, COMPACT_BYTES_INLINE_CAPACITY>>()
        );
    }
}
