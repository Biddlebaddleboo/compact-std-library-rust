//! Bit-packed boolean vector used by generated struct-of-arrays columns.

use compact_core::{Arena, ByteRange32};

use crate::{CollectionError, Result};

/// A growable arena-backed boolean vector storing eight values per byte.
#[repr(C)]
pub struct CompactBitVec<'arena> {
    bytes: ByteRange32<'arena>,
    len: u32,
    capacity: u32,
}

impl<'arena> CompactBitVec<'arena> {
    /// Construct an empty bit vector without allocating storage.
    pub fn new_in(_arena: &Arena<'arena, '_>) -> Self {
        Self {
            bytes: ByteRange32::empty(),
            len: 0,
            capacity: 0,
        }
    }

    /// Allocate zeroed bits for at least `capacity` values.
    pub fn with_capacity_in(capacity: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        let capacity = u32::try_from(capacity).map_err(|_| CollectionError::CapacityOverflow)?;
        let bytes = arena.alloc_zeroed_bytes(bytes_for_bits(capacity as usize)?)?;
        Ok(Self {
            bytes,
            len: 0,
            capacity,
        })
    }

    /// Return the number of stored bits.
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Return the bit capacity.
    pub const fn capacity(&self) -> usize {
        self.capacity as usize
    }

    /// Return whether there are no stored values.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Read one bit by index.
    pub fn get(&self, index: usize, arena: &Arena<'arena, '_>) -> Result<Option<bool>> {
        if index >= self.len() {
            return Ok(None);
        }
        let byte = arena.get_bytes(self.bytes)?[index / 8];
        Ok(Some(byte & (1 << (index % 8)) != 0))
    }

    /// Set one initialized bit by index.
    pub fn set(&self, index: usize, value: bool, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if index >= self.len() {
            return Err(CollectionError::Core(compact_core::Error::OutOfBounds));
        }
        set_bit(arena.get_bytes_mut(self.bytes)?, index, value);
        Ok(())
    }

    /// Append a bit, growing the backing range when needed.
    pub fn push_in(&mut self, value: bool, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if self.len == self.capacity {
            self.reserve_in(1, arena)?;
        }
        set_bit(arena.get_bytes_mut(self.bytes)?, self.len(), value);
        self.len += 1;
        Ok(())
    }

    /// Ensure room for at least `additional` bits.
    pub fn reserve_in(&mut self, additional: usize, arena: &mut Arena<'arena, '_>) -> Result<()> {
        let required = self
            .len()
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        let required = u32::try_from(required).map_err(|_| CollectionError::CapacityOverflow)?;
        if required <= self.capacity {
            return Ok(());
        }
        let new_capacity = required.max(self.capacity.saturating_mul(2).max(8));
        let replacement = arena.alloc_zeroed_bytes(bytes_for_bits(new_capacity as usize)?)?;
        arena.copy_bytes(self.bytes, replacement, self.bytes.len())?;
        self.bytes = replacement;
        self.capacity = new_capacity;
        Ok(())
    }

    /// Drop all logical bits while retaining allocated capacity.
    pub fn clear(&mut self, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if !self.bytes.is_empty() {
            arena.get_bytes_mut(self.bytes)?.fill(0);
        }
        self.len = 0;
        Ok(())
    }

    /// Reduce the logical bit length while retaining the allocated storage.
    pub fn truncate(&mut self, len: usize) {
        self.len = self.len.min(len.min(u32::MAX as usize) as u32);
    }
}

fn bytes_for_bits(bits: usize) -> Result<usize> {
    bits.checked_add(7)
        .map(|rounded| rounded / 8)
        .ok_or(CollectionError::CapacityOverflow)
}

fn set_bit(bytes: &mut [u8], index: usize, value: bool) {
    let byte = &mut bytes[index / 8];
    let mask = 1 << (index % 8);
    if value {
        *byte |= mask;
    } else {
        *byte &= !mask;
    }
}
