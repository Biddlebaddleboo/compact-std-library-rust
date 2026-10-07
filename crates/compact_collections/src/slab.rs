//! Dense fixed-capacity slab with reusable slots and generation-checked handles.

use core::marker::PhantomData;
use core::mem::ManuallyDrop;

use compact_core::{Arena, ArenaAllocation, CompactValue};

use crate::{CollectionError, Result};

const NONE: u32 = u32::MAX;
const OCCUPIED: u32 = 1 << 31;
const GENERATION_MASK: u32 = OCCUPIED - 1;

#[repr(C)]
union SlotPayload<T: CompactValue> {
    value: ManuallyDrop<T>,
    next_free: u32,
}

#[repr(C)]
struct Slot<T: CompactValue> {
    state: u32,
    payload: SlotPayload<T>,
}

impl<T: CompactValue> Drop for Slot<T> {
    fn drop(&mut self) {
        if self.state & OCCUPIED != 0 {
            // SAFETY: the occupied state is published only after the value
            // union arm is initialized, and is cleared before moving it out.
            unsafe { ManuallyDrop::drop(&mut self.payload.value) };
        }
    }
}

// SAFETY: a Slot moves its value arm together with occupancy metadata. Its
// Drop implementation handles the live union arm exactly once.
unsafe impl<T: CompactValue> CompactValue for Slot<T> {}

/// A slab handle carrying slot index, generation, and unique slab identity.
#[repr(C)]
pub struct SlabHandle<'arena, T> {
    owner_id: u32,
    index: u32,
    generation: u32,
    marker: PhantomData<fn(&'arena mut ()) -> &'arena mut ()>,
    type_marker: PhantomData<fn(T) -> T>,
}

impl<T> Copy for SlabHandle<'_, T> {}

impl<T> Clone for SlabHandle<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> core::fmt::Debug for SlabHandle<'_, T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SlabHandle")
            .field("owner_id", &self.owner_id)
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

impl<T> PartialEq for SlabHandle<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        (self.owner_id, self.index, self.generation)
            == (other.owner_id, other.index, other.generation)
    }
}

impl<T> Eq for SlabHandle<'_, T> {}

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
/// Handles reject stale slot generations, handles from another slab, and
/// handles whose former allocation was reclaimed and reused. Dropping the slab
/// drops every occupied value and reclaims its backing.
pub struct CompactSlab<'arena, T: CompactValue> {
    storage: Option<ArenaAllocation<'arena, Slot<T>>>,
    free_head: u32,
    len: u32,
}

impl<'arena, T: CompactValue> CompactSlab<'arena, T> {
    /// Allocate a slab and initialize its free-list links.
    pub fn with_capacity_in(capacity: usize, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        let capacity = u32::try_from(capacity).map_err(|_| CollectionError::CapacityOverflow)?;
        if capacity == 0 {
            return Ok(Self {
                storage: None,
                free_head: NONE,
                len: 0,
            });
        }
        let mut storage = arena.alloc_owned_slice::<Slot<T>>(capacity as usize)?;
        for index in 0..capacity {
            let next_free = if index + 1 == capacity {
                NONE
            } else {
                index + 1
            };
            storage.push(Slot {
                state: 0,
                payload: SlotPayload { next_free },
            })?;
        }
        Ok(Self {
            storage: Some(storage),
            free_head: 0,
            len: 0,
        })
    }

    /// Return the number of occupied slots.
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Return the total slot capacity.
    pub fn capacity(&self) -> usize {
        self.storage.as_ref().map_or(0, ArenaAllocation::capacity)
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
        let Some(storage) = &mut self.storage else {
            return Ok(None);
        };
        arena.validate_owned(storage)?;
        let owner_id = storage.allocation_id();
        let slots = storage.as_mut_slice();
        loop {
            let index = self.free_head;
            if index == NONE {
                return Ok(None);
            }
            let slot = &mut slots[index as usize];
            // SAFETY: vacant slots store an initialized next_free arm.
            let next = unsafe { slot.payload.next_free };
            let generation = slot.state & GENERATION_MASK;
            self.free_head = next;
            if generation == GENERATION_MASK {
                // Retire a slot before its generation would wrap.
                continue;
            }
            let generation = generation + 1;
            slot.payload = SlotPayload {
                value: ManuallyDrop::new(value),
            };
            slot.state = OCCUPIED | generation;
            self.len += 1;
            return Ok(Some(SlabHandle {
                owner_id,
                index,
                generation,
                marker: PhantomData,
                type_marker: PhantomData,
            }));
        }
    }

    /// Borrow the value named by `handle`, rejecting stale or foreign handles.
    pub fn get<'view>(
        &'view self,
        handle: SlabHandle<'arena, T>,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<&'view T> {
        let slot = self.checked_slot(handle, arena)?;
        // SAFETY: checked_slot confirms an occupied slot and matching handle.
        Ok(unsafe { &*((&slot.payload.value as *const ManuallyDrop<T>).cast::<T>()) })
    }

    /// Mutably borrow the value named by `handle`.
    pub fn get_mut<'view>(
        &'view mut self,
        handle: SlabHandle<'arena, T>,
        arena: &'view mut Arena<'arena, '_>,
    ) -> Result<&'view mut T> {
        self.validate_handle(handle, arena)?;
        let storage = self.storage.as_mut().expect("validated slab has storage");
        let slot = &mut storage.as_mut_slice()[handle.index as usize];
        // SAFETY: the validated occupied slot is exclusively borrowed.
        Ok(unsafe { &mut *((&mut slot.payload.value as *mut ManuallyDrop<T>).cast::<T>()) })
    }

    /// Remove a value and make its slot available for reuse.
    pub fn remove(
        &mut self,
        handle: SlabHandle<'arena, T>,
        arena: &mut Arena<'arena, '_>,
    ) -> Result<T> {
        self.validate_handle(handle, arena)?;
        let storage = self.storage.as_mut().expect("validated slab has storage");
        let slots = storage.as_mut_slice();
        let slot = &mut slots[handle.index as usize];
        // SAFETY: the occupied state proves the value arm is initialized. The
        // value is moved out and state changes before the slot is reused.
        let value = unsafe { ManuallyDrop::take(&mut slot.payload.value) };
        slot.state = handle.generation;
        if handle.generation == GENERATION_MASK {
            slot.payload = SlotPayload { next_free: NONE };
        } else {
            slot.payload = SlotPayload {
                next_free: self.free_head,
            };
            self.free_head = handle.index;
        }
        self.len -= 1;
        Ok(value)
    }

    fn validate_handle(
        &self,
        handle: SlabHandle<'arena, T>,
        arena: &Arena<'arena, '_>,
    ) -> Result<()> {
        let Some(storage) = &self.storage else {
            return Err(CollectionError::StaleHandle);
        };
        arena.validate_owned(storage)?;
        if handle.owner_id != storage.allocation_id() || handle.index >= storage.capacity() as u32 {
            return Err(CollectionError::StaleHandle);
        }
        let slot = &storage.as_slice()[handle.index as usize];
        if slot.state & OCCUPIED == 0 || slot.state & GENERATION_MASK != handle.generation {
            return Err(CollectionError::StaleHandle);
        }
        Ok(())
    }

    fn checked_slot<'view>(
        &'view self,
        handle: SlabHandle<'arena, T>,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<&'view Slot<T>> {
        self.validate_handle(handle, arena)?;
        let storage = self.storage.as_ref().expect("validated slab has storage");
        Ok(&storage.as_slice()[handle.index as usize])
    }
}

// SAFETY: moving the slab transfers its unique allocation token; occupied
// values move with their slot metadata and are dropped by Slot::drop.
unsafe impl<T: CompactValue> CompactValue for CompactSlab<'_, T> {}
