//! Contiguous arena-backed vector with explicit ownership and reclamation.

use core::slice;

use compact_core::{Arena, ArenaAllocation, CompactValue};

use crate::{CollectionError, Result};

/// A contiguous arena-backed vector.
///
/// The vector owns a unique arena-allocation token. Dropping the vector runs
/// element destructors for the initialized prefix and returns the buffer to the
/// arena's reusable allocator.
pub struct CompactVec<'arena, T: CompactValue> {
    storage: Option<ArenaAllocation<'arena, T>>,
}

impl<'arena, T: CompactValue> CompactVec<'arena, T> {
    /// Construct an empty vector tied to `arena` without allocating storage.
    pub fn new_in(_arena: &Arena<'arena, '_>) -> Self {
        Self { storage: None }
    }

    /// Allocate an empty vector with at least `capacity` element slots.
    pub fn with_capacity_in(capacity: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        u32::try_from(capacity).map_err(|_| CollectionError::CapacityOverflow)?;
        let storage = if capacity == 0 {
            None
        } else {
            Some(arena.alloc_owned_slice::<T>(capacity)?)
        };
        Ok(Self { storage })
    }

    /// Return the number of initialized elements.
    pub fn len(&self) -> usize {
        self.storage.as_ref().map_or(0, ArenaAllocation::len)
    }

    /// Return the allocated element capacity.
    pub fn capacity(&self) -> usize {
        self.storage.as_ref().map_or(0, ArenaAllocation::capacity)
    }

    /// Return whether the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Ensure room for at least `additional` more elements.
    ///
    /// Allocation failure leaves the initialized value sequence unchanged.
    pub fn reserve_in(&mut self, additional: usize, arena: &mut Arena<'arena, '_>) -> Result<()> {
        let required = self
            .len()
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        let required = u32::try_from(required).map_err(|_| CollectionError::CapacityOverflow)?;
        if required as usize <= self.capacity() {
            return Ok(());
        }

        let old_capacity = self.capacity() as u32;
        let doubled = old_capacity.saturating_mul(2);
        let new_capacity = if old_capacity == 0 {
            required.max(4)
        } else {
            required.max(doubled)
        } as usize;

        if let Some(storage) = &mut self.storage {
            if arena.try_resize_owned(storage, new_capacity)? {
                return Ok(());
            }
        }

        let mut replacement = arena.alloc_owned_slice::<T>(new_capacity)?;
        if let Some(storage) = &mut self.storage {
            storage.move_into(&mut replacement)?;
        }
        self.storage = Some(replacement);
        Ok(())
    }

    /// Append one value, growing the compact allocation when needed.
    pub fn push_in(&mut self, value: T, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.reserve_in(1, arena)?;
        let storage = self
            .storage
            .as_mut()
            .expect("reserve_in allocates storage for one element");
        arena.validate_owned(storage)?;
        storage.push(value)?;
        Ok(())
    }

    /// Remove and return the last value, if any.
    pub fn pop_in(&mut self, arena: &Arena<'arena, '_>) -> Result<Option<T>> {
        let Some(storage) = &mut self.storage else {
            return Ok(None);
        };
        arena.validate_owned(storage)?;
        Ok(storage.pop())
    }

    /// Return an initialized element by index.
    pub fn get<'view>(
        &'view self,
        index: usize,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<Option<&'view T>> {
        if let Some(storage) = &self.storage {
            arena.validate_owned(storage)?;
            Ok(storage.get(index))
        } else {
            Ok(None)
        }
    }

    /// Mutably borrow an initialized element by index.
    pub fn get_mut<'view>(
        &'view mut self,
        index: usize,
        arena: &'view mut Arena<'arena, '_>,
    ) -> Result<Option<&'view mut T>> {
        if let Some(storage) = &mut self.storage {
            arena.validate_owned(storage)?;
            Ok(storage.get_mut(index))
        } else {
            Ok(None)
        }
    }

    /// Borrow all initialized elements as a zero-copy native slice.
    pub fn as_slice<'view>(&'view self, arena: &'view Arena<'arena, '_>) -> Result<&'view [T]> {
        if let Some(storage) = &self.storage {
            arena.validate_owned(storage)?;
            Ok(storage.as_slice())
        } else {
            Ok(&[])
        }
    }

    /// Mutably borrow all initialized elements as a zero-copy native slice.
    pub fn as_mut_slice<'view>(
        &'view mut self,
        arena: &'view mut Arena<'arena, '_>,
    ) -> Result<&'view mut [T]> {
        if let Some(storage) = &mut self.storage {
            arena.validate_owned(storage)?;
            Ok(storage.as_mut_slice())
        } else {
            Ok(&mut [])
        }
    }

    /// Return an iterator over the initialized values.
    pub fn iter<'view>(
        &'view self,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<slice::Iter<'view, T>> {
        Ok(self.as_slice(arena)?.iter())
    }

    /// Drop every initialized value after `len`.
    pub fn truncate(&mut self, len: usize) {
        if let Some(storage) = &mut self.storage {
            storage.truncate(len);
        }
    }

    /// Drop all values while retaining the backing allocation for reuse.
    pub fn clear(&mut self) {
        self.truncate(0);
    }

    /// Reduce the buffer to the current length, releasing any unused tail.
    pub fn shrink_to_fit_in(&mut self, arena: &mut Arena<'arena, '_>) -> Result<()> {
        let Some(storage) = &mut self.storage else {
            return Ok(());
        };
        arena.validate_owned(storage)?;
        let len = storage.len();
        if len == 0 {
            self.storage = None;
            return Ok(());
        }
        if arena.try_resize_owned(storage, len)? {
            return Ok(());
        }
        let mut replacement = arena.alloc_owned_slice::<T>(len)?;
        storage.move_into(&mut replacement)?;
        self.storage = Some(replacement);
        Ok(())
    }

    pub(crate) fn allocation_mut(&mut self) -> Option<&mut ArenaAllocation<'arena, T>> {
        self.storage.as_mut()
    }
}

// SAFETY: moving this owner transfers its unique ArenaAllocation token; its
// element invariants are governed by T: CompactValue and no address-sensitive
// data is introduced by the vector wrapper.
unsafe impl<T: CompactValue> CompactValue for CompactVec<'_, T> {}
