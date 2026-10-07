//! Dense fixed-capacity slab with reusable slots and generation-checked handles.

use core::marker::PhantomData;
use core::mem::MaybeUninit;

use compact_core::{Arena, OffsetSlice32};

use crate::{CollectionError, Result};

const NONE: u32 = u32::MAX;
const OCCUPIED: u32 = 1 << 31;
const GENERATION_MASK: u32 = OCCUPIED - 1;

#[repr(C)]
#[derive(Clone, Copy)]
union SlotPayload<T: Copy> {
    value: MaybeUninit<T>,
    next_free: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Slot<T: Copy> {
    state: u32,
    payload: SlotPayload<T>,
}

/// A slab handle carrying slot index, generation, and owning slab offset.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlabHandle<'arena, T> {
    owner_offset: u32,
    index: u32,
    generation: u32,
    marker: PhantomData<fn(&'arena mut ()) -> &'arena mut ()>,
    type_marker: PhantomData<fn(T) -> T>,
}

impl<T> SlabHandle<'_, T> {
    /// Return the zero-based slot index.
    pub const fn index(self) -> usize {
        self.index as usize
    }

    /// Return the slot generation for diagnostics.
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

/// A dense fixed-capacity arena-local slab.
///
/// Slot handles reject stale generations and handles from another slab.
/// Generation exhaustion retires a slot instead of allowing an ABA handle
/// collision. The slab accepts `Copy` values and never runs native drops.
pub struct CompactSlab<'arena, T: Copy> {
    storage: OffsetSlice32<'arena, MaybeUninit<Slot<T>>>,
    free_head: u32,
    len: u32,
}

impl<'arena, T: Copy> CompactSlab<'arena, T> {
    /// Allocate a slab and initialize its free-list links.
    pub fn with_capacity_in(capacity: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        let capacity = u32::try_from(capacity).map_err(|_| CollectionError::CapacityOverflow)?;
        if capacity == 0 {
            return Ok(Self {
                storage: OffsetSlice32::empty(),
                free_head: NONE,
                len: 0,
            });
        }
        let storage = arena.alloc_uninit_slice::<Slot<T>>(capacity as usize)?;
        for index in 0..capacity {
            let next_free = if index + 1 == capacity {
                NONE
            } else {
                index + 1
            };
            arena.write_uninit_at(
                storage,
                index as usize,
                Slot {
                    state: 0,
                    payload: SlotPayload { next_free },
                },
            )?;
        }
        Ok(Self {
            storage,
            free_head: 0,
            len: 0,
        })
    }

    /// Return the number of occupied slots.
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Return the total slot capacity.
    pub const fn capacity(&self) -> usize {
        self.storage.len()
    }

    /// Return whether every slot is vacant.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Insert a value, returning `None` when no reusable slot remains.
    pub fn insert(
        &mut self,
        value: T,
        arena: &mut Arena<'arena, '_>,
    ) -> Result<Option<SlabHandle<'arena, T>>> {
        if self.free_head == NONE {
            return Ok(None);
        }
        // SAFETY: every slot is initialized during construction and remains a
        // valid Slot; mutation is exclusive through `arena`.
        let slots = unsafe { arena.get_slice_mut_assume_init(self.storage, self.capacity())? };
        loop {
            let index = self.free_head;
            if index == NONE {
                return Ok(None);
            }
            // SAFETY: vacant slots store an initialized `next_free` union arm.
            let next = unsafe { slots[index as usize].payload.next_free };
            let generation = slots[index as usize].state & GENERATION_MASK;
            self.free_head = next;
            if generation == GENERATION_MASK {
                // Retire this slot so a stale handle can never become valid
                // after generation wraparound.
                continue;
            }
            let generation = generation + 1;
            slots[index as usize].state = OCCUPIED | generation;
            slots[index as usize].payload.value = MaybeUninit::new(value);
            self.len += 1;
            return Ok(Some(SlabHandle {
                owner_offset: self.storage.offset().as_u32(),
                index,
                generation,
                marker: PhantomData,
                type_marker: PhantomData,
            }));
        }
    }

    /// Borrow the value named by `handle`, rejecting stale or foreign handles.
    pub fn get<'view>(
        &self,
        handle: SlabHandle<'arena, T>,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<&'view T> {
        let slot = self.checked_slot(handle, arena)?;
        // SAFETY: checked_slot confirms occupied state and matching generation;
        // occupied slots initialize the value union arm before publishing it.
        Ok(unsafe { slot.payload.value.assume_init_ref() })
    }

    /// Mutably borrow the value named by `handle`.
    pub fn get_mut<'view>(
        &self,
        handle: SlabHandle<'arena, T>,
        arena: &'view mut Arena<'arena, '_>,
    ) -> Result<&'view mut T> {
        if handle.owner_offset != self.storage.offset().as_u32()
            || handle.index >= self.capacity() as u32
        {
            return Err(CollectionError::StaleHandle);
        }
        // SAFETY: every slot is initialized and the exclusive arena borrow
        // guarantees unique access to the occupied slot.
        let slots = unsafe { arena.get_slice_mut_assume_init(self.storage, self.capacity())? };
        let slot = &mut slots[handle.index as usize];
        if slot.state & OCCUPIED == 0 || slot.state & GENERATION_MASK != handle.generation {
            return Err(CollectionError::StaleHandle);
        }
        // SAFETY: occupied slots contain a valid initialized T value.
        Ok(unsafe { slot.payload.value.assume_init_mut() })
    }

    /// Remove a value and make its slot available for reuse.
    pub fn remove(
        &mut self,
        handle: SlabHandle<'arena, T>,
        arena: &mut Arena<'arena, '_>,
    ) -> Result<T> {
        if handle.owner_offset != self.storage.offset().as_u32()
            || handle.index >= self.capacity() as u32
        {
            return Err(CollectionError::StaleHandle);
        }
        // SAFETY: every slot is initialized during construction.
        let slots = unsafe { arena.get_slice_mut_assume_init(self.storage, self.capacity())? };
        let slot = &mut slots[handle.index as usize];
        if slot.state & OCCUPIED == 0 || slot.state & GENERATION_MASK != handle.generation {
            return Err(CollectionError::StaleHandle);
        }
        // SAFETY: the occupied state guarantees the value arm is initialized;
        // T: Copy means reading it does not transfer a destructor obligation.
        let value = unsafe { slot.payload.value.assume_init_read() };
        let generation = slot.state & GENERATION_MASK;
        slot.state = generation;
        // SAFETY: after changing to vacant state, the free-list link arm is
        // initialized before the slot is made reachable from free_head.
        if generation == GENERATION_MASK {
            slot.payload.next_free = NONE;
        } else {
            slot.payload.next_free = self.free_head;
            self.free_head = handle.index;
        }
        self.len -= 1;
        Ok(value)
    }

    fn checked_slot<'view>(
        &self,
        handle: SlabHandle<'arena, T>,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<&'view Slot<T>> {
        if handle.owner_offset != self.storage.offset().as_u32()
            || handle.index >= self.capacity() as u32
        {
            return Err(CollectionError::StaleHandle);
        }
        // SAFETY: every slot is initialized and shared access is sufficient
        // because the returned value is only immutably borrowed.
        let slots = unsafe { arena.get_slice_assume_init(self.storage, self.capacity())? };
        let slot = &slots[handle.index as usize];
        if slot.state & OCCUPIED == 0 || slot.state & GENERATION_MASK != handle.generation {
            return Err(CollectionError::StaleHandle);
        }
        Ok(slot)
    }
}
