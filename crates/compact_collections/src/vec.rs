//! Contiguous arena-backed vector with explicit ownership and reclamation.

use core::borrow::{Borrow, BorrowMut};
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::{Deref, DerefMut, Index, IndexMut};
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
        arena: &Arena<'arena, '_>,
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
    pub fn as_slice<'view>(&'view self, arena: &Arena<'arena, '_>) -> Result<&'view [T]> {
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

impl<T: CompactValue> Deref for CompactVec<'_, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.storage.as_ref().map_or(&[], ArenaAllocation::as_slice)
    }
}

impl<T: CompactValue> DerefMut for CompactVec<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.storage
            .as_mut()
            .map_or(&mut [], ArenaAllocation::as_mut_slice)
    }
}

impl<T: CompactValue> AsRef<[T]> for CompactVec<'_, T> {
    fn as_ref(&self) -> &[T] {
        self
    }
}

impl<T: CompactValue> AsMut<[T]> for CompactVec<'_, T> {
    fn as_mut(&mut self) -> &mut [T] {
        self
    }
}

impl<T: CompactValue> Borrow<[T]> for CompactVec<'_, T> {
    fn borrow(&self) -> &[T] {
        self
    }
}

impl<T: CompactValue> BorrowMut<[T]> for CompactVec<'_, T> {
    fn borrow_mut(&mut self) -> &mut [T] {
        self
    }
}

impl<T: CompactValue, I> Index<I> for CompactVec<'_, T>
where
    [T]: Index<I>,
{
    type Output = <[T] as Index<I>>::Output;

    fn index(&self, index: I) -> &Self::Output {
        <[T] as Index<I>>::index(self, index)
    }
}

impl<T: CompactValue, I> IndexMut<I> for CompactVec<'_, T>
where
    [T]: IndexMut<I>,
{
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        <[T] as IndexMut<I>>::index_mut(self, index)
    }
}

impl<'view, 'arena, T: CompactValue> IntoIterator for &'view CompactVec<'arena, T> {
    type Item = &'view T;
    type IntoIter = slice::Iter<'view, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.deref().iter()
    }
}

impl<'view, 'arena, T: CompactValue> IntoIterator for &'view mut CompactVec<'arena, T> {
    type Item = &'view mut T;
    type IntoIter = slice::IterMut<'view, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.deref_mut().iter_mut()
    }
}

impl<T: CompactValue + fmt::Debug> fmt::Debug for CompactVec<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.deref().iter()).finish()
    }
}

impl<T: CompactValue + PartialEq> PartialEq for CompactVec<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl<T: CompactValue + Eq> Eq for CompactVec<'_, T> {}

impl<T: CompactValue + PartialOrd> PartialOrd for CompactVec<'_, T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        self.deref().partial_cmp(other.deref())
    }
}

impl<T: CompactValue + Ord> Ord for CompactVec<'_, T> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.deref().cmp(other.deref())
    }
}

impl<T: CompactValue + Hash> Hash for CompactVec<'_, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.deref().hash(state);
    }
}

// SAFETY: moving this owner transfers its unique ArenaAllocation token; its
// element invariants are governed by T: CompactValue and no address-sensitive
// data is introduced by the vector wrapper.
unsafe impl<T: CompactValue> CompactValue for CompactVec<'_, T> {}
