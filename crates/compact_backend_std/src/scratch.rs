//! Temporary allocations carved from one process-cage block.

use crate::{CageAllocation, CompactRuntime};
use compact_core::{checked_align_up, CompactValue, Error, Result};
use core::mem::{align_of, size_of};
use core::ptr;
use core::slice;

/// A bump-allocated temporary region inside the process cage.
///
/// Scratch storage supports byte slices and `Copy` values. Values with drop
/// glue belong in ordinary cage owners so they are destroyed exactly once.
pub struct ScratchRegion {
    storage: CageAllocation<u64>,
    cursor: u32,
    capacity: u32,
}

impl ScratchRegion {
    /// Reserve one cage block with the requested byte capacity.
    pub fn new(capacity: usize) -> Result<Self> {
        let words = capacity
            .checked_add(size_of::<u64>() - 1)
            .ok_or(Error::OffsetOverflow)?
            / size_of::<u64>();
        let rounded_capacity = words
            .checked_mul(size_of::<u64>())
            .ok_or(Error::OffsetOverflow)?;
        let capacity = u32::try_from(rounded_capacity).map_err(|_| Error::OffsetOverflow)?;
        let storage = CompactRuntime::alloc_owned_slice::<u64>(words)?;
        Ok(Self {
            storage,
            cursor: 0,
            capacity,
        })
    }

    /// Return bytes reserved for this region.
    pub const fn capacity(&self) -> usize {
        self.capacity as usize
    }
    /// Return bytes already handed out.
    pub const fn used(&self) -> usize {
        self.cursor as usize
    }
    /// Return whether no scratch bytes have been allocated.
    pub const fn is_empty(&self) -> bool {
        self.cursor == 0
    }

    /// Allocate an aligned, zero-filled temporary byte slice.
    pub fn alloc_bytes(&mut self, len: usize) -> Result<&mut [u8]> {
        let len = u32::try_from(len).map_err(|_| Error::OffsetOverflow)?;
        let start = self.cursor;
        let end = start.checked_add(len).ok_or(Error::OffsetOverflow)?;
        if end > self.capacity {
            return Err(Error::AllocationExhausted);
        }
        self.cursor = end;
        let slots = self.storage.uninit_capacity_mut();
        let base = slots.as_mut_ptr().cast::<u8>();
        // SAFETY: the selected range is within the u64 block; writing bytes is
        // valid for MaybeUninit<u64> storage, and zeroing makes the returned
        // u8 slice fully initialized.
        unsafe {
            let range = base.add(start as usize);
            ptr::write_bytes(range, 0, len as usize);
            Ok(slice::from_raw_parts_mut(range, len as usize))
        }
    }

    /// Allocate and initialize one aligned `Copy` value.
    pub fn alloc_value<T: Copy + CompactValue>(&mut self, value: T) -> Result<&mut T> {
        if align_of::<T>() > align_of::<u64>() {
            return Err(Error::AlignmentError);
        }
        let start = checked_align_up(self.cursor as usize, align_of::<T>())?;
        let end = start
            .checked_add(size_of::<T>())
            .ok_or(Error::OffsetOverflow)?;
        if end > self.capacity as usize {
            return Err(Error::AllocationExhausted);
        }
        self.cursor = u32::try_from(end).map_err(|_| Error::OffsetOverflow)?;
        let slots = self.storage.uninit_capacity_mut();
        let pointer = slots.as_mut_ptr().cast::<u8>();
        // SAFETY: `start` is aligned for T, the entire object fits in the
        // reserved block, and Copy + CompactValue excludes drop obligations.
        unsafe {
            let value_pointer = pointer.add(start).cast::<T>();
            value_pointer.write(value);
            Ok(&mut *value_pointer)
        }
    }
}
