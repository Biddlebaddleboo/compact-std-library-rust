//! Packed boolean vector stored as cage-backed bytes.

use compact_backend_std::{CageAllocation, CompactRuntime};
use compact_core::CompactValue;

use crate::{CollectionError, Result};

/// A compact vector that stores one boolean per bit.
pub struct CompactBitVec {
    bytes: Option<CageAllocation<u8>>,
    len: u32,
}

impl CompactBitVec {
    /// Construct an empty bit vector.
    pub const fn new() -> Self {
        Self {
            bytes: None,
            len: 0,
        }
    }
    /// Reserve enough bits for `capacity` values.
    pub fn with_capacity(capacity: usize) -> Result<Self> {
        let mut result = Self::new();
        result.reserve(capacity)?;
        Ok(result)
    }
    /// Return the number of bits stored.
    pub const fn len(&self) -> usize {
        self.len as usize
    }
    /// Return bit capacity.
    pub fn capacity(&self) -> usize {
        self.bytes.as_ref().map_or(0, |b| b.capacity() * 8)
    }
    /// Return whether the vector is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Read one bit.
    pub fn get(&self, index: usize) -> Option<bool> {
        if index >= self.len() {
            return None;
        }
        let bytes = self.bytes.as_ref()?;
        Some((bytes.as_slice()[index / 8] & (1 << (index % 8))) != 0)
    }
    /// Set one bit.
    pub fn set(&mut self, index: usize, value: bool) -> Result<()> {
        if index >= self.len() {
            return Err(CollectionError::Core(compact_core::Error::OutOfBounds));
        }
        let byte = &mut self
            .bytes
            .as_mut()
            .expect("nonempty bitvec has bytes")
            .as_mut_slice()[index / 8];
        let mask = 1_u8 << (index % 8);
        if value {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
        Ok(())
    }
    /// Append one bit.
    pub fn push(&mut self, value: bool) -> Result<()> {
        let index = self.len();
        if index == self.capacity() {
            self.reserve(1)?;
        }
        let byte_index = index / 8;
        if index % 8 == 0 {
            self.bytes
                .as_mut()
                .expect("reserve allocated bytes")
                .push(0)?;
        }
        if value {
            self.bytes.as_mut().unwrap().as_mut_slice()[byte_index] |= 1 << (index % 8);
        }
        self.len += 1;
        Ok(())
    }
    /// Ensure room for at least `additional` more bits.
    pub fn reserve(&mut self, additional: usize) -> Result<()> {
        let required = self
            .len()
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        let bytes_required = required
            .checked_add(7)
            .ok_or(CollectionError::CapacityOverflow)?
            / 8;
        if bytes_required <= self.bytes.as_ref().map_or(0, |b| b.capacity()) {
            return Ok(());
        }
        let old_capacity = self.bytes.as_ref().map_or(0, |b| b.capacity());
        let new_capacity = bytes_required.max(old_capacity.saturating_mul(2).max(1));
        if let Some(bytes) = &mut self.bytes {
            if bytes.try_resize(new_capacity)? {
                return Ok(());
            }
        }
        let mut replacement = CompactRuntime::alloc_owned_slice::<u8>(new_capacity)?;
        if let Some(bytes) = &mut self.bytes {
            bytes.move_into(&mut replacement)?;
        }
        self.bytes = Some(replacement);
        Ok(())
    }
    /// Drop all bits and release storage.
    pub fn clear(&mut self) {
        self.bytes = None;
        self.len = 0;
    }
    /// Truncate at the requested bit length.
    pub fn truncate(&mut self, len: usize) {
        if len >= self.len() {
            return;
        }
        self.len = len as u32;
        if let Some(bytes) = &mut self.bytes {
            let used_bytes = len.saturating_add(7) / 8;
            bytes.truncate(used_bytes);
            if len % 8 != 0 && used_bytes > 0 {
                let mask = (1_u8 << (len % 8)) - 1;
                bytes.as_mut_slice()[used_bytes - 1] &= mask;
            }
        }
    }
}

impl Default for CompactBitVec {
    fn default() -> Self {
        Self::new()
    }
}
// SAFETY: this wrapper owns a compact byte block and a scalar bit count.
unsafe impl CompactValue for CompactBitVec {}
