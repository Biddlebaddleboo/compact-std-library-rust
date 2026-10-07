//! Bit-packed boolean vector used by generated struct-of-arrays columns.

use compact_core::{Arena, ArenaAllocation, CompactValue};

use crate::{CollectionError, Result};

/// A growable arena-backed boolean vector storing eight values per byte.
pub struct CompactBitVec<'arena> {
    bytes: Option<ArenaAllocation<'arena, u8>>,
    len: u32,
    capacity: u32,
}

impl<'arena> CompactBitVec<'arena> {
    /// Construct an empty bit vector without allocating storage.
    pub fn new_in(_arena: &Arena<'arena, '_>) -> Self {
        Self {
            bytes: None,
            len: 0,
            capacity: 0,
        }
    }

    /// Allocate zeroed bits for at least `capacity` values.
    pub fn with_capacity_in(capacity: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        let capacity = u32::try_from(capacity).map_err(|_| CollectionError::CapacityOverflow)?;
        let byte_capacity = bytes_for_bits(capacity as usize)?;
        let bytes = if byte_capacity == 0 {
            None
        } else {
            Some(zeroed_bytes(byte_capacity, arena)?)
        };
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
        let bytes = self.bytes.as_ref().expect("nonempty bit vector has bytes");
        arena.validate_owned(bytes)?;
        let byte = bytes.as_slice()[index / 8];
        Ok(Some(byte & (1 << (index % 8)) != 0))
    }

    /// Set one initialized bit by index.
    pub fn set(&mut self, index: usize, value: bool, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if index >= self.len() {
            return Err(CollectionError::Core(compact_core::Error::OutOfBounds));
        }
        let bytes = self.bytes.as_mut().expect("nonempty bit vector has bytes");
        arena.validate_owned(bytes)?;
        set_bit(bytes.as_mut_slice(), index, value);
        Ok(())
    }

    /// Append a bit, growing the backing range when needed.
    pub fn push_in(&mut self, value: bool, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if self.len == self.capacity {
            self.reserve_in(1, arena)?;
        }
        let index = self.len();
        let bytes = self
            .bytes
            .as_mut()
            .expect("reserve_in allocates bit storage");
        arena.validate_owned(bytes)?;
        set_bit(bytes.as_mut_slice(), index, value);
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
        let new_byte_capacity = bytes_for_bits(new_capacity as usize)?;
        if let Some(bytes) = &mut self.bytes {
            arena.validate_owned(bytes)?;
            if arena.try_resize_owned(bytes, new_byte_capacity)? {
                while bytes.len() < new_byte_capacity {
                    bytes.push(0)?;
                }
                self.capacity = new_capacity;
                return Ok(());
            }
        }
        let mut replacement = arena.alloc_owned_slice::<u8>(new_byte_capacity)?;
        if let Some(bytes) = &self.bytes {
            replacement.extend_copy(bytes.as_slice())?;
        }
        while replacement.len() < new_byte_capacity {
            replacement.push(0)?;
        }
        self.bytes = Some(replacement);
        self.capacity = new_capacity;
        Ok(())
    }

    /// Drop all logical bits while retaining allocated capacity.
    pub fn clear(&mut self, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if let Some(bytes) = &mut self.bytes {
            arena.validate_owned(bytes)?;
            bytes.as_mut_slice().fill(0);
        }
        self.len = 0;
        Ok(())
    }

    /// Reduce the logical bit length while retaining the allocated storage.
    pub fn truncate(&mut self, len: usize) {
        self.len = self.len.min(len.min(u32::MAX as usize) as u32);
    }
}

// SAFETY: this wrapper contains only an owned u8 buffer and scalar metadata.
unsafe impl CompactValue for CompactBitVec<'_> {}

fn zeroed_bytes<'arena>(
    capacity: usize,
    arena: &mut Arena<'arena, '_>,
) -> Result<ArenaAllocation<'arena, u8>> {
    let mut bytes = arena.alloc_owned_slice::<u8>(capacity)?;
    for _ in 0..capacity {
        bytes.push(0)?;
    }
    Ok(bytes)
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
