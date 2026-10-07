//! Fallible compact vector backed by one process-cage allocation.

use core::borrow::{Borrow, BorrowMut};
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::{Deref, DerefMut, Index, IndexMut};
use core::ptr;
use core::slice;

use compact_backend_std::{CageAllocation, CompactRuntime};
use compact_core::CompactValue;

use crate::{CollectionError, Result};

/// A compact vector represented by its four-byte cage owner.
pub struct CompactVec<T: CompactValue> {
    storage: Option<CageAllocation<T>>,
}

impl<T: CompactValue> CompactVec<T> {
    /// Construct an empty vector without allocating cage space.
    pub const fn new() -> Self {
        Self { storage: None }
    }

    /// Allocate an empty vector with at least `capacity` element slots.
    pub fn with_capacity(capacity: usize) -> Result<Self> {
        u32::try_from(capacity).map_err(|_| CollectionError::CapacityOverflow)?;
        let storage = if capacity == 0 {
            None
        } else {
            Some(CompactRuntime::alloc_owned_slice(capacity)?)
        };
        Ok(Self { storage })
    }

    /// Return the initialized element count.
    pub fn len(&self) -> usize {
        self.storage.as_ref().map_or(0, |storage| storage.len())
    }
    /// Return the allocated element capacity.
    pub fn capacity(&self) -> usize {
        self.storage
            .as_ref()
            .map_or(0, |storage| storage.capacity())
    }
    /// Return whether the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Ensure room for at least `additional` more elements.
    pub fn reserve(&mut self, additional: usize) -> Result<()> {
        let (len, old) = self
            .storage
            .as_ref()
            .map_or((0, 0), CageAllocation::len_capacity);
        let required = len
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        let required =
            u32::try_from(required).map_err(|_| CollectionError::CapacityOverflow)? as usize;
        if required <= old {
            return Ok(());
        }
        let new_capacity = required.max(if old == 0 { 4 } else { old.saturating_mul(2) });
        if let Some(storage) = &mut self.storage {
            if storage.try_resize(new_capacity)? {
                return Ok(());
            }
        }
        let mut replacement = CompactRuntime::alloc_owned_slice::<T>(new_capacity)?;
        if let Some(storage) = &mut self.storage {
            storage.move_into(&mut replacement)?;
        }
        self.storage = Some(replacement);
        Ok(())
    }

    /// Append one value, growing the compact allocation when needed.
    pub fn push(&mut self, value: T) -> Result<()> {
        self.reserve(1)?;
        self.storage
            .as_mut()
            .expect("reserve allocates storage")
            .push(value)?;
        Ok(())
    }

    /// Remove and return the final value.
    pub fn pop(&mut self) -> Option<T> {
        self.storage.as_mut().and_then(CageAllocation::pop)
    }
    /// Return an initialized element by index.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.storage.as_ref().and_then(|s| s.get(index))
    }
    /// Mutably borrow an initialized element by index.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.storage.as_mut().and_then(|s| s.get_mut(index))
    }
    /// Borrow the initialized values as a native slice.
    pub fn as_slice(&self) -> &[T] {
        self.storage
            .as_ref()
            .map_or(&[], |storage| storage.as_slice())
    }
    /// Mutably borrow the initialized values as a native slice.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.storage
            .as_mut()
            .map_or(&mut [], |storage| storage.as_mut_slice())
    }
    /// Return an iterator over initialized values.
    pub fn iter(&self) -> slice::Iter<'_, T> {
        self.as_slice().iter()
    }
    /// Drop every initialized value after `len`.
    pub fn truncate(&mut self, len: usize) {
        if let Some(storage) = &mut self.storage {
            storage.truncate(len);
        }
    }
    /// Drop all values while retaining capacity.
    pub fn clear(&mut self) {
        self.truncate(0);
    }

    /// Retain only the elements selected by `keep`.
    ///
    /// The predicate runs once for each element, in vector order. If it
    /// panics, the vector remains valid and contains the retained prefix of
    /// already-visited elements followed by all elements not yet visited.
    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&T) -> bool,
    {
        let len = self.len();
        if len == 0 {
            return;
        }

        if !core::mem::needs_drop::<T>() {
            let data = self.as_mut_slice().as_mut_ptr();
            let mut guard = RetainNoDropGuard {
                vector: self,
                data,
                original_len: len,
                read: 0,
                write: 0,
                armed: true,
            };

            while guard.read < len {
                let read = guard.read;
                // SAFETY: `read` is within the original initialized prefix.
                let should_keep = unsafe { keep(&*data.add(read)) };
                if should_keep {
                    if guard.write != read {
                        // SAFETY: the source is initialized and the destination
                        // is a vacancy in the compacted prefix. `copy` is valid
                        // for overlapping slots; the stale source is outside
                        // the final initialized prefix.
                        unsafe { ptr::copy(data.add(read), data.add(guard.write), 1) };
                    }
                    guard.write += 1;
                }
                guard.read += 1;
            }

            guard.finish();
            return;
        }

        // Stage values in a native scratch vector so predicates and rejected
        // value destructors run in original left-to-right order while each
        // value is outside the compact vector's initialized prefix.
        let mut pending = std::vec::Vec::with_capacity(len);
        while let Some(value) = self.pop() {
            pending.push(value);
        }
        pending.reverse();
        let mut guard = RetainDropGuard {
            vector: self,
            pending,
            armed: true,
        };
        // `Vec::retain` provides the same forward predicate/drop ordering and
        // repairs its initialized prefix if either the predicate or a value
        // destructor unwinds. The outer guard moves its surviving values back.
        guard.pending.retain(&mut keep);

        let count = guard.pending.len();
        let written = guard
            .vector
            .storage
            .as_mut()
            .expect("nonempty retain preserves its allocation")
            .extend_from_iter(&mut guard.pending.drain(..), count)
            .expect("retained values fit in the original vector capacity");
        debug_assert_eq!(written, count);
        guard.armed = false;
    }

    /// Reduce capacity to the current length.
    pub fn shrink_to_fit(&mut self) -> Result<()> {
        let Some(storage) = &mut self.storage else {
            return Ok(());
        };
        let len = storage.len();
        if len == 0 {
            self.storage = None;
            return Ok(());
        }
        if storage.try_resize(len)? {
            return Ok(());
        }
        let mut replacement = CompactRuntime::alloc_owned_slice::<T>(len)?;
        storage.move_into(&mut replacement)?;
        self.storage = Some(replacement);
        Ok(())
    }

    /// Build a compact vector from an iterator.
    pub fn try_from_iter<I: IntoIterator<Item = T>>(iter: I) -> Result<Self> {
        let mut values = Self::new();
        values.try_extend(iter)?;
        Ok(values)
    }

    /// Append values from an iterator.
    pub fn try_extend<I: IntoIterator<Item = T>>(&mut self, iter: I) -> Result<()> {
        let mut iterator = iter.into_iter();
        let (lower, _) = iterator.size_hint();
        self.reserve(lower)?;

        // Fill the current allocation in batches. If it is full, pull one
        // pending value before growing so exact-capacity iterators do not cause
        // an unnecessary allocation at the end.
        let mut pending = None;
        loop {
            let (len, capacity) = self
                .storage
                .as_ref()
                .map_or((0, 0), CageAllocation::len_capacity);
            if len == capacity {
                pending = iterator.next();
                let Some(_) = pending else {
                    break;
                };
                self.reserve(1)?;
            }

            let available = self.storage.as_ref().map_or(0, |storage| {
                let (len, capacity) = storage.len_capacity();
                capacity - len
            });
            let written = {
                let storage = self.storage.as_mut().expect("reserve allocates storage");
                if let Some(value) = pending.take() {
                    let first = Some(value).into_iter();
                    let mut batch = first.chain(iterator.by_ref());
                    storage.extend_from_iter(&mut batch, available)?
                } else {
                    storage.extend_from_iter(&mut iterator, available)?
                }
            };
            if written < available {
                break;
            }
        }
        Ok(())
    }

    /// Append a copied slice through one reserve and one resolved cage view.
    pub fn try_extend_copy(&mut self, values: &[T]) -> Result<()>
    where
        T: Copy,
    {
        if values.is_empty() {
            return Ok(());
        }
        self.reserve(values.len())?;
        self.storage
            .as_mut()
            .expect("reserve allocates storage")
            .extend_copy(values)?;
        Ok(())
    }

    /// Append values from a fallible source in batches. If the source returns
    /// an error, its error is saved and the successfully appended prefix stays
    /// initialized and owned by this vector.
    #[doc(hidden)]
    pub fn try_extend_fallible<E>(
        &mut self,
        lower_bound: usize,
        mut next: impl FnMut() -> core::result::Result<Option<T>, E>,
        source_error: &mut Option<E>,
    ) -> Result<()> {
        self.reserve(lower_bound)?;
        let mut pending = None;
        loop {
            if source_error.is_some() {
                break;
            }
            let (len, capacity) = self
                .storage
                .as_ref()
                .map_or((0, 0), CageAllocation::len_capacity);
            if len == capacity {
                pending = match next() {
                    Ok(value) => value,
                    Err(error) => {
                        *source_error = Some(error);
                        break;
                    }
                };
                if pending.is_none() {
                    break;
                }
                self.reserve(1)?;
            }

            let available = self.storage.as_ref().map_or(0, |storage| {
                let (len, capacity) = storage.len_capacity();
                capacity - len
            });
            let written = {
                let storage = self.storage.as_mut().expect("reserve allocates storage");
                if let Some(value) = pending.take() {
                    let mut first = Some(value);
                    storage.extend_from_fallible_fn(
                        available,
                        || match first.take() {
                            Some(value) => Ok(Some(value)),
                            None => next(),
                        },
                        source_error,
                    )?
                } else {
                    storage.extend_from_fallible_fn(available, &mut next, source_error)?
                }
            };
            if source_error.is_some() || written < available {
                break;
            }
        }
        Ok(())
    }

    /// Clone all values into a new cage allocation.
    pub fn try_clone(&self) -> Result<Self>
    where
        T: Clone,
    {
        let mut cloned = Self::new();
        let values = self.as_slice();
        cloned.try_extend(values.iter().cloned())?;
        Ok(cloned)
    }

    /// Clone all values when `T` is copyable using a single bulk copy.
    pub fn try_clone_copy(&self) -> Result<Self>
    where
        T: Copy,
    {
        let values = self.as_slice();
        let mut cloned = Self::with_capacity(values.len())?;
        cloned.try_extend_copy(values)?;
        Ok(cloned)
    }
}

struct RetainNoDropGuard<T: CompactValue> {
    vector: *mut CompactVec<T>,
    data: *mut T,
    original_len: usize,
    read: usize,
    write: usize,
    armed: bool,
}

impl<T: CompactValue> RetainNoDropGuard<T> {
    fn finish(&mut self) {
        if !self.armed {
            return;
        }
        // SAFETY: the guard is created from the vector's exclusive borrow and
        // the `needs_drop == false` branch ensures truncating stale source
        // copies cannot skip user destructors.
        let vector = unsafe { &mut *self.vector };
        let remaining = self.original_len - self.read;
        // SAFETY: the unprocessed tail is initialized, `write <= read`, and
        // `ptr::copy` supports the overlapping compacting move.
        unsafe {
            ptr::copy(
                self.data.add(self.read),
                self.data.add(self.write),
                remaining,
            )
        };
        let final_len = self.write + remaining;
        vector
            .storage
            .as_mut()
            .expect("retaining a nonempty vector preserves its allocation")
            .truncate(final_len);
        self.armed = false;
    }
}

impl<T: CompactValue> Drop for RetainNoDropGuard<T> {
    fn drop(&mut self) {
        self.finish();
    }
}

struct RetainDropGuard<'a, T: CompactValue> {
    vector: &'a mut CompactVec<T>,
    pending: std::vec::Vec<T>,
    armed: bool,
}

impl<T: CompactValue> Drop for RetainDropGuard<'_, T> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let count = self.pending.len();
        if count == 0 {
            return;
        }
        let written = self
            .vector
            .storage
            .as_mut()
            .expect("retain preserves its backing allocation")
            .extend_from_iter(&mut self.pending.drain(..), count)
            .expect("retained values fit in the original vector capacity");
        debug_assert_eq!(written, count);
    }
}

impl<T: CompactValue> Deref for CompactVec<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}
impl<T: CompactValue> DerefMut for CompactVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}
impl<T: CompactValue> AsRef<[T]> for CompactVec<T> {
    fn as_ref(&self) -> &[T] {
        self
    }
}
impl<T: CompactValue> AsMut<[T]> for CompactVec<T> {
    fn as_mut(&mut self) -> &mut [T] {
        self
    }
}
impl<T: CompactValue> Borrow<[T]> for CompactVec<T> {
    fn borrow(&self) -> &[T] {
        self
    }
}
impl<T: CompactValue> BorrowMut<[T]> for CompactVec<T> {
    fn borrow_mut(&mut self) -> &mut [T] {
        self
    }
}
impl<T: CompactValue> Default for CompactVec<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T: CompactValue, I> Index<I> for CompactVec<T>
where
    [T]: Index<I>,
{
    type Output = <[T] as Index<I>>::Output;
    fn index(&self, index: I) -> &Self::Output {
        <[T] as Index<I>>::index(self.as_slice(), index)
    }
}
impl<T: CompactValue, I> IndexMut<I> for CompactVec<T>
where
    [T]: IndexMut<I>,
{
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        <[T] as IndexMut<I>>::index_mut(self.as_mut_slice(), index)
    }
}
impl<'a, T: CompactValue> IntoIterator for &'a CompactVec<T> {
    type Item = &'a T;
    type IntoIter = slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<'a, T: CompactValue> IntoIterator for &'a mut CompactVec<T> {
    type Item = &'a mut T;
    type IntoIter = slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.as_mut_slice().iter_mut()
    }
}
impl<T: CompactValue + fmt::Debug> fmt::Debug for CompactVec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
impl<T: CompactValue + PartialEq> PartialEq for CompactVec<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}
impl<T: CompactValue + Eq> Eq for CompactVec<T> {}
impl<T: CompactValue + PartialOrd> PartialOrd for CompactVec<T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        self.as_slice().partial_cmp(other.as_slice())
    }
}
impl<T: CompactValue + Ord> Ord for CompactVec<T> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}
impl<T: CompactValue + Hash> Hash for CompactVec<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

// SAFETY: the wrapper only owns a cage allocation and preserves T's move/drop contract.
unsafe impl<T: CompactValue> CompactValue for CompactVec<T> {}

impl<T: CompactValue + Clone> crate::TryClone for CompactVec<T> {
    type Cloned = Self;
    fn try_clone(&self) -> Result<Self::Cloned> {
        CompactVec::try_clone(self)
    }
}
impl<T: CompactValue> crate::TryExtend<T> for CompactVec<T> {
    fn try_extend<I: IntoIterator<Item = T>>(&mut self, iter: I) -> Result<()> {
        CompactVec::try_extend(self, iter)
    }
}
impl<T: CompactValue> crate::TryFromIterator<T> for CompactVec<T> {
    fn try_from_iter<I: IntoIterator<Item = T>>(iter: I) -> Result<Self> {
        CompactVec::try_from_iter(iter)
    }
}
