//! Arena-backed circular queues and bounded logging rings.

use compact_core::{Arena, ArenaAllocation, CompactValue};
use core::mem::MaybeUninit;
use core::slice;

use crate::{CollectionError, Result};

/// A double-ended queue backed by one circular arena allocation.
///
/// Logical order is tracked by `head` and `len`; physical storage uses
/// uninitialized slots so wrapping does not impose a contiguous prefix.
pub struct CompactVecDeque<'arena, T: CompactValue> {
    storage: Option<ArenaAllocation<'arena, MaybeUninit<T>>>,
    head: usize,
    len: usize,
}

impl<'arena, T: CompactValue> CompactVecDeque<'arena, T> {
    /// Construct an empty deque without allocating arena storage.
    pub const fn new() -> Self {
        Self {
            storage: None,
            head: 0,
            len: 0,
        }
    }

    /// Construct an empty deque tied to `arena` without allocating storage.
    pub fn new_in(_arena: &Arena<'arena, '_>) -> Self {
        Self::new()
    }

    /// Construct a deque with room for at least `capacity` elements.
    pub fn with_capacity_in(capacity: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        let capacity = u32::try_from(capacity).map_err(|_| CollectionError::CapacityOverflow)?;
        let storage = if capacity == 0 {
            None
        } else {
            Some(allocate_slots(capacity as usize, arena)?)
        };
        Ok(Self {
            storage,
            head: 0,
            len: 0,
        })
    }

    /// Construct a deque with room for at least `capacity` elements.
    pub fn with_capacity(capacity: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        Self::with_capacity_in(capacity, arena)
    }

    /// Return the number of initialized elements.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return the number of element slots.
    pub fn capacity(&self) -> usize {
        self.storage.as_ref().map_or(0, ArenaAllocation::capacity)
    }

    /// Return whether the deque contains no elements.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Borrow the front element.
    pub fn front<'view>(&'view self, arena: &'view Arena<'arena, '_>) -> Result<Option<&'view T>> {
        self.get(0, arena)
    }

    /// Mutably borrow the front element.
    pub fn front_mut<'view>(
        &'view mut self,
        arena: &'view mut Arena<'arena, '_>,
    ) -> Result<Option<&'view mut T>> {
        self.get_mut(0, arena)
    }

    /// Borrow the back element.
    pub fn back<'view>(&'view self, arena: &'view Arena<'arena, '_>) -> Result<Option<&'view T>> {
        self.len
            .checked_sub(1)
            .map_or(Ok(None), |index| self.get(index, arena))
    }

    /// Mutably borrow the back element.
    pub fn back_mut<'view>(
        &'view mut self,
        arena: &'view mut Arena<'arena, '_>,
    ) -> Result<Option<&'view mut T>> {
        self.len
            .checked_sub(1)
            .map_or(Ok(None), |index| self.get_mut(index, arena))
    }

    /// Borrow the element at logical index `index`.
    pub fn get<'view>(
        &'view self,
        index: usize,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<Option<&'view T>> {
        if index >= self.len {
            return Ok(None);
        }
        let storage = self.storage.as_ref().expect("nonempty deque has storage");
        arena.validate_owned(storage)?;
        let physical = physical_index(self.head, index, storage.capacity());
        // SAFETY: `index < len` identifies an initialized logical entry; the
        // owner remains immutably borrowed for the returned reference.
        Ok(Some(unsafe {
            storage.as_slice()[physical].assume_init_ref()
        }))
    }

    /// Mutably borrow the element at logical index `index`.
    pub fn get_mut<'view>(
        &'view mut self,
        index: usize,
        arena: &'view mut Arena<'arena, '_>,
    ) -> Result<Option<&'view mut T>> {
        if index >= self.len {
            return Ok(None);
        }
        let storage = self.storage.as_mut().expect("nonempty deque has storage");
        arena.validate_owned(storage)?;
        let physical = physical_index(self.head, index, storage.capacity());
        // SAFETY: `index < len` identifies a uniquely initialized slot; the
        // mutable owner borrow excludes any other access to it.
        Ok(Some(unsafe {
            storage.as_mut_slice()[physical].assume_init_mut()
        }))
    }

    /// Append an element at the back, growing when needed.
    pub fn push_back_in(&mut self, value: T, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.reserve_in(1, arena)?;
        let storage = self.storage.as_mut().expect("reserve allocates one slot");
        arena.validate_owned(storage)?;
        let capacity = storage.capacity();
        let physical = physical_index(self.head, self.len, capacity);
        storage.as_mut_slice()[physical].write(value);
        self.len += 1;
        Ok(())
    }

    /// Append an element at the back, growing when needed.
    pub fn push_back(&mut self, value: T, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.push_back_in(value, arena)
    }

    /// Insert an element at the front, growing when needed.
    pub fn push_front_in(&mut self, value: T, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.reserve_in(1, arena)?;
        let storage = self.storage.as_mut().expect("reserve allocates one slot");
        arena.validate_owned(storage)?;
        let capacity = storage.capacity();
        let new_head = if self.head == 0 {
            capacity - 1
        } else {
            self.head - 1
        };
        storage.as_mut_slice()[new_head].write(value);
        self.head = new_head;
        self.len += 1;
        Ok(())
    }

    /// Insert an element at the front, growing when needed.
    pub fn push_front(&mut self, value: T, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.push_front_in(value, arena)
    }

    /// Remove and return the front element.
    pub fn pop_front_in(&mut self, arena: &Arena<'arena, '_>) -> Result<Option<T>> {
        if self.len == 0 {
            return Ok(None);
        }
        let storage = self.storage.as_mut().expect("nonempty deque has storage");
        arena.validate_owned(storage)?;
        Ok(Some(self.pop_front_unchecked()))
    }

    /// Remove and return the front element.
    pub fn pop_front(&mut self, arena: &Arena<'arena, '_>) -> Result<Option<T>> {
        self.pop_front_in(arena)
    }

    /// Remove and return the back element.
    pub fn pop_back_in(&mut self, arena: &Arena<'arena, '_>) -> Result<Option<T>> {
        if self.len == 0 {
            return Ok(None);
        }
        let storage = self.storage.as_mut().expect("nonempty deque has storage");
        arena.validate_owned(storage)?;
        Ok(Some(self.pop_back_unchecked()))
    }

    /// Remove and return the back element.
    pub fn pop_back(&mut self, arena: &Arena<'arena, '_>) -> Result<Option<T>> {
        self.pop_back_in(arena)
    }

    /// Return an iterator over elements in logical order.
    pub fn iter<'view>(
        &'view self,
        arena: &Arena<'arena, '_>,
    ) -> Result<CompactVecDequeIter<'view, T>> {
        let slots = if let Some(storage) = &self.storage {
            arena.validate_owned(storage)?;
            storage.as_slice()
        } else {
            &[]
        };
        Ok(CompactVecDequeIter {
            slots,
            head: self.head,
            capacity: self.capacity(),
            front: 0,
            back: self.len,
        })
    }

    /// Return a mutable iterator in logical order.
    pub fn iter_mut<'view>(
        &'view mut self,
        arena: &mut Arena<'arena, '_>,
    ) -> Result<slice::IterMut<'view, T>> {
        Ok(self.make_contiguous(arena)?.iter_mut())
    }

    /// Drop elements after `len` while retaining the allocation.
    pub fn truncate(&mut self, len: usize) {
        while self.len > len {
            let value = self.pop_back_unchecked();
            drop(value);
        }
    }

    /// Drop all elements while retaining the allocation.
    pub fn clear(&mut self) {
        self.truncate(0);
    }

    /// Ensure room for at least `additional` more elements.
    pub fn reserve_in(&mut self, additional: usize, arena: &mut Arena<'arena, '_>) -> Result<()> {
        let required = self
            .len
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        let required = u32::try_from(required).map_err(|_| CollectionError::CapacityOverflow)?;
        let old_capacity = self.capacity();
        if required as usize <= old_capacity {
            return Ok(());
        }
        let new_capacity = if old_capacity == 0 {
            (required as usize).max(4)
        } else {
            (required as usize).max((old_capacity as u32).saturating_mul(2) as usize)
        };

        if let Some(storage) = &mut self.storage {
            arena.validate_owned(storage)?;
            self.make_contiguous_unchecked();
            let storage = self.storage.as_mut().expect("storage was present");
            if arena.try_resize_owned(storage, new_capacity)? {
                for _ in old_capacity..new_capacity {
                    storage
                        .push(MaybeUninit::uninit())
                        .expect("resized allocation has the requested capacity");
                }
                return Ok(());
            }
        }

        let mut replacement = allocate_slots(new_capacity, arena)?;
        if let Some(storage) = &mut self.storage {
            let source = storage.as_slice();
            let destination = replacement.as_mut_slice();
            for index in 0..self.len {
                // SAFETY: the deque was made contiguous and every logical
                // value occupies the initialized prefix `[0..len]`.
                let value = unsafe { source[index].assume_init_read() };
                destination[index].write(value);
            }
        }
        self.storage = Some(replacement);
        self.head = 0;
        Ok(())
    }

    /// Ensure room for at least `additional` more elements.
    pub fn reserve(&mut self, additional: usize, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.reserve_in(additional, arena)
    }

    /// Reduce capacity to the current length.
    pub fn shrink_to_fit_in(&mut self, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if let Some(storage) = &self.storage {
            arena.validate_owned(storage)?;
        }
        if self.len == 0 {
            self.storage = None;
            self.head = 0;
            return Ok(());
        }
        self.make_contiguous_unchecked();
        let old_capacity = self.capacity();
        let len = self.len;
        let storage = self.storage.as_mut().expect("nonempty deque has storage");
        storage.truncate(len);
        match arena.try_resize_owned(storage, len) {
            Ok(true) => Ok(()),
            Ok(false) => {
                restore_slot_prefix(storage, old_capacity);
                let mut replacement = allocate_slots(len, arena)?;
                let source = storage.as_slice();
                let destination = replacement.as_mut_slice();
                for index in 0..len {
                    // SAFETY: the deque is contiguous and the logical prefix
                    // remains initialized while it is moved to replacement.
                    let value = unsafe { source[index].assume_init_read() };
                    destination[index].write(value);
                }
                self.storage = Some(replacement);
                Ok(())
            }
            Err(error) => {
                restore_slot_prefix(storage, old_capacity);
                Err(error.into())
            }
        }
    }

    /// Reduce capacity to the current length.
    pub fn shrink_to_fit(&mut self, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.shrink_to_fit_in(arena)
    }

    /// Rearrange wrapped entries into one contiguous initialized slice.
    pub fn make_contiguous<'view>(
        &'view mut self,
        arena: &mut Arena<'arena, '_>,
    ) -> Result<&'view mut [T]> {
        if let Some(storage) = &self.storage {
            arena.validate_owned(storage)?;
        }
        self.make_contiguous_unchecked();
        let Some(storage) = &mut self.storage else {
            return Ok(&mut []);
        };
        let slots = storage.as_mut_slice();
        // SAFETY: after reordering, exactly the first `len` slots contain live
        // T values, each with the same alignment and layout as MaybeUninit<T>.
        Ok(unsafe { slice::from_raw_parts_mut(slots.as_mut_ptr().cast::<T>(), self.len) })
    }

    /// Retain only elements for which `keep` returns true.
    ///
    /// Each removal is committed before its destructor runs. If the predicate
    /// or a destructor panics, the remaining deque still has valid ownership.
    pub fn retain<F>(&mut self, arena: &mut Arena<'arena, '_>, mut keep: F) -> Result<()>
    where
        F: FnMut(&T) -> bool,
    {
        if let Some(storage) = &self.storage {
            arena.validate_owned(storage)?;
        }
        let mut index = 0;
        while index < self.len {
            let physical = physical_index(self.head, index, self.capacity());
            let should_keep = {
                let storage = self.storage.as_ref().expect("nonempty deque has storage");
                // SAFETY: `index < len` identifies an initialized slot.
                keep(unsafe { storage.as_slice()[physical].assume_init_ref() })
            };
            if should_keep {
                index += 1;
            } else {
                let removed = self.remove_at(index);
                drop(removed);
            }
        }
        Ok(())
    }

    fn make_contiguous_unchecked(&mut self) {
        if self.head == 0 || self.len == 0 {
            return;
        }
        let storage = self.storage.as_mut().expect("nonempty deque has storage");
        storage.as_mut_slice().rotate_left(self.head);
        self.head = 0;
    }

    fn pop_front_unchecked(&mut self) -> T {
        let storage = self.storage.as_mut().expect("nonempty deque has storage");
        let physical = self.head;
        let capacity = storage.capacity();
        self.len -= 1;
        self.head = if self.len == 0 || physical + 1 == capacity {
            0
        } else {
            physical + 1
        };
        // SAFETY: the front slot was initialized and has been removed from the
        // logical deque before ownership is returned to the caller.
        unsafe { storage.as_mut_slice()[physical].assume_init_read() }
    }

    fn pop_back_unchecked(&mut self) -> T {
        let storage = self.storage.as_mut().expect("nonempty deque has storage");
        let physical = physical_index(self.head, self.len - 1, storage.capacity());
        self.len -= 1;
        if self.len == 0 {
            self.head = 0;
        }
        // SAFETY: the back slot was initialized and has been removed from the
        // logical deque before ownership is returned to the caller.
        unsafe { storage.as_mut_slice()[physical].assume_init_read() }
    }

    fn remove_at(&mut self, index: usize) -> T {
        self.make_contiguous_unchecked();
        let len = self.len;
        let storage = self.storage.as_mut().expect("nonempty deque has storage");
        let slots = storage.as_mut_slice();
        // SAFETY: index is in the initialized logical prefix.
        let removed = unsafe { slots[index].assume_init_read() };
        for source in index + 1..len {
            // SAFETY: each source is initialized and each previous slot was
            // made uninitialized by the preceding read.
            let value = unsafe { slots[source].assume_init_read() };
            slots[source - 1].write(value);
        }
        self.len -= 1;
        self.head = 0;
        removed
    }

    fn drop_remaining(&mut self) {
        let mut guard = DequeDropGuard {
            deque: self,
            armed: true,
        };
        while self.len > 0 {
            let value = self.pop_back_unchecked();
            drop(value);
        }
        guard.armed = false;
    }
}

impl<T: CompactValue> Default for CompactVecDeque<'_, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: CompactValue> Drop for CompactVecDeque<'_, T> {
    fn drop(&mut self) {
        self.drop_remaining();
    }
}

struct DequeDropGuard<'arena, T: CompactValue> {
    deque: *mut CompactVecDeque<'arena, T>,
    armed: bool,
}

impl<T: CompactValue> Drop for DequeDropGuard<'_, T> {
    fn drop(&mut self) {
        if self.armed {
            // SAFETY: the pointer is derived from the exclusively borrowed
            // deque at drop start and is used only to finish panic cleanup.
            unsafe { (*self.deque).drop_remaining() };
        }
    }
}

// SAFETY: this iterator only exposes the initialized logical range; each
// yielded shared reference is tied to its immutable borrow of the deque.
/// A double-ended iterator over a [`CompactVecDeque`]'s logical entries.
pub struct CompactVecDequeIter<'view, T> {
    slots: &'view [MaybeUninit<T>],
    head: usize,
    capacity: usize,
    front: usize,
    back: usize,
}

impl<'view, T> Iterator for CompactVecDequeIter<'view, T> {
    type Item = &'view T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        let physical = physical_index(self.head, self.front, self.capacity);
        self.front += 1;
        // SAFETY: the logical iterator range contains only initialized slots.
        Some(unsafe { self.slots[physical].assume_init_ref() })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.back - self.front;
        (remaining, Some(remaining))
    }
}

impl<T> DoubleEndedIterator for CompactVecDequeIter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        let physical = physical_index(self.head, self.back, self.capacity);
        // SAFETY: the logical iterator range contains only initialized slots.
        Some(unsafe { self.slots[physical].assume_init_ref() })
    }
}

impl<T> ExactSizeIterator for CompactVecDequeIter<'_, T> {}
impl<T> core::iter::FusedIterator for CompactVecDequeIter<'_, T> {}

// SAFETY: the deque owns its storage and moves values according to T's
// CompactValue contract; its drop implementation destroys all logical entries.
unsafe impl<T: CompactValue> CompactValue for CompactVecDeque<'_, T> {}

/// A fixed-capacity ring that evicts and drops the oldest entry before reuse.
pub struct CompactRing<'arena, T: CompactValue> {
    deque: CompactVecDeque<'arena, T>,
    maximum_len: usize,
}

impl<'arena, T: CompactValue> CompactRing<'arena, T> {
    /// Construct a bounded ring whose capacity is `maximum_len`.
    pub fn with_capacity_in(maximum_len: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        if maximum_len == 0 {
            return Err(CollectionError::Core(compact_core::Error::InvalidCapacity));
        }
        let deque = CompactVecDeque::with_capacity_in(maximum_len, arena)?;
        Ok(Self { deque, maximum_len })
    }

    /// Construct a bounded ring whose capacity is `maximum_len`.
    pub fn with_capacity(maximum_len: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        Self::with_capacity_in(maximum_len, arena)
    }

    /// Return the number of retained entries.
    pub fn len(&self) -> usize {
        self.deque.len()
    }

    /// Return the maximum number of retained entries.
    pub const fn capacity(&self) -> usize {
        self.maximum_len
    }

    /// Return whether the ring contains no entries.
    pub fn is_empty(&self) -> bool {
        self.deque.is_empty()
    }

    /// Borrow the oldest retained entry.
    pub fn front<'view>(&'view self, arena: &'view Arena<'arena, '_>) -> Result<Option<&'view T>> {
        self.deque.front(arena)
    }

    /// Append an entry, dropping the oldest entry first when the ring is full.
    pub fn push_back_in(&mut self, value: T, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if self.deque.len() == self.maximum_len {
            let evicted = self
                .deque
                .pop_front_in(arena)?
                .expect("full ring has a front entry");
            drop(evicted);
        }
        self.deque.push_back_in(value, arena)
    }

    /// Append an entry, dropping the oldest entry first when the ring is full.
    pub fn push_back(&mut self, value: T, arena: &mut Arena<'arena, '_>) -> Result<()> {
        self.push_back_in(value, arena)
    }

    /// Remove and return the oldest entry.
    pub fn pop_front(&mut self, arena: &Arena<'arena, '_>) -> Result<Option<T>> {
        self.deque.pop_front_in(arena)
    }

    /// Return an iterator over retained entries from oldest to newest.
    pub fn iter<'view>(
        &'view self,
        arena: &Arena<'arena, '_>,
    ) -> Result<CompactVecDequeIter<'view, T>> {
        self.deque.iter(arena)
    }

    /// Drop all retained entries while keeping the ring allocation for reuse.
    pub fn clear(&mut self) {
        self.deque.clear();
    }
}

// SAFETY: CompactRing delegates ownership and destruction to its deque.
unsafe impl<T: CompactValue> CompactValue for CompactRing<'_, T> {}

fn allocate_slots<'arena, T: CompactValue>(
    capacity: usize,
    arena: &mut Arena<'arena, '_>,
) -> Result<ArenaAllocation<'arena, MaybeUninit<T>>> {
    let mut storage = arena.alloc_owned_slice::<MaybeUninit<T>>(capacity)?;
    for _ in 0..capacity {
        storage.push(MaybeUninit::uninit())?;
    }
    Ok(storage)
}

fn restore_slot_prefix<T: CompactValue>(
    storage: &mut ArenaAllocation<'_, MaybeUninit<T>>,
    capacity: usize,
) {
    while storage.len() < capacity {
        storage
            .push(MaybeUninit::uninit())
            .expect("existing allocation retains its former slot capacity");
    }
}

fn physical_index(head: usize, logical: usize, capacity: usize) -> usize {
    let until_wrap = capacity - head;
    if logical >= until_wrap {
        logical - until_wrap
    } else {
        head + logical
    }
}
