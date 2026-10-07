//! Contiguous arena-backed vector with compact `offset/len/capacity` metadata.

use core::mem::MaybeUninit;
use core::slice;

use compact_core::{Arena, OffsetSlice32};

use crate::{CollectionError, Result};

/// A contiguous arena-backed vector.
///
/// The general metadata is twelve bytes: a four-byte storage offset, a
/// four-byte initialized length, and a four-byte capacity. Growth doubles
/// capacity, so old monotonic-arena buffers consume less than twice the final
/// capacity in aggregate. Values must be `Copy`; native destructors are never
/// silently skipped.
#[repr(C)]
pub struct CompactVec<'arena, T: Copy> {
    storage: OffsetSlice32<'arena, MaybeUninit<T>>,
    len: u32,
}

impl<'arena, T: Copy> CompactVec<'arena, T> {
    /// Construct an empty vector tied to `arena` without allocating storage.
    pub fn new_in(_arena: &Arena<'arena, '_>) -> Self {
        Self {
            storage: OffsetSlice32::empty(),
            len: 0,
        }
    }

    /// Allocate an empty vector with at least `capacity` element slots.
    pub fn with_capacity_in(capacity: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        let capacity = u32::try_from(capacity).map_err(|_| CollectionError::CapacityOverflow)?;
        let storage = if capacity == 0 {
            OffsetSlice32::empty()
        } else {
            arena.alloc_uninit_slice::<T>(capacity as usize)?
        };
        Ok(Self { storage, len: 0 })
    }

    /// Return the number of initialized elements.
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Return the allocated element capacity.
    pub const fn capacity(&self) -> usize {
        self.storage.len()
    }

    /// Return whether the vector is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Ensure room for at least `additional` more elements.
    pub fn reserve_in(&mut self, additional: usize, arena: &mut Arena<'arena, '_>) -> Result<()> {
        let required = self
            .len()
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        let required = u32::try_from(required).map_err(|_| CollectionError::CapacityOverflow)?;
        if required as usize <= self.storage.len() {
            return Ok(());
        }

        let old_capacity = self.storage.len() as u32;
        let doubled = old_capacity.saturating_mul(2);
        let new_capacity = if old_capacity == 0 {
            required.max(4)
        } else {
            required.max(doubled)
        };
        let replacement = arena.alloc_uninit_slice::<T>(new_capacity as usize)?;
        // SAFETY: `CompactVec` only increments len after initializing each
        // inserted slot, and all mutation methods preserve that prefix. The
        // new allocation has room and is not observed before the copy ends.
        unsafe { arena.copy_slice_assume_init(self.storage, self.len(), replacement)? };
        self.storage = replacement;
        Ok(())
    }

    /// Append one `Copy` value, growing the compact allocation when needed.
    pub fn push_in(&mut self, value: T, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.reserve_in(1, arena)?;
        let index = self.len();
        arena.write_uninit_at(self.storage, index, value)?;
        let next_len = index
            .checked_add(1)
            .ok_or(CollectionError::CapacityOverflow)?;
        self.len = u32::try_from(next_len).map_err(|_| CollectionError::CapacityOverflow)?;
        Ok(())
    }

    /// Remove and return the last value, if any.
    pub fn pop_in(&mut self, arena: &Arena<'arena, '_>) -> Result<Option<T>> {
        if self.len == 0 {
            return Ok(None);
        }
        let index = self.len() - 1;
        let value = self.as_slice(arena)?[index];
        self.len -= 1;
        Ok(Some(value))
    }

    /// Return an initialized element by index.
    pub fn get<'view>(
        &self,
        index: usize,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<Option<&'view T>> {
        Ok(self.as_slice(arena)?.get(index))
    }

    /// Mutably borrow an initialized element by index.
    pub fn get_mut<'view>(
        &self,
        index: usize,
        arena: &'view mut Arena<'arena, '_>,
    ) -> Result<Option<&'view mut T>> {
        Ok(self.as_mut_slice(arena)?.get_mut(index))
    }

    /// Borrow all initialized elements as a zero-copy native slice.
    pub fn as_slice<'view>(&self, arena: &'view Arena<'arena, '_>) -> Result<&'view [T]> {
        if self.len == 0 {
            return Ok(&[]);
        }
        // SAFETY: the private length is advanced only after a successful slot
        // initialization, and is reduced before elements cease to be exposed.
        Ok(unsafe { arena.get_slice_assume_init(self.storage, self.len())? })
    }

    /// Mutably borrow all initialized elements as a zero-copy native slice.
    pub fn as_mut_slice<'view>(
        &self,
        arena: &'view mut Arena<'arena, '_>,
    ) -> Result<&'view mut [T]> {
        if self.len == 0 {
            return Ok(&mut []);
        }
        // SAFETY: the private length tracks initialized values, and the
        // exclusive arena borrow prevents any competing references.
        Ok(unsafe { arena.get_slice_mut_assume_init(self.storage, self.len())? })
    }

    /// Return an iterator over the initialized values.
    pub fn iter<'view>(&self, arena: &'view Arena<'arena, '_>) -> Result<slice::Iter<'view, T>> {
        Ok(self.as_slice(arena)?.iter())
    }

    /// Reduce the initialized length without reclaiming monotonic arena bytes.
    pub fn truncate(&mut self, len: usize) {
        self.len = self.len.min(len.min(u32::MAX as usize) as u32);
    }

    /// Remove all elements without reclaiming the backing allocation.
    pub fn clear(&mut self) {
        self.len = 0;
    }
}
