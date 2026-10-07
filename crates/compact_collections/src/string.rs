//! Compact UTF-8 string with a twelve-byte inline representation.

use compact_core::{Arena, ByteRange32};

use crate::{CollectionError, Result};

const INLINE_CAPACITY: usize = 12;
const HEAP_TAG: u32 = u32::MAX;

/// An arena-backed UTF-8 string with twelve inline bytes.
///
/// The representation occupies sixteen bytes: a four-byte mode/length header
/// and twelve payload bytes. Inline strings allocate nothing. Long strings
/// store an arena offset, length, and capacity in those payload bytes.
pub struct CompactString<'arena> {
    tag: u32,
    payload: [u8; INLINE_CAPACITY],
    marker: core::marker::PhantomData<fn(&'arena mut ()) -> &'arena mut ()>,
}

impl<'arena> CompactString<'arena> {
    /// Construct an empty inline string tied to `arena`.
    pub fn new_in(_arena: &Arena<'arena, '_>) -> Self {
        Self::empty()
    }

    /// Construct an empty inline string without arena allocation.
    pub const fn empty() -> Self {
        Self {
            tag: 0,
            payload: [0; INLINE_CAPACITY],
            marker: core::marker::PhantomData,
        }
    }

    /// Copy a UTF-8 string into compact arena storage.
    pub fn from_str_in(value: &str, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        if value.len() <= INLINE_CAPACITY {
            let mut result = Self::empty();
            result.tag = value.len() as u32;
            result.payload[..value.len()].copy_from_slice(value.as_bytes());
            return Ok(result);
        }
        let range = arena.alloc_zeroed_bytes(value.len())?;
        arena.get_bytes_mut(range)?[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self::heap(range, value.len(), value.len()))
    }

    /// Return the UTF-8 byte length.
    pub fn len(&self) -> usize {
        if self.is_heap() {
            self.heap_len()
        } else {
            self.tag as usize
        }
    }

    /// Return whether the string is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return the current storage capacity in bytes.
    pub fn capacity(&self) -> usize {
        if self.is_heap() {
            self.heap_capacity()
        } else {
            INLINE_CAPACITY
        }
    }

    /// Borrow the exact initialized UTF-8 bytes.
    pub fn as_bytes<'view>(&'view self, arena: &'view Arena<'arena, '_>) -> Result<&'view [u8]> {
        if self.is_heap() {
            let range = self.heap_range();
            Ok(&arena.get_bytes(range)?[..self.heap_len()])
        } else {
            Ok(&self.payload[..self.tag as usize])
        }
    }

    /// Borrow the string as a zero-copy native `&str`.
    pub fn as_str<'view>(&'view self, arena: &'view Arena<'arena, '_>) -> Result<&'view str> {
        core::str::from_utf8(self.as_bytes(arena)?).map_err(|_| CollectionError::InvalidUtf8)
    }

    /// Append UTF-8 text, preserving the old value if arena allocation fails.
    pub fn push_str_in(&mut self, value: &str, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if value.is_empty() {
            return Ok(());
        }
        let old_len = self.len();
        let required = old_len
            .checked_add(value.len())
            .ok_or(CollectionError::CapacityOverflow)?;

        if !self.is_heap() && required <= INLINE_CAPACITY {
            self.payload[old_len..required].copy_from_slice(value.as_bytes());
            self.tag = required as u32;
            return Ok(());
        }

        if self.is_heap() && required <= self.heap_capacity() {
            let range = self.heap_range();
            arena.get_bytes_mut(range)?[old_len..required].copy_from_slice(value.as_bytes());
            self.set_heap_len(required);
            return Ok(());
        }

        let old_capacity = self.capacity();
        let new_capacity = required.max(old_capacity.saturating_mul(2).max(16));
        let replacement = arena.alloc_zeroed_bytes(new_capacity)?;
        if self.is_heap() {
            // The replacement allocation is newer and disjoint from the
            // existing string. Core performs the copy under one arena borrow.
            arena.copy_bytes(self.heap_range(), replacement, old_len)?;
        } else if old_len != 0 {
            arena.get_bytes_mut(replacement)?[..old_len].copy_from_slice(&self.payload[..old_len]);
        }
        arena.get_bytes_mut(replacement)?[old_len..required].copy_from_slice(value.as_bytes());
        *self = Self::heap(replacement, required, new_capacity);
        Ok(())
    }

    /// Append one Unicode scalar value.
    pub fn push_char_in(&mut self, value: char, arena: &mut Arena<'arena, '_>) -> Result<()> {
        let mut encoded = [0_u8; 4];
        self.push_str_in(value.encode_utf8(&mut encoded), arena)
    }

    /// Clear the string and return it to the empty inline representation.
    pub fn clear(&mut self) {
        *self = Self::empty();
    }

    /// Truncate at a UTF-8 character boundary.
    pub fn truncate_in(&mut self, new_len: usize, arena: &Arena<'arena, '_>) -> Result<()> {
        if new_len >= self.len() {
            return Ok(());
        }
        let value = self.as_str(arena)?;
        if !value.is_char_boundary(new_len) {
            return Err(CollectionError::Core(compact_core::Error::OutOfBounds));
        }
        if new_len <= INLINE_CAPACITY {
            let mut inline = [0; INLINE_CAPACITY];
            inline[..new_len].copy_from_slice(&value.as_bytes()[..new_len]);
            self.payload = inline;
            self.tag = new_len as u32;
            return Ok(());
        }
        self.set_heap_len(new_len);
        Ok(())
    }

    /// Compare with an ordinary native string.
    pub fn eq_str(&self, other: &str, arena: &Arena<'arena, '_>) -> Result<bool> {
        Ok(self.as_str(arena)? == other)
    }

    fn is_heap(&self) -> bool {
        self.tag == HEAP_TAG
    }

    fn heap(range: ByteRange32<'arena>, len: usize, capacity: usize) -> Self {
        let mut result = Self {
            tag: HEAP_TAG,
            payload: [0; INLINE_CAPACITY],
            marker: core::marker::PhantomData,
        };
        result.payload[..4].copy_from_slice(&range.offset().to_ne_bytes());
        result.payload[4..8].copy_from_slice(&(len as u32).to_ne_bytes());
        result.payload[8..12].copy_from_slice(&(capacity as u32).to_ne_bytes());
        result
    }

    fn heap_range(&self) -> ByteRange32<'arena> {
        let offset = u32::from_ne_bytes(self.payload[..4].try_into().unwrap());
        let capacity = self.heap_capacity() as u32;
        // SAFETY: private heap metadata is written only from a valid
        // zero-initialized byte allocation returned by the same arena.
        unsafe { ByteRange32::from_raw_parts_unchecked(offset, capacity) }
    }

    fn heap_len(&self) -> usize {
        u32::from_ne_bytes(self.payload[4..8].try_into().unwrap()) as usize
    }

    fn heap_capacity(&self) -> usize {
        u32::from_ne_bytes(self.payload[8..12].try_into().unwrap()) as usize
    }

    fn set_heap_len(&mut self, len: usize) {
        self.payload[4..8].copy_from_slice(&(len as u32).to_ne_bytes());
    }
}
