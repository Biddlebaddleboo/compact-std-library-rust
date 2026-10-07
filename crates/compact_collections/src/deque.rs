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

    fn physical_index(head: usize, logical: usize, capacity: usize) -> usize {
        debug_assert!(capacity > 0);
        debug_assert!(head < capacity);
        let until_wrap = capacity - head;
        if logical >= until_wrap {
            logical - until_wrap
        } else {
            head + logical
        }
    }

    /// Return the front value.
    pub fn front(&self) -> Option<&T> {
        if self.len == 0 {
            None
        } else {
            let slots = self
                .storage
                .as_ref()
                .expect("nonempty deque has storage")
                .uninit_capacity();
            // SAFETY: every logical deque slot contains an initialized nested value.
            Some(unsafe {
                slots[self.head as usize]
                    .assume_init_ref()
                    .assume_init_ref()
            })
        }
    }
    /// Mutably borrow the front value.
    pub fn front_mut(&mut self) -> Option<&mut T> {
        if self.len == 0 {
            None
        } else {
            let slots = self
                .storage
                .as_mut()
                .expect("nonempty deque has storage")
                .uninit_capacity_mut();
            // SAFETY: every logical deque slot contains one uniquely borrowed value.
            Some(unsafe {
                slots[self.head as usize]
                    .assume_init_mut()
                    .assume_init_mut()
            })
        }
    }
    /// Return the back value.
    pub fn back(&self) -> Option<&T> {
        if self.len == 0 {
            None
        } else {
            let slots = self
                .storage
                .as_ref()
                .expect("nonempty deque has storage")
                .uninit_capacity();
            let physical = Self::physical_index(self.head as usize, self.len() - 1, slots.len());
            // SAFETY: every logical deque slot contains an initialized nested value.
            Some(unsafe { slots[physical].assume_init_ref().assume_init_ref() })
        }
    }
    /// Mutably borrow the back value.
    pub fn back_mut(&mut self) -> Option<&mut T> {
        if self.len == 0 {
            None
        } else {
            let head = self.head as usize;
            let logical = self.len() - 1;
            let slots = self
                .storage
                .as_mut()
                .expect("nonempty deque has storage")
                .uninit_capacity_mut();
            let at = Self::physical_index(head, logical, slots.len());
            // SAFETY: every logical deque slot contains one uniquely borrowed value.
            Some(unsafe { slots[at].assume_init_mut().assume_init_mut() })
        }
    }
    /// Return a logical element by index.
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len() {
            None
        } else {
            let slots = self
                .storage
                .as_ref()
                .expect("nonempty deque has storage")
                .uninit_capacity();
            let physical = Self::physical_index(self.head as usize, index, slots.len());
            // SAFETY: every logical deque slot contains an initialized nested value.
            Some(unsafe { slots[physical].assume_init_ref().assume_init_ref() })
        }
    }
    /// Mutably borrow a logical element by index.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len() {
            None
        } else {
            let head = self.head as usize;
            let slots = self
                .storage
                .as_mut()
                .expect("nonempty deque has storage")
                .uninit_capacity_mut();
            let at = Self::physical_index(head, index, slots.len());
            // SAFETY: the deque is exclusively borrowed and this logical slot is unique.
            Some(unsafe { slots[at].assume_init_mut().assume_init_mut() })
        }
    }
    /// Append a value at the back.
    pub fn push_back(&mut self, value: T) -> Result<()> {
        self.reserve(1)?;
        let head = self.head as usize;
        let logical = self.len();
        let slots = self
            .storage
            .as_mut()
            .expect("reserve allocates storage")
            .uninit_capacity_mut();
        let at = Self::physical_index(head, logical, slots.len());
        slots[at].write(MaybeUninit::new(value));
        self.len += 1;
        Ok(())
    }
    /// Append a value at the front.
    pub fn push_front(&mut self, value: T) -> Result<()> {
        self.reserve(1)?;
        let slots = self
            .storage
            .as_mut()
            .expect("reserve allocates storage")
            .uninit_capacity_mut();
        let capacity = slots.len();
        self.head = if self.len == 0 {
            0
        } else if self.head == 0 {
            (capacity - 1) as u32
        } else {
            self.head - 1
        };
        slots[self.head as usize].write(MaybeUninit::new(value));
        self.len += 1;
        Ok(())
    }
    /// Remove and return the front value.
    pub fn pop_front(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let at = self.head as usize;
        let (value, capacity) = {
            let slots = self
                .storage
                .as_mut()
                .expect("nonempty deque has storage")
                .uninit_capacity_mut();
            let capacity = slots.len();
            // SAFETY: the front slot is initialized and removed exactly once.
            (
                unsafe { slots[at].assume_init_read().assume_init_read() },
                capacity,
            )
        };
        self.head = if self.len == 1 || at + 1 == capacity {
            0
        } else {
            self.head + 1
        };
        self.len -= 1;
        Some(value)
    }
    /// Remove and return the back value.
    pub fn pop_back(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let head = self.head as usize;
        let logical = self.len() - 1;
        let value = {
            let slots = self
                .storage
                .as_mut()
                .expect("nonempty deque has storage")
                .uninit_capacity_mut();
            let at = Self::physical_index(head, logical, slots.len());
            // SAFETY: the back slot is initialized and removed exactly once.
            unsafe { slots[at].assume_init_read().assume_init_read() }
        };
        self.len -= 1;
        if self.len == 0 {
            self.head = 0;
        }
        Some(value)
    }
    /// Return an iterator over logical order.
    pub fn iter(&self) -> CompactVecDequeIter<'_, T> {
        let slots = self
            .storage
            .as_ref()
            .map_or(&[][..], CageAllocation::uninit_capacity);
        let capacity = slots.len();
        let head = self.head as usize;
        let len = self.len();
        let back_index = if len == 0 {
            head
        } else {
            Self::physical_index(head, len - 1, capacity)
        };
        CompactVecDequeIter {
            slots,
            capacity,
            front_index: head,
            back_index,
            remaining: len,
        }
    }
    /// Ensure room for at least `additional` values.
    pub fn reserve(&mut self, additional: usize) -> Result<()> {
        let required = (self.len as usize)
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        let required =
            u32::try_from(required).map_err(|_| CollectionError::CapacityOverflow)? as usize;
        let old_capacity = self
            .storage
            .as_ref()
            .map_or(0, |storage| storage.capacity());
        if required <= old_capacity {
            return Ok(());
        }
        let cap = required.max(old_capacity.saturating_mul(2).max(4));
        let mut replacement = CompactRuntime::alloc_owned_slice::<MaybeUninit<T>>(cap)?;
        let slots = replacement.uninit_capacity_mut();
        // Allocate first. Moving values below cannot fail.
        let old_len = self.len();
        if old_len != 0 {
            let head = self.head as usize;
            let old_slots = self
                .storage
                .as_mut()
                .expect("nonempty deque has storage")
                .uninit_capacity_mut();
            let old_capacity = old_slots.len();
            for (index, slot) in slots.iter_mut().take(old_len).enumerate() {
                let physical = Self::physical_index(head, index, old_capacity);
                // SAFETY: these logical slots are initialized and each is moved once.
                let value = unsafe { old_slots[physical].assume_init_read().assume_init_read() };
                slot.write(MaybeUninit::new(value));
            }
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
        let start = self.head as usize;
        let len = self.len();
        let slots = self
            .storage
            .as_mut()
            .expect("nonempty deque has storage")
            .uninit_capacity_mut();
        if start + len <= slots.len() {
            let end = start + len;
            let ptr = slots[start..end]
                .as_mut_ptr()
                .cast::<MaybeUninit<T>>()
                .cast::<T>();
            // SAFETY: this is the initialized logical range in storage.
            return Ok(unsafe { slice::from_raw_parts_mut(ptr, len) });
        }
        let mut replacement = CompactRuntime::alloc_owned_slice::<MaybeUninit<T>>(len)?;
        let slots = replacement.uninit_capacity_mut();
        let old_slots = self
            .storage
            .as_mut()
            .expect("nonempty deque has storage")
            .uninit_capacity_mut();
        let old_capacity = old_slots.len();
        for (index, slot) in slots.iter_mut().take(len).enumerate() {
            let physical = Self::physical_index(start, index, old_capacity);
            // SAFETY: these logical slots are initialized and each is moved once.
            let value = unsafe { old_slots[physical].assume_init_read().assume_init_read() };
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
    slots: &'a [MaybeUninit<MaybeUninit<T>>],
    capacity: usize,
    front_index: usize,
    back_index: usize,
    remaining: usize,
}
impl<'a, T: CompactValue> Iterator for CompactVecDequeIter<'a, T> {
    type Item = &'a T;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let index = self.front_index;
        self.remaining -= 1;
        if self.remaining != 0 {
            self.front_index = if index + 1 == self.capacity {
                0
            } else {
                index + 1
            };
        }
        // SAFETY: the iterator borrows the deque, and every remaining logical
        // slot contains an initialized value. Each front position is visited once.
        Some(unsafe { self.slots[index].assume_init_ref().assume_init_ref() })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl<T: CompactValue> DoubleEndedIterator for CompactVecDequeIter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let index = self.back_index;
        self.remaining -= 1;
        if self.remaining != 0 {
            self.back_index = if index == 0 {
                self.capacity - 1
            } else {
                index - 1
            };
        }
        // SAFETY: the iterator borrows the deque, and every remaining logical
        // slot contains an initialized value. Each back position is visited once.
        Some(unsafe { self.slots[index].assume_init_ref().assume_init_ref() })
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
