//! Process-wide cage and four-byte allocation owners.

use compact_core::{
    checked_align_up, CompactValue, Error, Offset32, Result, MAX_CAGE_BYTES, MIN_CAGE_BYTES,
};
use core::marker::PhantomData;
use core::mem::{align_of, size_of, MaybeUninit};
use core::slice;
use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, MutexGuard, OnceLock};

const HEADER_MAGIC: u64 = 0x4341_4745_5632_3301;

#[repr(C)]
#[derive(Clone, Copy)]
struct AllocationHeader {
    magic: u64,
    block_start: u32,
    prefix: u32,
    block_len: u64,
    capacity: u32,
    initialized: u32,
}

struct Allocator {
    cursor: usize,
    live_bytes: usize,
    free: BTreeMap<usize, usize>,
    live_offsets: HashSet<u32>,
}

struct CageState {
    capacity: usize,
    memory: Box<[MaybeUninit<u8>]>,
    allocator: Mutex<Allocator>,
}

impl CageState {
    fn base(&self) -> *mut u8 {
        self.memory.as_ptr().cast_mut().cast::<u8>()
    }
}

// SAFETY: the allocation is stable for the process lifetime; all allocator
// metadata is protected by its mutex and access to individual live values is
// governed by unique owners and Rust borrows.
unsafe impl Send for CageState {}
unsafe impl Sync for CageState {}

static CAGE: OnceLock<CageState> = OnceLock::new();

/// Configuration for the one process-wide compact cage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CageConfig {
    /// Requested cage capacity in bytes.
    pub capacity: usize,
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
        if config.capacity as u64 > MAX_CAGE_BYTES {
            return Err(Error::CageTooLarge);
        }
        let mut memory = Vec::<MaybeUninit<u8>>::new();
        memory
            .try_reserve_exact(config.capacity)
            .map_err(|_| Error::AllocationFailed)?;
        // SAFETY: every bit pattern is valid for `MaybeUninit<u8>`, and the
        // vector reserved at least the requested number of elements above.
        unsafe { memory.set_len(config.capacity) };
        let memory = memory.into_boxed_slice();
        let state = CageState {
            capacity: config.capacity,
            memory,
            allocator: Mutex::new(Allocator {
                cursor: 8,
                live_bytes: 0,
                free: BTreeMap::new(),
                live_offsets: HashSet::new(),
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
        Ok(lock(state()?)?.live_bytes)
    }

    /// Return capacity not currently occupied by live allocation blocks.
    pub fn remaining_bytes() -> Result<usize> {
        let state = state()?;
        Ok(state.capacity.saturating_sub(lock(state)?.live_bytes))
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
        allocation.header()?;
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
        let header = unsafe { read_header(state.base(), offset.as_u32()) }?;
        if header.initialized == 0 {
            return Err(Error::InitializationError);
        }
        // SAFETY: the caller upholds the lifetime and provenance contract.
        Ok(unsafe { &*state.base().add(offset.as_u32() as usize).cast::<T>() })
    }

    /// Resolve an offset to a byte range.
    ///
    /// # Safety
    ///
    /// The range must be wholly inside a live byte allocation kept alive by
    /// an owner for the returned lifetime.
    pub unsafe fn resolve_bytes_unchecked<'a>(offset: u32, len: usize) -> Result<&'a [u8]> {
        let state = state()?;
        let header = unsafe { read_header(state.base(), offset) }?;
        if len > header.capacity as usize {
            return Err(Error::OutOfBounds);
        }
        // SAFETY: the caller guarantees liveness and initialization; the
        // allocation header bounds the requested byte range.
        Ok(unsafe { slice::from_raw_parts(state.base().add(offset as usize), len) })
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

        let mut selected = None;
        for (&start, &available) in &allocator.free {
            if let Some(layout) = block_layout(state.base(), start, needed, alignment)? {
                if layout.2 <= available {
                    selected = Some((start, available, layout));
                    break;
                }
            }
        }

        let (data_offset, block_start, block_len) =
            if let Some((start, available, (data, _prefix, len))) = selected {
                allocator.free.remove(&start);
                if available > len {
                    allocator.free.insert(start + len, available - len);
                }
                (data, start, len)
            } else {
                let start = allocator.cursor;
                let (data, _prefix, len) = block_layout(state.base(), start, needed, alignment)?
                    .ok_or(Error::AllocationExhausted)?;
                let end = start.checked_add(len).ok_or(Error::OffsetOverflow)?;
                if end > state.capacity || end as u64 > MAX_CAGE_BYTES {
                    return Err(Error::AllocationExhausted);
                }
                allocator.cursor = end;
                (data, start, len)
            };
        let offset = u32::try_from(data_offset).map_err(|_| Error::OffsetOverflow)?;
        if offset == 0 {
            return Err(Error::InvalidOffset);
        }
        let header = AllocationHeader {
            magic: HEADER_MAGIC,
            block_start: u32::try_from(block_start).map_err(|_| Error::OffsetOverflow)?,
            prefix: u32::try_from(data_offset - size_of::<AllocationHeader>() - block_start)
                .map_err(|_| Error::OffsetOverflow)?,
            block_len: len_as_u64(block_len)?,
            capacity: capacity_u32,
            initialized: 0,
        };
        // SAFETY: block layout aligns the header and reserves the whole block.
        unsafe {
            state
                .base()
                .add(data_offset - size_of::<AllocationHeader>())
                .cast::<AllocationHeader>()
                .write(header)
        };
        allocator.live_offsets.insert(offset);
        allocator.live_bytes = allocator
            .live_bytes
            .checked_add(block_len)
            .ok_or(Error::OffsetOverflow)?;
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
        // SAFETY: the owner tracks the initialized prefix and remains borrowed.
        unsafe { slice::from_raw_parts(self.data_ptr(), self.len()) }
    }
    /// Mutably borrow the initialized prefix.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        let len = self.len();
        // SAFETY: the unique owner is mutably borrowed.
        unsafe { slice::from_raw_parts_mut(self.data_ptr(), len) }
    }
    /// Return an initialized element by index.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.as_slice().get(index)
    }
    /// Mutably borrow an initialized element by index.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.as_mut_slice().get_mut(index)
    }
    /// Initialize the next element slot.
    pub fn push(&mut self, value: T) -> Result<()> {
        let header = self.header()?;
        if header.initialized >= header.capacity {
            return Err(Error::OutOfBounds);
        }
        let index = header.initialized as usize;
        // SAFETY: this is the next uninitialized slot within the owner.
        unsafe { self.data_ptr().add(index).write(value) };
        self.set_initialized(index as u32 + 1)?;
        Ok(())
    }
    /// Remove and return the final initialized element.
    pub fn pop(&mut self) -> Option<T> {
        let len = self.len();
        if len == 0 {
            return None;
        }
        self.set_initialized((len - 1) as u32)
            .expect("live cage owner header");
        // SAFETY: the element was initialized and removed from the drop prefix.
        Some(unsafe { self.data_ptr().add(len - 1).read() })
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
        while allocation.len() > new_len {
            let index = allocation.len() - 1;
            allocation
                .set_initialized(index as u32)
                .expect("live cage owner header");
            // SAFETY: length was lowered first, so unwinding cannot double-drop this element.
            unsafe { allocation.data_ptr().add(index).drop_in_place() };
        }
        guard.armed = false;
    }
    /// Append a copyable slice to the initialized prefix.
    pub fn extend_copy(&mut self, values: &[T]) -> Result<()>
    where
        T: Copy,
    {
        let start = self.len();
        let end = start
            .checked_add(values.len())
            .ok_or(Error::OffsetOverflow)?;
        if end > self.capacity() {
            return Err(Error::OutOfBounds);
        }
        if !values.is_empty() {
            // SAFETY: destination is uninitialized and the source is valid.
            unsafe {
                self.data_ptr()
                    .add(start)
                    .copy_from_nonoverlapping(values.as_ptr(), values.len())
            };
        }
        self.set_initialized(end as u32)
    }
    /// Move the initialized prefix into an empty owner of the same type.
    pub fn move_into(&mut self, destination: &mut Self) -> Result<()> {
        if !destination.is_empty() {
            return Err(Error::InitializationError);
        }
        let len = self.len();
        if len > destination.capacity() {
            return Err(Error::OutOfBounds);
        }
        self.set_initialized(0)?;
        for index in 0..len {
            // SAFETY: source is read once and destination is uninitialized.
            unsafe {
                destination
                    .data_ptr()
                    .add(index)
                    .write(self.data_ptr().add(index).read())
            };
        }
        destination.set_initialized(len as u32)
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
        let start = self.len();
        let end = start.checked_add(len).ok_or(Error::OffsetOverflow)?;
        if end > self.capacity() {
            return Err(Error::OutOfBounds);
        }
        for index in 0..len {
            // SAFETY: guaranteed by the caller; the destination is in capacity.
            let value = unsafe { source.add(index).cast::<T>().read() };
            unsafe { self.data_ptr().add(start + index).write(value) };
        }
        self.set_initialized(end as u32)
    }
    /// Try to change capacity without relocating the allocation.
    pub fn try_resize(&mut self, capacity: usize) -> Result<bool> {
        let requested = u32::try_from(capacity).map_err(|_| Error::OffsetOverflow)?;
        let header = self.header()?;
        if requested < header.initialized {
            return Err(Error::InitializationError);
        }
        let bytes = size_of::<T>()
            .checked_mul(capacity)
            .ok_or(Error::OffsetOverflow)?
            .max(1);
        let state = state()?;
        let mut allocator = lock(state)?;
        let start = header.block_start as usize;
        let old_len = usize::try_from(header.block_len).map_err(|_| Error::OffsetOverflow)?;
        let prefix = header.prefix as usize;
        let new_len = checked_align_up(
            prefix
                .checked_add(size_of::<AllocationHeader>())
                .and_then(|n| n.checked_add(bytes))
                .ok_or(Error::OffsetOverflow)?,
            8,
        )?;
        if new_len <= old_len {
            if new_len < old_len {
                insert_free(&mut allocator, start + new_len, old_len - new_len);
            }
            allocator.live_bytes = allocator.live_bytes - old_len + new_len;
            let mut changed = header;
            changed.block_len = len_as_u64(new_len)?;
            changed.capacity = requested;
            self.write_header(changed);
            return Ok(true);
        }
        let end = start.checked_add(old_len).ok_or(Error::OffsetOverflow)?;
        let extra = new_len - old_len;
        if end == allocator.cursor {
            if allocator
                .cursor
                .checked_add(extra)
                .ok_or(Error::OffsetOverflow)?
                > state.capacity
            {
                return Ok(false);
            }
            allocator.cursor += extra;
        } else if let Some(&free_len) = allocator.free.get(&end) {
            if free_len < extra {
                return Ok(false);
            }
            allocator.free.remove(&end);
            if free_len > extra {
                allocator.free.insert(end + extra, free_len - extra);
            }
        } else {
            return Ok(false);
        }
        allocator.live_bytes = allocator
            .live_bytes
            .checked_add(extra)
            .ok_or(Error::OffsetOverflow)?;
        let mut changed = header;
        changed.block_len = len_as_u64(new_len)?;
        changed.capacity = requested;
        self.write_header(changed);
        Ok(true)
    }
    /// Borrow the full capacity as potentially uninitialized slots.
    pub fn uninit_capacity(&self) -> &[MaybeUninit<T>] {
        // SAFETY: MaybeUninit permits reading every slot state.
        unsafe { slice::from_raw_parts(self.data_ptr().cast::<MaybeUninit<T>>(), self.capacity()) }
    }
    /// Mutably borrow the full capacity as potentially uninitialized slots.
    pub fn uninit_capacity_mut(&mut self) -> &mut [MaybeUninit<T>] {
        // SAFETY: the unique owner is mutably borrowed and every capacity slot is writable.
        unsafe {
            slice::from_raw_parts_mut(self.data_ptr().cast::<MaybeUninit<T>>(), self.capacity())
        }
    }
    fn raw_offset(&self) -> u32 {
        self.offset.0.get()
    }
    fn data_ptr(&self) -> *mut T {
        // SAFETY: offset comes only from this allocator and the process cage is never moved.
        unsafe {
            state()
                .expect("initialized compact runtime")
                .base()
                .add(self.raw_offset() as usize)
                .cast::<T>()
        }
    }
    fn header(&self) -> Result<AllocationHeader> {
        // SAFETY: this non-copy owner represents an allocator-issued offset.
        unsafe { read_header(state()?.base(), self.raw_offset()) }
    }
    fn write_header(&self, header: AllocationHeader) {
        // SAFETY: this owner represents the live allocation whose header is updated.
        unsafe {
            state()
                .expect("initialized compact runtime")
                .base()
                .add(self.raw_offset() as usize - size_of::<AllocationHeader>())
                .cast::<AllocationHeader>()
                .write(header)
        }
    }
    fn set_initialized(&mut self, initialized: u32) -> Result<()> {
        let mut header = self.header()?;
        if initialized > header.capacity {
            return Err(Error::InitializationError);
        }
        header.initialized = initialized;
        self.write_header(header);
        Ok(())
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
fn len_as_u64(value: usize) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::OffsetOverflow)
}

fn block_layout(
    base: *mut u8,
    start: usize,
    bytes: usize,
    alignment: usize,
) -> Result<Option<(usize, usize, usize)>> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(Error::AlignmentError);
    }
    let after_header = (base as usize)
        .checked_add(start)
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
        .checked_sub(start)
        .ok_or(Error::OffsetOverflow)?;
    let raw = prefix
        .checked_add(size_of::<AllocationHeader>())
        .and_then(|n| n.checked_add(bytes))
        .ok_or(Error::OffsetOverflow)?;
    let len = checked_align_up(raw, 8)?;
    Ok(Some((data, prefix, len)))
}

unsafe fn read_header(base: *mut u8, offset: u32) -> Result<AllocationHeader> {
    let header_offset = (offset as usize)
        .checked_sub(size_of::<AllocationHeader>())
        .ok_or(Error::InvalidOffset)?;
    // SAFETY: caller supplies an allocator-issued offset or upholds its unsafe contract.
    let header = unsafe { base.add(header_offset).cast::<AllocationHeader>().read() };
    if header.magic != HEADER_MAGIC {
        return Err(Error::InvalidOffset);
    }
    Ok(header)
}

fn insert_free(allocator: &mut Allocator, mut start: usize, mut len: usize) {
    if len == 0 {
        return;
    }
    if let Some((&previous_start, &previous_len)) = allocator.free.range(..=start).next_back() {
        let previous_end = previous_start + previous_len;
        if previous_end == start {
            allocator.free.remove(&previous_start);
            start = previous_start;
            len += previous_len;
        }
    }
    if let Some((&next_start, &next_len)) = allocator.free.range(start..).next() {
        if start + len == next_start {
            allocator.free.remove(&next_start);
            len += next_len;
        }
    }
    allocator.free.insert(start, len);
    while let Some((&tail_start, &tail_len)) = allocator.free.iter().next_back() {
        if tail_start + tail_len != allocator.cursor {
            break;
        }
        allocator.cursor = tail_start;
        allocator.free.remove(&tail_start);
    }
}

fn release(offset: u32) {
    let Ok(state) = state() else {
        return;
    };
    let Ok(mut allocator) = lock(state) else {
        return;
    };
    if !allocator.live_offsets.remove(&offset) {
        return;
    }
    // SAFETY: live offset was registered on allocation and owners release once.
    let Ok(header) = (unsafe { read_header(state.base(), offset) }) else {
        return;
    };
    let len = header.block_len as usize;
    allocator.live_bytes = allocator.live_bytes.saturating_sub(len);
    insert_free(&mut allocator, header.block_start as usize, len);
}
