//! Compact byte buffer with a twenty-byte inline payload.

use compact_backend_std::{CageAllocation, CompactRuntime};
use compact_core::CompactValue;
use core::borrow::{Borrow, BorrowMut};
use core::hash::{Hash, Hasher};
use core::ops::{Deref, DerefMut, Index, IndexMut};

use crate::{CollectionError, Result};

/// Inline payload selected for the V2.3 byte wrapper.
pub const COMPACT_BYTES_INLINE_CAPACITY: usize = 20;

enum BytesRepr {
    Inline {
        len: u8,
        bytes: [u8; COMPACT_BYTES_INLINE_CAPACITY],
    },
    Heap(CageAllocation<u8>),
}

/// Byte vector with inline storage and a four-byte cage owner when promoted.
pub struct CompactBytes {
    repr: BytesRepr,
}

impl CompactBytes {
    /// Construct an empty inline byte buffer.
    pub const fn new() -> Self {
        Self {
            repr: BytesRepr::Inline {
                len: 0,
                bytes: [0; COMPACT_BYTES_INLINE_CAPACITY],
            },
        }
    }
    /// Allocate a buffer with the requested capacity.
    pub fn with_capacity(capacity: usize) -> Result<Self> {
        if capacity <= COMPACT_BYTES_INLINE_CAPACITY {
            return Ok(Self::new());
        }
        Ok(Self {
            repr: BytesRepr::Heap(CompactRuntime::alloc_owned_slice(capacity)?),
        })
    }
    /// Copy a byte slice into compact storage.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let mut result = Self::with_capacity(bytes.len())?;
        result.extend_from_slice(bytes)?;
        Ok(result)
    }
    /// Return the number of initialized bytes.
    pub fn len(&self) -> usize {
        match &self.repr {
            BytesRepr::Inline { len, .. } => *len as usize,
            BytesRepr::Heap(a) => a.len(),
        }
    }
    /// Return the current byte capacity.
    pub fn capacity(&self) -> usize {
        match &self.repr {
            BytesRepr::Inline { .. } => COMPACT_BYTES_INLINE_CAPACITY,
            BytesRepr::Heap(a) => a.capacity(),
        }
    }
    /// Return whether no bytes are stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Borrow the initialized bytes.
    pub fn as_slice(&self) -> &[u8] {
        match &self.repr {
            BytesRepr::Inline { len, bytes } => &bytes[..*len as usize],
            BytesRepr::Heap(a) => a.as_slice(),
        }
    }
    /// Return a pointer valid until mutation or drop.
    pub fn as_ptr(&self) -> *const u8 {
        self.as_slice().as_ptr()
    }
    /// Expose bytes to a synchronous native call.
    pub fn with_ffi_bytes<R>(&self, call: impl FnOnce(&[u8]) -> R) -> R {
        call(self.as_slice())
    }
    /// Mutably borrow the initialized bytes.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        match &mut self.repr {
            BytesRepr::Inline { len, bytes } => &mut bytes[..*len as usize],
            BytesRepr::Heap(a) => a.as_mut_slice(),
        }
    }
    /// Append one byte.
    pub fn push(&mut self, byte: u8) -> Result<()> {
        let required = self
            .len()
            .checked_add(1)
            .ok_or(CollectionError::CapacityOverflow)?;
        if let BytesRepr::Inline { len, bytes } = &mut self.repr {
            if required <= COMPACT_BYTES_INLINE_CAPACITY {
                bytes[*len as usize] = byte;
                *len = required as u8;
                return Ok(());
            }
        }
        self.reserve(1)?;
        if let BytesRepr::Heap(a) = &mut self.repr {
            a.push(byte)?;
        }
        Ok(())
    }
    /// Append a byte slice.
    pub fn extend_from_slice(&mut self, value: &[u8]) -> Result<()> {
        if value.is_empty() {
            return Ok(());
        }
        let new_len = self
            .len()
            .checked_add(value.len())
            .ok_or(CollectionError::CapacityOverflow)?;
        if let BytesRepr::Inline { len, bytes } = &mut self.repr {
            if new_len <= COMPACT_BYTES_INLINE_CAPACITY {
                let start = *len as usize;
                bytes[start..new_len].copy_from_slice(value);
                *len = new_len as u8;
                return Ok(());
            }
        }
        self.reserve(value.len())?;
        if let BytesRepr::Heap(a) = &mut self.repr {
            a.extend_copy(value)?;
        }
        Ok(())
    }
    /// Drop bytes after `new_len`.
    pub fn truncate(&mut self, new_len: usize) {
        if let BytesRepr::Inline { len, .. } = &mut self.repr {
            *len = (*len as usize).min(new_len) as u8;
        } else if let BytesRepr::Heap(a) = &mut self.repr {
            a.truncate(new_len);
        }
    }
    /// Clear while retaining allocated storage.
    pub fn clear(&mut self) {
        self.truncate(0);
    }
    /// Ensure room for at least `additional` bytes.
    pub fn reserve(&mut self, additional: usize) -> Result<()> {
        let required = self
            .len()
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        if required <= self.capacity() {
            return Ok(());
        }
        let capacity = required.max(self.capacity().saturating_mul(2));
        if let BytesRepr::Heap(a) = &mut self.repr {
            if a.try_resize(capacity)? {
                return Ok(());
            }
        }
        let mut replacement = CompactRuntime::alloc_owned_slice::<u8>(capacity)?;
        replacement.extend_copy(self.as_slice())?;
        self.repr = BytesRepr::Heap(replacement);
        Ok(())
    }
    /// Release unused capacity or return short content to inline storage.
    pub fn shrink_to_fit(&mut self) -> Result<()> {
        if self.len() <= COMPACT_BYTES_INLINE_CAPACITY {
            if let BytesRepr::Heap(a) = &self.repr {
                let len = a.len();
                let mut bytes = [0; COMPACT_BYTES_INLINE_CAPACITY];
                bytes[..len].copy_from_slice(a.as_slice());
                self.repr = BytesRepr::Inline {
                    len: len as u8,
                    bytes,
                };
            }
            return Ok(());
        }
        let BytesRepr::Heap(a) = &mut self.repr else {
            return Ok(());
        };
        let len = a.len();
        if a.try_resize(len)? {
            return Ok(());
        }
        let mut replacement = CompactRuntime::alloc_owned_slice::<u8>(len)?;
        replacement.extend_copy(a.as_slice())?;
        self.repr = BytesRepr::Heap(replacement);
        Ok(())
    }
    /// Remove the suffix beginning at `at` and return it as a new buffer.
    pub fn split_off(&mut self, at: usize) -> Result<Self> {
        if at > self.len() {
            return Err(CollectionError::Core(compact_core::Error::OutOfBounds));
        }
        let tail = Self::from_slice(&self.as_slice()[at..])?;
        self.truncate(at);
        Ok(tail)
    }
}

impl Default for CompactBytes {
    fn default() -> Self {
        Self::new()
    }
}
impl Deref for CompactBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}
impl DerefMut for CompactBytes {
    fn deref_mut(&mut self) -> &mut [u8] {
        self.as_mut_slice()
    }
}
impl AsRef<[u8]> for CompactBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}
impl AsMut<[u8]> for CompactBytes {
    fn as_mut(&mut self) -> &mut [u8] {
        self
    }
}
impl Borrow<[u8]> for CompactBytes {
    fn borrow(&self) -> &[u8] {
        self
    }
}
impl BorrowMut<[u8]> for CompactBytes {
    fn borrow_mut(&mut self) -> &mut [u8] {
        self
    }
}
impl<I> Index<I> for CompactBytes
where
    [u8]: Index<I>,
{
    type Output = <[u8] as Index<I>>::Output;
    fn index(&self, i: I) -> &Self::Output {
        <[u8] as Index<I>>::index(self.as_slice(), i)
    }
}
impl<I> IndexMut<I> for CompactBytes
where
    [u8]: IndexMut<I>,
{
    fn index_mut(&mut self, i: I) -> &mut Self::Output {
        <[u8] as IndexMut<I>>::index_mut(self.as_mut_slice(), i)
    }
}
impl core::fmt::Debug for CompactBytes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("CompactBytes")
            .field(&self.as_slice())
            .finish()
    }
}
impl PartialEq for CompactBytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}
impl Eq for CompactBytes {}
impl PartialEq<[u8]> for CompactBytes {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_slice() == other
    }
}
impl PartialEq<&[u8]> for CompactBytes {
    fn eq(&self, other: &&[u8]) -> bool {
        self.as_slice() == *other
    }
}
impl PartialOrd for CompactBytes {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for CompactBytes {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}
impl Hash for CompactBytes {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}
// SAFETY: inline bytes move with the wrapper and heap storage is a unique cage owner.
unsafe impl CompactValue for CompactBytes {}
