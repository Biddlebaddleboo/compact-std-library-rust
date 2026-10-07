//! Inline-first compact small vector.

use compact_backend_std::{CageAllocation, CompactRuntime};
use compact_core::CompactValue;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};

use crate::{CollectionError, Result};

/// A compact vector that stores up to `N` values inline before promoting to the cage.
pub struct CompactSmallVec<T: CompactValue, const N: usize> {
    inline: [MaybeUninit<T>; N],
    inline_len: usize,
    heap: Option<CageAllocation<T>>,
}

impl<T: CompactValue, const N: usize> CompactSmallVec<T, N> {
    /// Construct an empty small vector.
    pub fn new() -> Self {
        Self {
            inline: core::array::from_fn(|_| MaybeUninit::uninit()),
            inline_len: 0,
            heap: None,
        }
    }
    /// Return the initialized element count.
    pub fn len(&self) -> usize {
        self.heap.as_ref().map_or(self.inline_len, |h| h.len())
    }
    /// Return available inline or cage capacity.
    pub fn capacity(&self) -> usize {
        self.heap.as_ref().map_or(N, |h| h.capacity())
    }
    /// Return whether storage is still inline.
    pub fn is_inline(&self) -> bool {
        self.heap.is_none()
    }
    /// Return whether the collection is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Ensure room for at least `additional` more values.
    pub fn reserve(&mut self, additional: usize) -> Result<()> {
        let required = self
            .len()
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        if required <= self.capacity() {
            return Ok(());
        }
        let capacity = required.max(self.capacity().saturating_mul(2).max(4));
        if let Some(heap) = &mut self.heap {
            if heap.try_resize(capacity)? {
                return Ok(());
            }
            let mut replacement = CompactRuntime::alloc_owned_slice::<T>(capacity)?;
            heap.move_into(&mut replacement)?;
            self.heap = Some(replacement);
            return Ok(());
        }
        let mut replacement = CompactRuntime::alloc_owned_slice::<T>(capacity)?;
        // SAFETY: exactly `inline_len` inline slots are initialized and become moved.
        unsafe {
            replacement.move_from_uninit_slice(self.inline.as_mut_ptr(), self.inline_len)?;
        }
        self.inline_len = 0;
        self.heap = Some(replacement);
        Ok(())
    }
    /// Append a value.
    pub fn push(&mut self, value: T) -> Result<()> {
        if self.heap.is_none() && self.inline_len < N {
            self.inline[self.inline_len].write(value);
            self.inline_len += 1;
            return Ok(());
        }
        self.reserve(1)?;
        self.heap
            .as_mut()
            .expect("reserve promotes storage")
            .push(value)?;
        Ok(())
    }
    /// Borrow all initialized values.
    pub fn as_slice(&self) -> &[T] {
        if let Some(heap) = &self.heap {
            return heap.as_slice();
        }
        // SAFETY: inline prefix is tracked by inline_len.
        unsafe { core::slice::from_raw_parts(self.inline.as_ptr().cast::<T>(), self.inline_len) }
    }
    /// Mutably borrow all initialized values.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        if let Some(heap) = &mut self.heap {
            return heap.as_mut_slice();
        }
        // SAFETY: inline prefix is uniquely borrowed and tracked by inline_len.
        unsafe {
            core::slice::from_raw_parts_mut(self.inline.as_mut_ptr().cast::<T>(), self.inline_len)
        }
    }
    /// Return an initialized value by index.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.as_slice().get(index)
    }
    /// Mutably borrow an initialized value by index.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.as_mut_slice().get_mut(index)
    }
    /// Remove and return the final value.
    pub fn pop(&mut self) -> Option<T> {
        if let Some(heap) = &mut self.heap {
            return heap.pop();
        }
        if self.inline_len == 0 {
            return None;
        }
        self.inline_len -= 1;
        // SAFETY: the former final slot was initialized and is now removed from the prefix.
        Some(unsafe { self.inline[self.inline_len].assume_init_read() })
    }
    /// Drop values after `len`.
    pub fn truncate(&mut self, len: usize) {
        if let Some(heap) = &mut self.heap {
            heap.truncate(len);
            return;
        }
        while self.inline_len > len {
            self.inline_len -= 1;
            // SAFETY: length is lowered before the destructor is called.
            unsafe { self.inline[self.inline_len].assume_init_drop() };
        }
    }
    /// Drop all values and keep current storage.
    pub fn clear(&mut self) {
        self.truncate(0);
    }
}

impl<T: CompactValue, const N: usize> Default for CompactSmallVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T: CompactValue, const N: usize> Deref for CompactSmallVec<T, N> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}
impl<T: CompactValue, const N: usize> DerefMut for CompactSmallVec<T, N> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}
impl<T: CompactValue, const N: usize> Drop for CompactSmallVec<T, N> {
    fn drop(&mut self) {
        let mut guard = SmallVecDropGuard {
            values: self,
            armed: true,
        };
        self.truncate(0);
        guard.armed = false;
    }
}

struct SmallVecDropGuard<T: CompactValue, const N: usize> {
    values: *mut CompactSmallVec<T, N>,
    armed: bool,
}
impl<T: CompactValue, const N: usize> Drop for SmallVecDropGuard<T, N> {
    fn drop(&mut self) {
        if self.armed {
            // SAFETY: the guard is created from the exclusive borrow in Drop
            // and is only used during unwinding to drop the remaining values.
            unsafe { (*self.values).truncate(0) };
        }
    }
}
// SAFETY: inline elements move with the wrapper and heap values are owned by a cage allocation.
unsafe impl<T: CompactValue, const N: usize> CompactValue for CompactSmallVec<T, N> {}
