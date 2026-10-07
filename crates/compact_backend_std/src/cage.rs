//! Process-wide cage and four-byte allocation owners.

use compact_core::{
    checked_align_up, CompactValue, Error, Offset32, Result, MAX_CAGE_BYTES, MIN_CAGE_BYTES,
};
use core::marker::PhantomData;
use core::mem::{align_of, size_of, MaybeUninit};
use core::ptr::NonNull;
use core::slice;
use std::alloc::{alloc, dealloc, Layout};
use std::sync::{Mutex, MutexGuard, OnceLock};

const INITIAL_CURSOR: u32 = 8;
const FREE_NODE_BYTES: u32 = size_of::<FreeNode>() as u32;

#[repr(C)]
#[derive(Clone, Copy)]
struct AllocationHeader {
    block_len: u32,
    prefix: u32,
    capacity: u32,
    initialized: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FreeNode {
    next: u32,
    len: u32,
}

const _: [(); 16] = [(); size_of::<AllocationHeader>()];
const _: [(); 8] = [(); size_of::<FreeNode>()];

struct Allocator {
    cursor: u32,
    live_bytes: u32,
    free_head: u32,
}

struct CageState {
    capacity: usize,
    memory: NonNull<u8>,
    allocator: Mutex<Allocator>,
}

// SAFETY: `memory` uniquely owns a stable raw allocation. Allocator scalars and
// free-list writes are serialized by `allocator`; each live range's data and
// header are mutated through its exclusive owner or during an allocator
// critical section. Shared views follow `CompactValue`'s safety contract and
// normal Rust borrowing. `CageAllocation<T>` is only Send/Sync when `T` has
// the corresponding native thread-safety traits.
unsafe impl Send for CageState {}
unsafe impl Sync for CageState {}

impl CageState {
    fn base(&self) -> *mut u8 {
        self.memory.as_ptr()
    }
}

impl Drop for CageState {
    fn drop(&mut self) {
        let layout =
            Layout::from_size_align(self.capacity, 8).expect("valid cage allocation layout");
        // SAFETY: `memory` was allocated with this exact layout in `init`.
        unsafe { dealloc(self.memory.as_ptr(), layout) };
    }
}

static CAGE: OnceLock<CageState> = OnceLock::new();

/// Configuration for the one process-wide compact cage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CageConfig {
    /// Requested cage capacity in bytes.
    pub capacity: usize,
}

/// Read-only snapshot of the process cage allocator.
///
/// This diagnostic surface is hidden from the normal API documentation and
/// does not alter allocator state or retained owner layouts.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AllocatorStats {
    /// Bytes occupied by live allocations, including headers and padding.
    pub live_bytes: u32,
    /// Current high-water cursor measured from the start of the cage.
    pub high_water_cursor: u32,
    /// Total bytes in reusable free blocks.
    pub free_bytes: u32,
    /// Number of reusable free blocks.
    pub free_blocks: u32,
    /// Size of the largest reusable free block.
    pub largest_free_block: u32,
}

impl CageConfig {
    /// Create a configuration with the requested capacity.
    pub const fn new(capacity: usize) -> Self {
        Self { capacity }
    }
}

/// Process-wide runtime access and initialization.
pub struct CompactRuntime;

impl CompactRuntime {
    /// Initialize the process cage exactly once.
    pub fn init(config: CageConfig) -> Result<()> {
        if CAGE.get().is_some() {
            return Err(Error::RuntimeAlreadyInitialized);
        }
        if config.capacity < MIN_CAGE_BYTES {
            return Err(Error::InvalidCapacity);
        }
        if config.capacity as u64 > MAX_CAGE_BYTES || config.capacity > u32::MAX as usize {
            return Err(Error::CageTooLarge);
        }
        let layout =
            Layout::from_size_align(config.capacity, 8).map_err(|_| Error::InvalidCapacity)?;
        // SAFETY: `layout` is nonzero and valid after the checks above.
        let memory = NonNull::new(unsafe { alloc(layout) }).ok_or(Error::AllocationFailed)?;
        let state = CageState {
            capacity: config.capacity,
            memory,
            allocator: Mutex::new(Allocator {
                cursor: INITIAL_CURSOR,
                live_bytes: 0,
                free_head: 0,
            }),
        };
        CAGE.set(state)
            .map_err(|_| Error::RuntimeAlreadyInitialized)
    }

    /// Return whether the process cage has been initialized.
    pub fn is_initialized() -> bool {
        CAGE.get().is_some()
    }

    /// Return the configured cage capacity.
    pub fn capacity() -> Result<usize> {
        Ok(state()?.capacity)
    }

    /// Return live allocated bytes including allocation headers and padding.
    pub fn used_bytes() -> Result<usize> {
        Ok(lock(state()?)?.live_bytes as usize)
    }

    /// Return capacity not currently occupied by live allocation blocks.
    pub fn remaining_bytes() -> Result<usize> {
        let state = state()?;
        Ok(state
            .capacity
            .saturating_sub(lock(state)?.live_bytes as usize))
    }

    /// Snapshot live and reusable cage allocator ranges without mutation.
    #[doc(hidden)]
    pub fn allocator_stats() -> Result<AllocatorStats> {
        let state = state()?;
        let allocator = lock(state)?;
        let mut stats = AllocatorStats {
            live_bytes: allocator.live_bytes,
            high_water_cursor: allocator.cursor,
            ..AllocatorStats::default()
        };
        let mut current = allocator.free_head;
        while current != 0 {
            // SAFETY: the free list is protected by the allocator lock and each
            // link is maintained as an in-cage range by allocator operations.
            let node = unsafe { read_free_node(state, current)? };
            stats.free_bytes = stats
                .free_bytes
                .checked_add(node.len)
                .ok_or(Error::InvalidOffset)?;
            stats.free_blocks = stats
                .free_blocks
                .checked_add(1)
                .ok_or(Error::InvalidOffset)?;
            stats.largest_free_block = stats.largest_free_block.max(node.len);
            current = node.next;
        }
        Ok(stats)
    }

    /// Allocate an uninitialized typed block with the requested element capacity.
    pub fn alloc_owned_slice<T: CompactValue>(capacity: usize) -> Result<CageAllocation<T>> {
        CageAllocation::allocate(capacity)
    }

    /// Allocate one initialized value.
    pub fn alloc_owned_value<T: CompactValue>(value: T) -> Result<CageAllocation<T>> {
        let mut allocation = Self::alloc_owned_slice::<T>(1)?;
        allocation.push(value)?;
        Ok(allocation)
    }

    /// Reserve one temporary bump region inside the process cage.
    pub fn scratch(capacity: usize) -> Result<crate::ScratchRegion> {
        crate::ScratchRegion::new(capacity)
    }

    /// Check that an owner still names a live allocation in this process cage.
    pub fn validate_owned<T: CompactValue>(allocation: &CageAllocation<T>) -> Result<()> {
        let header = allocation.header()?;
        let state = state()?;
        let allocator = lock(state)?;
        let start = allocation
            .raw_offset()
            .checked_sub(size_of::<AllocationHeader>() as u32)
            .and_then(|header_offset| header_offset.checked_sub(header.prefix))
            .ok_or(Error::InvalidOffset)?;
        let end = start
            .checked_add(header.block_len)
            .ok_or(Error::InvalidOffset)?;
        if end > allocator.cursor {
            return Err(Error::InvalidOffset);
        }
        let mut free = allocator.free_head;
        while free != 0 {
            let node = unsafe { read_free_node(state, free)? };
            let free_end = free.checked_add(node.len).ok_or(Error::InvalidOffset)?;
            if start < free_end && free < end {
                return Err(Error::InvalidOffset);
            }
            free = node.next;
        }
        Ok(())
    }

    /// Validate intrusive allocator ordering and live/free byte accounting.
    ///
    /// This diagnostic is intended for property tests, fuzzing, and audits.
    #[doc(hidden)]
    pub fn validate_allocator_state() -> Result<()> {
        let state = state()?;
        let allocator = lock(state)?;
        if allocator.cursor as usize > state.capacity || allocator.cursor < INITIAL_CURSOR {
            return Err(Error::InvalidOffset);
        }
        let mut free_bytes = 0_u32;
        let mut previous_end = 0;
        let mut current = allocator.free_head;
        while current != 0 {
            if current < INITIAL_CURSOR || (previous_end != 0 && current <= previous_end) {
                return Err(Error::InvalidOffset);
            }
            let node = unsafe { read_free_node(state, current)? };
            if node.len < FREE_NODE_BYTES || node.len % 8 != 0 {
                return Err(Error::InvalidOffset);
            }
            let end = current.checked_add(node.len).ok_or(Error::InvalidOffset)?;
            if end >= allocator.cursor || current == previous_end {
                return Err(Error::InvalidOffset);
            }
            free_bytes = free_bytes
                .checked_add(node.len)
                .ok_or(Error::InvalidOffset)?;
            previous_end = end;
            current = node.next;
        }
        if free_bytes
            .checked_add(allocator.live_bytes)
            .ok_or(Error::InvalidOffset)?
            != allocator.cursor - INITIAL_CURSOR
        {
            return Err(Error::InitializationError);
        }
        Ok(())
    }

    /// Resolve an offset to a value.
    ///
    /// # Safety
    ///
    /// The offset must name a live initialized `T`, and an owner must keep
    /// that allocation alive and immutably borrowed for the returned lifetime.
    pub unsafe fn resolve_unchecked<'a, T: CompactValue>(offset: Offset32<T>) -> Result<&'a T> {
        if offset.is_null() {
            return Err(Error::InvalidOffset);
        }
        let state = state()?;
        let header = unsafe { read_header(state, offset.as_u32()) }?;
        if header.initialized == 0 {
            return Err(Error::InitializationError);
        }
        // SAFETY: the caller upholds the lifetime and provenance contract.
        Ok(unsafe { &*ptr_from_offset::<T>(state, offset.as_u32()) })
    }

    /// Resolve an offset to a byte range.
    ///
    /// # Safety
    ///
    /// The range must be wholly inside a live byte allocation kept alive by
    /// an owner for the returned lifetime.
    pub unsafe fn resolve_bytes_unchecked<'a>(offset: u32, len: usize) -> Result<&'a [u8]> {
        let state = state()?;
        let header = unsafe { read_header(state, offset) }?;
        if len > header.capacity as usize {
            return Err(Error::OutOfBounds);
        }
        // SAFETY: the caller guarantees liveness and initialization; the
        // allocation header bounds the requested byte range.
        Ok(unsafe { slice_from_offset(state, offset, len) })
    }
}

/// A unique compact allocation owner represented by one non-null `u32` offset.
#[repr(transparent)]
#[must_use = "dropping this owner releases its cage allocation"]
pub struct CageAllocation<T: CompactValue> {
    offset: NonZeroOffset,
    marker: PhantomData<T>,
}

#[repr(transparent)]
#[derive(Clone, Copy)]
struct NonZeroOffset(core::num::NonZeroU32);

/// Temporary native view whose lifetime is tied to an owner borrow.
struct ResolvedAllocation<'a, T> {
    ptr: *mut T,
    header: AllocationHeader,
    marker: PhantomData<&'a [T]>,
}

impl<'a, T> ResolvedAllocation<'a, T> {
    fn as_slice(&self) -> &'a [T] {
        // SAFETY: the view was resolved from its live owner and its marker keeps
        // the owner borrowed for `'a`; the header records the initialized prefix.
        unsafe { slice::from_raw_parts(self.ptr, self.header.initialized as usize) }
    }
}

/// Temporary exclusive native view whose lifetime is tied to an owner borrow.
struct ResolvedAllocationMut<'a, T> {
    ptr: *mut T,
    header_ptr: *mut AllocationHeader,
    header: AllocationHeader,
    marker: PhantomData<&'a mut [T]>,
}

impl<'a, T> ResolvedAllocationMut<'a, T> {
    fn into_mut_slice(self) -> &'a mut [T] {
        // SAFETY: the view holds the unique owner borrow and the header records
        // exactly the initialized prefix.
        unsafe { slice::from_raw_parts_mut(self.ptr, self.header.initialized as usize) }
    }

    fn set_initialized(&mut self, initialized: u32) {
        debug_assert!(initialized <= self.header.capacity);
        self.header.initialized = initialized;
        // SAFETY: this view is exclusively borrowed from the live allocation.
        unsafe { (*self.header_ptr).initialized = initialized };
    }
}

struct AppendInitGuard<'a, T> {
    ptr: *mut T,
    header_ptr: *mut AllocationHeader,
    start: u32,
    written: u32,
    marker: PhantomData<&'a mut [T]>,
}

impl<'a, T> AppendInitGuard<'a, T> {
    fn new<'v>(view: &'v mut ResolvedAllocationMut<'_, T>) -> AppendInitGuard<'v, T> {
        AppendInitGuard {
            ptr: view.ptr,
            header_ptr: view.header_ptr,
            start: view.header.initialized,
            written: 0,
            marker: PhantomData,
        }
    }
}

impl<T> Drop for AppendInitGuard<'_, T> {
    fn drop(&mut self) {
        // Publish the initialized prefix even if the iterator or constructor
        // panicked. Any already-written values will then be dropped normally.
        // SAFETY: created while holding the owner's exclusive borrow.
        unsafe { (*self.header_ptr).initialized = self.start + self.written };
    }
}

impl<T: CompactValue> CageAllocation<T> {
    fn allocate(capacity: usize) -> Result<Self> {
        let capacity_u32 = u32::try_from(capacity).map_err(|_| Error::OffsetOverflow)?;
        let bytes = size_of::<T>()
            .checked_mul(capacity)
            .ok_or(Error::OffsetOverflow)?;
        let state = state()?;
        let mut allocator = lock(state)?;
        let needed = bytes.max(1);
        let alignment = align_of::<T>().max(4);
        let (data_offset, prefix, block_len) =
            allocate_block(state, &mut allocator, needed, alignment)?;
        let offset = data_offset;
        let header = AllocationHeader {
            block_len,
            prefix,
            capacity: capacity_u32,
            initialized: 0,
        };
        // SAFETY: `allocate_block` reserves this aligned header and data range.
        unsafe { header_ptr(state, offset).write(header) };
        let offset = core::num::NonZeroU32::new(offset).ok_or(Error::InvalidOffset)?;
        Ok(Self {
            offset: NonZeroOffset(offset),
            marker: PhantomData,
        })
    }

    /// Return the initialized element count.
    pub fn len(&self) -> usize {
        self.header().expect("live cage owner header").initialized as usize
    }
    /// Return the allocated element capacity.
    pub fn capacity(&self) -> usize {
        self.header().expect("live cage owner header").capacity as usize
    }
    /// Return initialized length and capacity from one resolved header read.
    #[doc(hidden)]
    pub fn len_capacity(&self) -> (usize, usize) {
        let header = self.header().expect("live cage owner header");
        (header.initialized as usize, header.capacity as usize)
    }
    /// Return whether the allocation has no initialized elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Return this allocation's typed offset.
    pub fn offset(&self) -> Offset32<T> {
        // SAFETY: this owner always contains the allocator-issued live offset.
        unsafe { Offset32::from_raw_unchecked(self.raw_offset()) }
    }
    /// Borrow the initialized prefix.
    pub fn as_slice(&self) -> &[T] {
        self.resolved().expect("live cage owner header").as_slice()
    }
    /// Mutably borrow the initialized prefix.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.resolved_mut()
            .expect("live cage owner header")
            .into_mut_slice()
    }
    /// Return an initialized element by index.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.resolved()
            .expect("live cage owner header")
            .as_slice()
            .get(index)
    }
    /// Mutably borrow an initialized element by index.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.resolved_mut()
            .expect("live cage owner header")
            .into_mut_slice()
            .get_mut(index)
    }
    /// Initialize the next element slot.
    pub fn push(&mut self, value: T) -> Result<()> {
        let mut resolved = self.resolved_mut()?;
        if resolved.header.initialized >= resolved.header.capacity {
            return Err(Error::OutOfBounds);
        }
        let index = resolved.header.initialized as usize;
        // SAFETY: this is the next uninitialized slot within the owner.
        unsafe { resolved.ptr.add(index).write(value) };
        resolved.set_initialized(index as u32 + 1);
        Ok(())
    }
    /// Remove and return the final initialized element.
    pub fn pop(&mut self) -> Option<T> {
        let mut resolved = self.resolved_mut().expect("live cage owner header");
        let len = resolved.header.initialized as usize;
        if len == 0 {
            return None;
        }
        resolved.set_initialized((len - 1) as u32);
        // SAFETY: the element was initialized and removed from the drop prefix.
        Some(unsafe { resolved.ptr.add(len - 1).read() })
    }
    /// Drop initialized elements until `new_len` remain.
    pub fn truncate(&mut self, new_len: usize) {
        let mut guard = TruncateGuard {
            allocation: self,
            new_len,
            armed: true,
        };
        // SAFETY: the guard is created from this exclusive owner borrow.
        let allocation = unsafe { &mut *guard.allocation };
        let mut resolved = allocation.resolved_mut().expect("live cage owner header");
        if !core::mem::needs_drop::<T>() {
            let target_len = new_len.min(resolved.header.initialized as usize);
            resolved.set_initialized(target_len as u32);
            guard.armed = false;
            return;
        }
        while resolved.header.initialized as usize > new_len {
            let index = resolved.header.initialized as usize - 1;
            resolved.set_initialized(index as u32);
            // SAFETY: length was lowered first, so unwinding cannot double-drop this element.
            unsafe { resolved.ptr.add(index).drop_in_place() };
        }
        guard.armed = false;
    }
    /// Append a copyable slice to the initialized prefix.
    pub fn extend_copy(&mut self, values: &[T]) -> Result<()>
    where
        T: Copy,
    {
        let mut resolved = self.resolved_mut()?;
        let start = resolved.header.initialized as usize;
        let end = start
            .checked_add(values.len())
            .ok_or(Error::OffsetOverflow)?;
        if end > resolved.header.capacity as usize {
            return Err(Error::OutOfBounds);
        }
        if !values.is_empty() {
            // SAFETY: destination is uninitialized and the source is valid.
            unsafe {
                resolved
                    .ptr
                    .add(start)
                    .copy_from_nonoverlapping(values.as_ptr(), values.len())
            };
        }
        resolved.set_initialized(end as u32);
        Ok(())
    }

    /// Append values from an iterator into available capacity, resolving the
    /// allocation once. A panic publishes the exact initialized prefix.
    #[doc(hidden)]
    pub fn extend_from_iter(
        &mut self,
        iterator: &mut impl Iterator<Item = T>,
        max: usize,
    ) -> Result<usize> {
        let mut resolved = self.resolved_mut()?;
        let start = resolved.header.initialized as usize;
        let available = resolved.header.capacity as usize - start;
        if max > available {
            return Err(Error::OutOfBounds);
        }
        let mut guard = AppendInitGuard::new(&mut resolved);
        while (guard.written as usize) < max {
            let Some(value) = iterator.next() else {
                break;
            };
            // SAFETY: max was checked against available capacity and each slot
            // is written once before the guard publishes it as initialized.
            unsafe { guard.ptr.add(start + guard.written as usize).write(value) };
            guard.written += 1;
        }
        Ok(guard.written as usize)
    }

    /// Append values from a fallible producer into available capacity. Source
    /// errors are saved for the caller after the successfully produced prefix
    /// has been published.
    #[doc(hidden)]
    pub fn extend_from_fallible_fn<E>(
        &mut self,
        max: usize,
        mut next: impl FnMut() -> core::result::Result<Option<T>, E>,
        source_error: &mut Option<E>,
    ) -> Result<usize> {
        let mut resolved = self.resolved_mut()?;
        let start = resolved.header.initialized as usize;
        let available = resolved.header.capacity as usize - start;
        if max > available {
            return Err(Error::OutOfBounds);
        }
        let mut guard = AppendInitGuard::new(&mut resolved);
        while (guard.written as usize) < max {
            let value = match next() {
                Ok(Some(value)) => value,
                Ok(None) => break,
                Err(error) => {
                    *source_error = Some(error);
                    break;
                }
            };
            // SAFETY: max was checked against available capacity and each value
            // is written before the guard publishes it as initialized.
            unsafe { guard.ptr.add(start + guard.written as usize).write(value) };
            guard.written += 1;
        }
        Ok(guard.written as usize)
    }

    /// Construct and append `count` values, publishing an initialized prefix
    /// safely if the constructor panics.
    #[doc(hidden)]
    pub fn extend_from_fn(
        &mut self,
        count: usize,
        mut make_value: impl FnMut() -> T,
    ) -> Result<()> {
        let mut resolved = self.resolved_mut()?;
        let start = resolved.header.initialized as usize;
        let available = resolved.header.capacity as usize - start;
        if count > available {
            return Err(Error::OutOfBounds);
        }
        let mut guard = AppendInitGuard::new(&mut resolved);
        while (guard.written as usize) < count {
            let value = make_value();
            // SAFETY: count was checked against available capacity and each
            // produced value is written before the guard publishes it.
            unsafe { guard.ptr.add(start + guard.written as usize).write(value) };
            guard.written += 1;
        }
        Ok(())
    }
    /// Move the initialized prefix into an empty owner of the same type.
    pub fn move_into(&mut self, destination: &mut Self) -> Result<()> {
        let mut source = self.resolved_mut()?;
        let mut destination = destination.resolved_mut()?;
        if destination.header.initialized != 0 {
            return Err(Error::InitializationError);
        }
        let len = source.header.initialized as usize;
        if len > destination.header.capacity as usize {
            return Err(Error::OutOfBounds);
        }
        if len != 0 {
            // SAFETY: the owners are distinct, the source prefix is initialized,
            // and the destination range is uninitialized and large enough.
            unsafe { destination.ptr.copy_from_nonoverlapping(source.ptr, len) };
        }
        source.set_initialized(0);
        destination.set_initialized(len as u32);
        Ok(())
    }
    /// Move initialized values from an inline uninitialized slice.
    ///
    /// # Safety
    ///
    /// `source` must point to `len` initialized, aligned `T` values. The source
    /// slots must not overlap the destination allocation and become
    /// uninitialized; they must not be read or dropped afterward.
    pub unsafe fn move_from_uninit_slice(
        &mut self,
        source: *mut MaybeUninit<T>,
        len: usize,
    ) -> Result<()> {
        let mut resolved = self.resolved_mut()?;
        let start = resolved.header.initialized as usize;
        let end = start.checked_add(len).ok_or(Error::OffsetOverflow)?;
        if end > resolved.header.capacity as usize {
            return Err(Error::OutOfBounds);
        }
        for index in 0..len {
            // SAFETY: guaranteed by the caller; the destination is in capacity.
            let value = unsafe { source.add(index).cast::<T>().read() };
            unsafe { resolved.ptr.add(start + index).write(value) };
        }
        resolved.set_initialized(end as u32);
        Ok(())
    }
    /// Try to change capacity without relocating the allocation.
    pub fn try_resize(&mut self, capacity: usize) -> Result<bool> {
        let requested = u32::try_from(capacity).map_err(|_| Error::OffsetOverflow)?;
        let state = state()?;
        let offset = self.raw_offset();
        // SAFETY: this non-copy owner represents a live allocator-issued offset.
        let header = unsafe { read_header(state, offset) }?;
        validate_typed_header::<T>(state, offset, header)?;
        if requested < header.initialized {
            return Err(Error::InitializationError);
        }
        let bytes = size_of::<T>()
            .checked_mul(capacity)
            .ok_or(Error::OffsetOverflow)?
            .max(1);
        let mut allocator = lock(state)?;
        let start = self
            .raw_offset()
            .checked_sub(size_of::<AllocationHeader>() as u32)
            .and_then(|header_offset| header_offset.checked_sub(header.prefix))
            .ok_or(Error::InvalidOffset)?;
        let old_len = header.block_len;
        let new_len = u32::try_from(checked_align_up(
            (header.prefix as usize)
                .checked_add(size_of::<AllocationHeader>())
                .and_then(|n| n.checked_add(bytes))
                .ok_or(Error::OffsetOverflow)?,
            8,
        )?)
        .map_err(|_| Error::OffsetOverflow)?;
        if new_len <= old_len {
            if new_len < old_len {
                insert_free(state, &mut allocator, start + new_len, old_len - new_len)?;
            }
            allocator.live_bytes = allocator.live_bytes - old_len + new_len;
            let mut changed = header;
            changed.block_len = new_len;
            changed.capacity = requested;
            // SAFETY: this owner uniquely represents the live allocation.
            unsafe { header_ptr(state, offset).write(changed) };
            return Ok(true);
        }
        let end = start.checked_add(old_len).ok_or(Error::OffsetOverflow)?;
        let extra = new_len - old_len;
        if end == allocator.cursor {
            if allocator
                .cursor
                .checked_add(extra)
                .ok_or(Error::OffsetOverflow)?
                > state.capacity as u32
            {
                return Ok(false);
            }
            allocator.cursor += extra;
        } else if let Some((free_len, _next)) = free_node_at(state, &allocator, end)? {
            if free_len < extra {
                return Ok(false);
            }
            consume_free_prefix(state, &mut allocator, end, extra)?;
        } else {
            return Ok(false);
        }
        allocator.live_bytes = allocator
            .live_bytes
            .checked_add(extra)
            .ok_or(Error::OffsetOverflow)?;
        let mut changed = header;
        changed.block_len = new_len;
        changed.capacity = requested;
        // SAFETY: this owner uniquely represents the live allocation.
        unsafe { header_ptr(state, offset).write(changed) };
        Ok(true)
    }
    /// Borrow the full capacity as potentially uninitialized slots.
    pub fn uninit_capacity(&self) -> &[MaybeUninit<T>] {
        let resolved = self.resolved().expect("live cage owner header");
        // SAFETY: MaybeUninit permits reading every slot state; the owner borrow
        // keeps the allocation live for the returned slice.
        unsafe {
            slice::from_raw_parts(
                resolved.ptr.cast::<MaybeUninit<T>>(),
                resolved.header.capacity as usize,
            )
        }
    }
    /// Mutably borrow the full capacity as potentially uninitialized slots.
    pub fn uninit_capacity_mut(&mut self) -> &mut [MaybeUninit<T>] {
        let resolved = self.resolved_mut().expect("live cage owner header");
        let ptr = resolved.ptr;
        let capacity = resolved.header.capacity as usize;
        // SAFETY: the unique owner is mutably borrowed and every capacity slot is writable.
        unsafe { slice::from_raw_parts_mut(ptr.cast::<MaybeUninit<T>>(), capacity) }
    }
    fn raw_offset(&self) -> u32 {
        self.offset.0.get()
    }
    fn header(&self) -> Result<AllocationHeader> {
        // SAFETY: this non-copy owner represents an allocator-issued offset.
        let state = state()?;
        let header = unsafe { read_header(state, self.raw_offset()) }?;
        validate_typed_header::<T>(state, self.raw_offset(), header)?;
        Ok(header)
    }
    fn resolved(&self) -> Result<ResolvedAllocation<'_, T>> {
        let state = state()?;
        let offset = self.raw_offset();
        // SAFETY: this owner represents an allocator-issued live allocation.
        let header = unsafe { read_header(state, offset) }?;
        validate_typed_header::<T>(state, offset, header)?;
        // SAFETY: the validated owner offset points to its aligned payload.
        let ptr = unsafe { ptr_from_offset::<T>(state, offset) };
        Ok(ResolvedAllocation {
            ptr,
            header,
            marker: PhantomData,
        })
    }
    fn resolved_mut(&mut self) -> Result<ResolvedAllocationMut<'_, T>> {
        let state = state()?;
        let offset = self.raw_offset();
        // SAFETY: this exclusive owner represents an allocator-issued live allocation.
        let header = unsafe { read_header(state, offset) }?;
        validate_typed_header::<T>(state, offset, header)?;
        // SAFETY: the validated owner offset points to its aligned payload/header.
        let ptr = unsafe { ptr_from_offset::<T>(state, offset) };
        let header_ptr = unsafe { header_ptr(state, offset) };
        Ok(ResolvedAllocationMut {
            ptr,
            header_ptr,
            header,
            marker: PhantomData,
        })
    }
}

impl CageAllocation<u64> {
    /// Borrow the initialized words as their exact byte representation.
    pub fn as_byte_slice(&self) -> &[u8] {
        let resolved = self.resolved().expect("live cage owner header");
        let len = (resolved.header.initialized as usize)
            .checked_mul(size_of::<u64>())
            .expect("cage byte length fits its allocation");
        // SAFETY: every initialized `u64` has all bytes initialized, and the
        // returned slice is tied to this immutable owner borrow.
        unsafe { slice::from_raw_parts(resolved.ptr.cast::<u8>(), len) }
    }
}

struct TruncateGuard<T: CompactValue> {
    allocation: *mut CageAllocation<T>,
    new_len: usize,
    armed: bool,
}
impl<T: CompactValue> Drop for TruncateGuard<T> {
    fn drop(&mut self) {
        if self.armed {
            // SAFETY: guard is created from an exclusive owner borrow and runs only during unwind.
            unsafe { (*self.allocation).truncate(self.new_len) };
        }
    }
}

struct ReleaseGuard(u32);
impl Drop for ReleaseGuard {
    fn drop(&mut self) {
        release(self.0);
    }
}

impl<T: CompactValue> Drop for CageAllocation<T> {
    fn drop(&mut self) {
        let _release = ReleaseGuard(self.raw_offset());
        self.truncate(0);
    }
}

unsafe impl<T: CompactValue> CompactValue for CageAllocation<T> {}

fn state() -> Result<&'static CageState> {
    CAGE.get().ok_or(Error::RuntimeNotInitialized)
}
fn lock(state: &CageState) -> Result<MutexGuard<'_, Allocator>> {
    state.allocator.lock().map_err(|_| Error::AllocatorPoisoned)
}
fn block_layout(
    base: *mut u8,
    start: u32,
    bytes: usize,
    alignment: usize,
) -> Result<(u32, u32, u32)> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(Error::AlignmentError);
    }
    let after_header = (base as usize)
        .checked_add(start as usize)
        .and_then(|n| n.checked_add(size_of::<AllocationHeader>()))
        .ok_or(Error::OffsetOverflow)?;
    let data_address =
        checked_align_up(after_header, alignment.max(align_of::<AllocationHeader>()))?;
    let data = data_address
        .checked_sub(base as usize)
        .ok_or(Error::OffsetOverflow)?;
    let header_offset = data
        .checked_sub(size_of::<AllocationHeader>())
        .ok_or(Error::OffsetOverflow)?;
    let prefix = header_offset
        .checked_sub(start as usize)
        .ok_or(Error::OffsetOverflow)?;
    let raw = prefix
        .checked_add(size_of::<AllocationHeader>())
        .and_then(|n| n.checked_add(bytes))
        .ok_or(Error::OffsetOverflow)?;
    let len = checked_align_up(raw, 8)?;
    Ok((
        u32::try_from(data).map_err(|_| Error::OffsetOverflow)?,
        u32::try_from(prefix).map_err(|_| Error::OffsetOverflow)?,
        u32::try_from(len).map_err(|_| Error::OffsetOverflow)?,
    ))
}

/// Canonical conversion from a cage-relative offset to a temporary native pointer.
///
/// # Safety
///
/// `offset` must be within the cage allocation. The caller must ensure the
/// resulting pointer is used only while the owning cage allocation remains live.
unsafe fn ptr_from_offset<T>(state: &CageState, offset: u32) -> *mut T {
    // SAFETY: the caller proves the offset is within the cage allocation.
    unsafe { state.base().add(offset as usize).cast::<T>() }
}

/// Canonical conversion from a cage-relative byte range to a temporary slice.
///
/// # Safety
///
/// The caller proves that `offset..offset + len` lies in one live allocation,
/// that every byte is initialized, and that the returned borrow is tied to its owner.
unsafe fn slice_from_offset<'a>(state: &CageState, offset: u32, len: usize) -> &'a [u8] {
    // SAFETY: delegated to the caller; pointer formation is centralized above.
    unsafe { slice::from_raw_parts(ptr_from_offset::<u8>(state, offset), len) }
}

/// Resolve the common header immediately before an allocation's data offset.
///
/// # Safety
///
/// `offset` must be an allocator-issued, live owner offset or validated by the
/// caller's unsafe contract.
unsafe fn header_ptr(state: &CageState, offset: u32) -> *mut AllocationHeader {
    let header_offset = offset
        .checked_sub(size_of::<AllocationHeader>() as u32)
        .expect("live allocation has a preceding header");
    // SAFETY: guaranteed by this function's caller.
    unsafe { ptr_from_offset(state, header_offset) }
}

unsafe fn read_header(state: &CageState, offset: u32) -> Result<AllocationHeader> {
    let header_offset = offset
        .checked_sub(size_of::<AllocationHeader>() as u32)
        .ok_or(Error::InvalidOffset)?;
    if offset as usize > state.capacity {
        return Err(Error::InvalidOffset);
    }
    // SAFETY: caller supplies an allocator-issued offset or upholds the unsafe contract.
    let header = unsafe { ptr_from_offset::<AllocationHeader>(state, header_offset).read() };
    let prefix_and_header = header
        .prefix
        .checked_add(size_of::<AllocationHeader>() as u32)
        .ok_or(Error::InvalidOffset)?;
    let block_start = header_offset
        .checked_sub(header.prefix)
        .ok_or(Error::InvalidOffset)?;
    let block_end = block_start
        .checked_add(header.block_len)
        .ok_or(Error::InvalidOffset)?;
    if header.block_len < prefix_and_header
        || header.block_len == 0
        || block_end as usize > state.capacity
        || header.initialized > header.capacity
    {
        return Err(Error::InvalidOffset);
    }
    Ok(header)
}

fn validate_typed_header<T>(
    state: &CageState,
    offset: u32,
    header: AllocationHeader,
) -> Result<()> {
    let payload_bytes = size_of::<T>()
        .checked_mul(header.capacity as usize)
        .ok_or(Error::OffsetOverflow)?
        .max(1);
    let payload_start = (offset as usize)
        .checked_add(payload_bytes)
        .ok_or(Error::InvalidOffset)?;
    let block_end = (offset as usize)
        .checked_sub(size_of::<AllocationHeader>())
        .and_then(|n| n.checked_sub(header.prefix as usize))
        .and_then(|n| n.checked_add(header.block_len as usize))
        .ok_or(Error::InvalidOffset)?;
    if payload_start > block_end || block_end > state.capacity {
        return Err(Error::InvalidOffset);
    }
    Ok(())
}

fn allocate_block(
    state: &CageState,
    allocator: &mut Allocator,
    bytes: usize,
    alignment: usize,
) -> Result<(u32, u32, u32)> {
    let required_bytes = u32::try_from(bytes).map_err(|_| Error::OffsetOverflow)?;
    let mut previous = 0;
    let mut current = allocator.free_head;
    while current != 0 {
        let node = unsafe { read_free_node(state, current)? };
        let (data, prefix, required_len) =
            block_layout(state.base(), current, required_bytes as usize, alignment)?;
        if required_len <= node.len {
            let remainder = node.len - required_len;
            let allocated_len = if remainder >= FREE_NODE_BYTES {
                required_len
            } else {
                node.len
            };
            let live_bytes = allocator
                .live_bytes
                .checked_add(allocated_len)
                .ok_or(Error::OffsetOverflow)?;
            let updated_link = if remainder >= FREE_NODE_BYTES {
                let remainder_start = current
                    .checked_add(required_len)
                    .ok_or(Error::OffsetOverflow)?;
                unsafe {
                    write_free_node(
                        state,
                        remainder_start,
                        FreeNode {
                            next: node.next,
                            len: remainder,
                        },
                    )
                };
                remainder_start
            } else {
                node.next
            };
            if previous == 0 {
                allocator.free_head = updated_link;
            } else {
                let mut previous_node = unsafe { read_free_node(state, previous)? };
                previous_node.next = updated_link;
                unsafe { write_free_node(state, previous, previous_node) };
            }
            allocator.live_bytes = live_bytes;
            return Ok((data, prefix, allocated_len));
        }
        previous = current;
        current = node.next;
    }

    let start = allocator.cursor;
    let (data, prefix, len) =
        block_layout(state.base(), start, required_bytes as usize, alignment)?;
    let end = start.checked_add(len).ok_or(Error::OffsetOverflow)?;
    if end > state.capacity as u32 || end as u64 > MAX_CAGE_BYTES {
        return Err(Error::AllocationExhausted);
    }
    let live_bytes = allocator
        .live_bytes
        .checked_add(len)
        .ok_or(Error::OffsetOverflow)?;
    allocator.cursor = end;
    allocator.live_bytes = live_bytes;
    Ok((data, prefix, len))
}

unsafe fn read_free_node(state: &CageState, offset: u32) -> Result<FreeNode> {
    let end = offset
        .checked_add(FREE_NODE_BYTES)
        .ok_or(Error::InvalidOffset)?;
    if offset == 0 || end as usize > state.capacity {
        return Err(Error::InvalidOffset);
    }
    // SAFETY: the allocator only links free blocks inside its cage and the
    // node bytes are initialized whenever a block is inserted into the list.
    Ok(unsafe { ptr_from_offset::<FreeNode>(state, offset).read_unaligned() })
}

unsafe fn write_free_node(state: &CageState, offset: u32, node: FreeNode) {
    // SAFETY: callers prove this range belongs to a free block inside the cage.
    unsafe { ptr_from_offset::<FreeNode>(state, offset).write_unaligned(node) };
}

fn free_node_at(
    state: &CageState,
    allocator: &Allocator,
    offset: u32,
) -> Result<Option<(u32, u32)>> {
    let mut current = allocator.free_head;
    while current != 0 {
        let node = unsafe { read_free_node(state, current)? };
        if current == offset {
            return Ok(Some((node.len, node.next)));
        }
        if current > offset {
            break;
        }
        current = node.next;
    }
    Ok(None)
}

fn consume_free_prefix(
    state: &CageState,
    allocator: &mut Allocator,
    start: u32,
    consumed: u32,
) -> Result<()> {
    let mut previous = 0;
    let mut current = allocator.free_head;
    while current != 0 && current < start {
        previous = current;
        current = unsafe { read_free_node(state, current)? }.next;
    }
    if current != start {
        return Err(Error::InvalidOffset);
    }
    let node = unsafe { read_free_node(state, current)? };
    if consumed > node.len {
        return Err(Error::InvalidOffset);
    }
    let remainder = node.len - consumed;
    let updated_link = if remainder >= FREE_NODE_BYTES {
        let remainder_start = start.checked_add(consumed).ok_or(Error::OffsetOverflow)?;
        unsafe {
            write_free_node(
                state,
                remainder_start,
                FreeNode {
                    next: node.next,
                    len: remainder,
                },
            )
        };
        remainder_start
    } else if remainder == 0 {
        node.next
    } else {
        return Err(Error::InvalidOffset);
    };
    if previous == 0 {
        allocator.free_head = updated_link;
    } else {
        let mut previous_node = unsafe { read_free_node(state, previous)? };
        previous_node.next = updated_link;
        unsafe { write_free_node(state, previous, previous_node) };
    }
    Ok(())
}

fn insert_free(state: &CageState, allocator: &mut Allocator, start: u32, len: u32) -> Result<()> {
    if len == 0 || len < FREE_NODE_BYTES || len % 8 != 0 {
        return Err(Error::InvalidOffset);
    }
    let end = start.checked_add(len).ok_or(Error::OffsetOverflow)?;
    if start < INITIAL_CURSOR || end > allocator.cursor {
        return Err(Error::InvalidOffset);
    }

    let mut previous = 0;
    let mut next = allocator.free_head;
    while next != 0 && next < start {
        previous = next;
        next = unsafe { read_free_node(state, next)? }.next;
    }
    let previous_node = if previous == 0 {
        None
    } else {
        Some(unsafe { read_free_node(state, previous)? })
    };
    let next_node = if next == 0 {
        None
    } else {
        Some(unsafe { read_free_node(state, next)? })
    };
    if let Some(node) = previous_node {
        let previous_end = previous.checked_add(node.len).ok_or(Error::InvalidOffset)?;
        if previous_end > start {
            return Err(Error::InvalidOffset);
        }
    }
    if next_node.is_some() && end > next {
        return Err(Error::InvalidOffset);
    }

    let previous_is_adjacent =
        previous_node.and_then(|node| previous.checked_add(node.len)) == Some(start);
    let merged_start = if previous_is_adjacent {
        previous
    } else {
        start
    };
    let mut merged_len = len;
    let mut merged_next = next;
    if previous_is_adjacent {
        let node = previous_node.expect("adjacent previous free block");
        merged_len = node.len.checked_add(len).ok_or(Error::OffsetOverflow)?;
        merged_next = node.next;
    } else if previous == 0 {
        allocator.free_head = start;
    } else {
        let mut node = previous_node.expect("previous free block");
        node.next = start;
        unsafe { write_free_node(state, previous, node) };
    }
    if merged_start.checked_add(merged_len) == Some(next) {
        let node = next_node.expect("next free block");
        merged_len = merged_len
            .checked_add(node.len)
            .ok_or(Error::OffsetOverflow)?;
        merged_next = node.next;
    }
    unsafe {
        write_free_node(
            state,
            merged_start,
            FreeNode {
                next: merged_next,
                len: merged_len,
            },
        )
    };

    loop {
        let mut prev = 0;
        let mut last = allocator.free_head;
        if last == 0 {
            break;
        }
        loop {
            let node = unsafe { read_free_node(state, last)? };
            if node.next == 0 {
                if last.checked_add(node.len) == Some(allocator.cursor) {
                    if prev == 0 {
                        allocator.free_head = 0;
                    } else {
                        let mut previous_node = unsafe { read_free_node(state, prev)? };
                        previous_node.next = 0;
                        unsafe { write_free_node(state, prev, previous_node) };
                    }
                    allocator.cursor = last;
                }
                break;
            }
            prev = last;
            last = node.next;
        }
        if allocator.free_head == 0 || last != allocator.cursor {
            break;
        }
    }
    Ok(())
}

fn release(offset: u32) {
    let Ok(state) = state() else {
        return;
    };
    let mut allocator = match lock(state) {
        Ok(allocator) => allocator,
        Err(_) => return,
    };
    // SAFETY: the non-copy owner releases this allocator-issued offset once.
    let Ok(header) = (unsafe { read_header(state, offset) }) else {
        return;
    };
    let Some(start) = offset
        .checked_sub(size_of::<AllocationHeader>() as u32)
        .and_then(|header_offset| header_offset.checked_sub(header.prefix))
    else {
        return;
    };
    let Some(live_bytes) = allocator.live_bytes.checked_sub(header.block_len) else {
        return;
    };
    if insert_free(state, &mut allocator, start, header.block_len).is_ok() {
        allocator.live_bytes = live_bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_state(capacity: usize) -> CageState {
        let layout = Layout::from_size_align(capacity, 8).unwrap();
        // SAFETY: the test uses a nonzero, valid layout and `CageState` owns it.
        let memory = NonNull::new(unsafe { alloc(layout) }).unwrap();
        CageState {
            capacity,
            memory,
            allocator: Mutex::new(Allocator {
                cursor: INITIAL_CURSOR,
                live_bytes: 0,
                free_head: 0,
            }),
        }
    }

    #[test]
    fn allocation_representation_and_intrusive_free_ranges() {
        assert!(size_of::<AllocationHeader>() <= 16);
        assert_eq!(size_of::<CageAllocation<u64>>(), 4);
        assert_eq!(size_of::<Option<CageAllocation<u64>>>(), 4);

        let state = local_state(4096);
        let mut allocator = lock(&state).unwrap();
        let mut starts = [0_u32; 4];
        let mut lengths = [0_u32; 4];
        for index in 0..4 {
            starts[index] = allocator.cursor;
            let (_, _, len) = allocate_block(&state, &mut allocator, 32, 8).unwrap();
            lengths[index] = len;
        }

        for index in [0, 2, 1] {
            allocator.live_bytes -= lengths[index];
            insert_free(&state, &mut allocator, starts[index], lengths[index]).unwrap();
        }
        let combined = unsafe { read_free_node(&state, starts[0]).unwrap() };
        assert_eq!(combined.len, lengths[0] + lengths[1] + lengths[2]);
        assert_eq!(combined.next, 0);

        let (_, _, split_len) = allocate_block(&state, &mut allocator, 16, 8).unwrap();
        assert_eq!(allocator.free_head, starts[0] + split_len);
        let split = unsafe { read_free_node(&state, allocator.free_head).unwrap() };
        assert_eq!(split.len, combined.len - split_len);

        allocator.live_bytes -= split_len;
        insert_free(&state, &mut allocator, starts[0], split_len).unwrap();
        let combined = unsafe { read_free_node(&state, starts[0]).unwrap() };
        let prefix = block_layout(state.base(), starts[0], 1, 8).unwrap().1;
        let exact_bytes = combined.len - prefix - size_of::<AllocationHeader>() as u32;
        let old_cursor = allocator.cursor;
        let (_, _, exact_len) =
            allocate_block(&state, &mut allocator, exact_bytes as usize, 8).unwrap();
        assert_eq!(exact_len, combined.len);
        assert_eq!(allocator.free_head, 0);
        assert_eq!(allocator.cursor, old_cursor);

        allocator.live_bytes -= exact_len;
        insert_free(&state, &mut allocator, starts[0], exact_len).unwrap();
        allocator.live_bytes -= lengths[3];
        insert_free(&state, &mut allocator, starts[3], lengths[3]).unwrap();
        assert_eq!(allocator.cursor, starts[0]);
        assert_eq!(allocator.free_head, 0);
        assert_eq!(allocator.live_bytes, 0);
    }
}
