//! Cage-backed double-ended queue and bounded ring.

use compact_backend_std::{CageAllocation, CompactRuntime};
use compact_core::{CompactValue, Error};
use core::mem::MaybeUninit;
use core::slice;

use crate::{CollectionError, Result};

/// A compact double-ended queue with one owner, head, and length.
pub struct CompactVecDeque<T: CompactValue> {
    storage: Option<CageAllocation<MaybeUninit<T>>>,
    head: u32,
    len: u32,
}

impl<T: CompactValue> CompactVecDeque<T> {
    /// Construct an empty deque.
    pub const fn new() -> Self {
        Self {
            storage: None,
            head: 0,
            len: 0,
        }
    }
    /// Allocate a deque with at least `capacity` slots.
    pub fn with_capacity(capacity: usize) -> Result<Self> {
        let capacity = u32::try_from(capacity).map_err(|_| CollectionError::CapacityOverflow)?;
        let storage = if capacity == 0 {
            None
        } else {
            Some(CompactRuntime::alloc_owned_slice(capacity as usize)?)
        };
        Ok(Self {
            storage,
            head: 0,
            len: 0,
        })
    }
    /// Return the number of stored values.
    pub const fn len(&self) -> usize {
        self.len as usize
    }
    /// Return allocated slot capacity.
    pub fn capacity(&self) -> usize {
        self.storage.as_ref().map_or(0, |s| s.capacity())
    }
    /// Return whether the deque is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn physical(&self, logical: usize) -> usize {
        ((self.head as u64 + logical as u64) % self.capacity() as u64) as usize
    }
    fn value_at(&self, physical: usize) -> &T {
        let slot = &self
            .storage
            .as_ref()
            .expect("nonempty deque has storage")
            .uninit_capacity()[physical];
        // SAFETY: every logical deque slot contains an initialized nested value.
        unsafe { slot.assume_init_ref().assume_init_ref() }
    }
    fn value_at_mut(&mut self, physical: usize) -> &mut T {
        let slot = &mut self
            .storage
            .as_mut()
            .expect("nonempty deque has storage")
            .uninit_capacity_mut()[physical];
        // SAFETY: every logical deque slot contains one uniquely borrowed value.
        unsafe { slot.assume_init_mut().assume_init_mut() }
    }
    fn write_at(&mut self, physical: usize, value: T) {
        let slot = &mut self
            .storage
            .as_mut()
            .expect("deque storage exists")
            .uninit_capacity_mut()[physical];
        slot.write(MaybeUninit::new(value));
    }
    fn take_at(&mut self, physical: usize) -> T {
        let slot = &mut self
            .storage
            .as_mut()
            .expect("deque storage exists")
            .uninit_capacity_mut()[physical];
        // SAFETY: slot was initialized once and is removed from the logical range before return.
        unsafe { slot.assume_init_read().assume_init_read() }
    }

    /// Return the front value.
    pub fn front(&self) -> Option<&T> {
        if self.len == 0 {
            None
        } else {
            Some(self.value_at(self.head as usize))
        }
    }
    /// Mutably borrow the front value.
    pub fn front_mut(&mut self) -> Option<&mut T> {
        if self.len == 0 {
            None
        } else {
            Some(self.value_at_mut(self.head as usize))
        }
    }
    /// Return the back value.
    pub fn back(&self) -> Option<&T> {
        if self.len == 0 {
            None
        } else {
            Some(self.value_at(self.physical(self.len() - 1)))
        }
    }
    /// Mutably borrow the back value.
    pub fn back_mut(&mut self) -> Option<&mut T> {
        if self.len == 0 {
            None
        } else {
            let at = self.physical(self.len() - 1);
            Some(self.value_at_mut(at))
        }
    }
    /// Return a logical element by index.
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len() {
            None
        } else {
            Some(self.value_at(self.physical(index)))
        }
    }
    /// Mutably borrow a logical element by index.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len() {
            None
        } else {
            let at = self.physical(index);
            Some(self.value_at_mut(at))
        }
    }
    /// Append a value at the back.
    pub fn push_back(&mut self, value: T) -> Result<()> {
        self.reserve(1)?;
        let at = self.physical(self.len());
        self.write_at(at, value);
        self.len += 1;
        Ok(())
    }
    /// Append a value at the front.
    pub fn push_front(&mut self, value: T) -> Result<()> {
        self.reserve(1)?;
        self.head = if self.len == 0 {
            0
        } else {
            ((self.head as u64 + self.capacity() as u64 - 1) % self.capacity() as u64) as u32
        };
        self.write_at(self.head as usize, value);
        self.len += 1;
        Ok(())
    }
    /// Remove and return the front value.
    pub fn pop_front(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let at = self.head as usize;
        self.head = if self.len == 1 {
            0
        } else {
            ((self.head as u64 + 1) % self.capacity() as u64) as u32
        };
        self.len -= 1;
        Some(self.take_at(at))
    }
    /// Remove and return the back value.
    pub fn pop_back(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let at = self.physical(self.len() - 1);
        self.len -= 1;
        if self.len == 0 {
            self.head = 0;
        }
        Some(self.take_at(at))
    }
    /// Return an iterator over logical order.
    pub fn iter(&self) -> CompactVecDequeIter<'_, T> {
        CompactVecDequeIter {
            deque: self,
            front: 0,
            back: self.len(),
        }
    }
    /// Ensure room for at least `additional` values.
    pub fn reserve(&mut self, additional: usize) -> Result<()> {
        let required = self
            .len()
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        let required =
            u32::try_from(required).map_err(|_| CollectionError::CapacityOverflow)? as usize;
        if required <= self.capacity() {
            return Ok(());
        }
        let cap = required.max(self.capacity().saturating_mul(2).max(4));
        let mut replacement = CompactRuntime::alloc_owned_slice::<MaybeUninit<T>>(cap)?;
        let slots = replacement.uninit_capacity_mut();
        // Allocate first. Moving values below cannot fail.
        let old_len = self.len();
        for (index, slot) in slots.iter_mut().take(old_len).enumerate() {
            let value = self.take_at(self.physical(index));
            slot.write(MaybeUninit::new(value));
        }
        self.len = 0;
        self.storage = Some(replacement);
        self.head = 0;
        self.len = old_len as u32;
        Ok(())
    }
    /// Drop values after the requested logical length.
    pub fn truncate(&mut self, len: usize) {
        while self.len() > len {
            drop(self.pop_back());
        }
    }
    /// Drop all values while retaining capacity.
    pub fn clear(&mut self) {
        self.truncate(0);
    }
    /// Return the values as one contiguous mutable slice, rotating if needed.
    pub fn make_contiguous(&mut self) -> Result<&mut [T]> {
        if self.len == 0 {
            return Ok(&mut []);
        }
        if self.head as usize + self.len() <= self.capacity() {
            let start = self.head as usize;
            let end = start + self.len();
            let slots = self.storage.as_mut().unwrap().uninit_capacity_mut();
            let ptr = slots[start..end]
                .as_mut_ptr()
                .cast::<MaybeUninit<T>>()
                .cast::<T>();
            // SAFETY: this is the initialized logical range in storage.
            return Ok(unsafe { slice::from_raw_parts_mut(ptr, self.len()) });
        }
        let mut replacement = CompactRuntime::alloc_owned_slice::<MaybeUninit<T>>(self.len())?;
        let len = self.len();
        let slots = replacement.uninit_capacity_mut();
        for (index, slot) in slots.iter_mut().take(len).enumerate() {
            let value = self.take_at(self.physical(index));
            slot.write(MaybeUninit::new(value));
        }
        self.storage = Some(replacement);
        self.head = 0;
        let outer = self.storage.as_mut().unwrap().uninit_capacity_mut();
        let ptr = outer.as_mut_ptr().cast::<MaybeUninit<T>>().cast::<T>();
        // SAFETY: values were moved into the first `len` contiguous slots.
        Ok(unsafe { slice::from_raw_parts_mut(ptr, len) })
    }
}

impl<T: CompactValue> Default for CompactVecDeque<T> {
    fn default() -> Self {
        Self::new()
    }
}
struct DequeDropGuard<T: CompactValue> {
    deque: *mut CompactVecDeque<T>,
    armed: bool,
}
impl<T: CompactValue> Drop for DequeDropGuard<T> {
    fn drop(&mut self) {
        if self.armed {
            // SAFETY: created from an exclusive borrow during the deque destructor.
            let deque = unsafe { &mut *self.deque };
            while let Some(value) = deque.pop_front() {
                drop(value);
            }
        }
    }
}
impl<T: CompactValue> Drop for CompactVecDeque<T> {
    fn drop(&mut self) {
        let mut guard = DequeDropGuard {
            deque: self,
            armed: true,
        };
        while let Some(value) = self.pop_front() {
            drop(value);
        }
        guard.armed = false;
    }
}
// SAFETY: the wrapper owns only cage offsets and Rust scalar metadata.
unsafe impl<T: CompactValue> CompactValue for CompactVecDeque<T> {}

/// Double-ended iterator over a compact deque.
pub struct CompactVecDequeIter<'a, T: CompactValue> {
    deque: &'a CompactVecDeque<T>,
    front: usize,
    back: usize,
}
impl<'a, T: CompactValue> Iterator for CompactVecDequeIter<'a, T> {
    type Item = &'a T;
    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        let index = self.front;
        self.front += 1;
        self.deque.get(index)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.back - self.front;
        (n, Some(n))
    }
}
impl<T: CompactValue> DoubleEndedIterator for CompactVecDequeIter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        self.deque.get(self.back)
    }
}
impl<T: CompactValue> ExactSizeIterator for CompactVecDequeIter<'_, T> {}
impl<T: CompactValue> core::iter::FusedIterator for CompactVecDequeIter<'_, T> {}

/// Fixed-capacity FIFO ring that evicts the oldest item when full.
pub struct CompactRing<T: CompactValue> {
    values: CompactVecDeque<T>,
    maximum_len: u32,
}
impl<T: CompactValue> CompactRing<T> {
    /// Create a bounded ring.
    pub fn with_capacity(maximum_len: usize) -> Result<Self> {
        let maximum_len =
            u32::try_from(maximum_len).map_err(|_| CollectionError::CapacityOverflow)?;
        Ok(Self {
            values: CompactVecDeque::with_capacity(maximum_len as usize)?,
            maximum_len,
        })
    }
    /// Return the number of values.
    pub fn len(&self) -> usize {
        self.values.len()
    }
    /// Return the fixed capacity.
    pub const fn capacity(&self) -> usize {
        self.maximum_len as usize
    }
    /// Return whether the ring is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    /// Return the oldest value.
    pub fn front(&self) -> Option<&T> {
        self.values.front()
    }
    /// Append a value, dropping the oldest value when full.
    pub fn push_back(&mut self, value: T) -> Result<()> {
        if self.maximum_len == 0 {
            return Err(CollectionError::Core(Error::AllocationExhausted));
        }
        if self.len() == self.capacity() {
            drop(self.values.pop_front());
        }
        self.values.push_back(value)
    }
    /// Remove the oldest value.
    pub fn pop_front(&mut self) -> Option<T> {
        self.values.pop_front()
    }
    /// Iterate over values oldest to newest.
    pub fn iter(&self) -> CompactVecDequeIter<'_, T> {
        self.values.iter()
    }
    /// Clear the ring.
    pub fn clear(&mut self) {
        self.values.clear();
    }
}
// SAFETY: the ring owns a compact deque and a scalar capacity.
unsafe impl<T: CompactValue> CompactValue for CompactRing<T> {}
