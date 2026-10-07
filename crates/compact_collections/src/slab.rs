//! Generational slab with slot-local stale-handle protection.

use compact_backend_std::{CageAllocation, CompactRuntime};
use compact_core::CompactValue;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::{CollectionError, Result};

struct Slot<T: CompactValue> {
    generation: u32,
    next_free: u32,
    value: Option<T>,
}
unsafe impl<T: CompactValue> CompactValue for Slot<T> {}
static NEXT_SLAB_ID: AtomicU32 = AtomicU32::new(1);

/// Copyable slab handle with a slot index and per-slot generation.
#[repr(C)]
#[derive(Debug, Eq, PartialEq)]
pub struct SlabHandle<T> {
    index: u32,
    generation: u32,
    slab_id: u32,
    marker: PhantomData<fn() -> T>,
}
impl<T> Copy for SlabHandle<T> {}
impl<T> Clone for SlabHandle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> SlabHandle<T> {
    /// Return the slot index.
    pub const fn index(self) -> usize {
        self.index as usize
    }
    /// Return the slot generation.
    pub const fn generation(self) -> u32 {
        self.generation
    }
}
unsafe impl<T> CompactValue for SlabHandle<T> {}

/// Cage-backed slab that rejects stale handles after a slot is reused.
pub struct CompactSlab<T: CompactValue> {
    slots: CageAllocation<Slot<T>>,
    len: u32,
    free_head: u32,
    slab_id: u32,
}

impl<T: CompactValue> CompactSlab<T> {
    /// Create a slab with the requested number of slots.
    pub fn with_capacity(capacity: usize) -> Result<Self> {
        let capacity = u32::try_from(capacity).map_err(|_| CollectionError::CapacityOverflow)?;
        let slab_id = NEXT_SLAB_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| CollectionError::Core(compact_core::Error::OffsetOverflow))?;
        let mut slots = CompactRuntime::alloc_owned_slice::<Slot<T>>(capacity as usize)?;
        for index in 0..capacity {
            slots.push(Slot {
                generation: 1,
                next_free: if index + 1 < capacity { index + 2 } else { 0 },
                value: None,
            })?;
        }
        Ok(Self {
            slots,
            len: 0,
            free_head: if capacity == 0 { 0 } else { 1 },
            slab_id,
        })
    }
    /// Return live item count.
    pub const fn len(&self) -> usize {
        self.len as usize
    }
    /// Return slot capacity.
    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }
    /// Return whether no items are stored.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn grow(&mut self) -> Result<()> {
        let old = self.capacity();
        let new = old.checked_mul(2).unwrap_or(0).max(4);
        if self.slots.try_resize(new)? {
            for index in old..new {
                self.slots.push(Slot {
                    generation: 1,
                    next_free: self.free_head,
                    value: None,
                })?;
                self.free_head = index as u32 + 1;
            }
            return Ok(());
        }
        let mut replacement = CompactRuntime::alloc_owned_slice::<Slot<T>>(new)?;
        self.slots.move_into(&mut replacement)?;
        for index in old..new {
            replacement.push(Slot {
                generation: 1,
                next_free: self.free_head,
                value: None,
            })?;
            self.free_head = index as u32 + 1;
        }
        self.slots = replacement;
        Ok(())
    }
    /// Insert a value and return its generational handle.
    pub fn insert(&mut self, value: T) -> Result<SlabHandle<T>> {
        if self.free_head == 0 {
            self.grow()?;
        }
        let index = self.free_head - 1;
        let slot = &mut self.slots.as_mut_slice()[index as usize];
        self.free_head = slot.next_free;
        slot.next_free = 0;
        slot.value = Some(value);
        self.len += 1;
        Ok(SlabHandle {
            index,
            generation: slot.generation,
            slab_id: self.slab_id,
            marker: PhantomData,
        })
    }
    /// Return a value if the handle is live and belongs to this slab.
    pub fn get(&self, handle: SlabHandle<T>) -> Option<&T> {
        if handle.slab_id != self.slab_id {
            return None;
        }
        let slot = self.slots.as_slice().get(handle.index as usize)?;
        (slot.generation == handle.generation)
            .then_some(slot.value.as_ref())
            .flatten()
    }
    /// Mutably borrow a value if the handle is live.
    pub fn get_mut(&mut self, handle: SlabHandle<T>) -> Option<&mut T> {
        if handle.slab_id != self.slab_id {
            return None;
        }
        let slot = self.slots.as_mut_slice().get_mut(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        slot.value.as_mut()
    }
    /// Remove a value and invalidate its handle.
    pub fn remove(&mut self, handle: SlabHandle<T>) -> Option<T> {
        if handle.slab_id != self.slab_id {
            return None;
        }
        let slot = self.slots.as_mut_slice().get_mut(handle.index as usize)?;
        if slot.generation != handle.generation || slot.value.is_none() {
            return None;
        }
        let value = slot.value.take();
        self.len -= 1;
        if slot.generation == u32::MAX {
            slot.generation = 0;
        } else {
            slot.generation += 1;
            slot.next_free = self.free_head;
            self.free_head = handle.index + 1;
        }
        value
    }
    /// Iterate over live values and their handles.
    pub fn iter(&self) -> impl Iterator<Item = (SlabHandle<T>, &T)> {
        self.slots
            .as_slice()
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                slot.value.as_ref().map(|value| {
                    (
                        SlabHandle {
                            index: index as u32,
                            generation: slot.generation,
                            slab_id: self.slab_id,
                            marker: PhantomData,
                        },
                        value,
                    )
                })
            })
    }
}

// SAFETY: slots own their values and every handle is scalar metadata.
unsafe impl<T: CompactValue> CompactValue for CompactSlab<T> {}
