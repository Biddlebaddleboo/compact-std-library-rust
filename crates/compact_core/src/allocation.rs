//! Reusable arena allocation state and typed allocation owners.

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::{align_of, size_of, MaybeUninit};
use core::ptr::{self, NonNull};
use core::slice;

use crate::{checked_align_up, Error, Result, MAX_ARENA_BYTES};

const BLOCK_ALIGNMENT: usize = 4;
const FREE_NODE_BYTES: usize = 8;
const PERSISTENT_ARENA_MAGIC: u64 = 0x434F_4D50_4143_5432;
const PERSISTENT_ARENA_VALID: u32 = 0xA11A_2E02;

#[repr(C)]
#[derive(Clone, Copy)]
struct FreeNode {
    len: u32,
    next: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AllocationHeader {
    block_len: u32,
    prefix: u32,
    capacity: u32,
    initialized: u32,
}

struct ArenaInner {
    base: *mut u8,
    capacity: usize,
    cursor: usize,
    free_head: u32,
    next_id: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PersistentArenaHeader {
    magic: u64,
    abi_major: u16,
    abi_minor: u16,
    capacity: u64,
    validity: u32,
}

/// Stable arena-local allocator state stored at the start of the backing.
///
/// Wrappers keep a pointer to this object so their drop glue can reclaim arena
/// storage even if the stack `Arena` value itself moves.
#[repr(C)]
pub(crate) struct ArenaState {
    inner: UnsafeCell<ArenaInner>,
}

impl ArenaState {
    /// # Safety
    ///
    /// `base` must point to `capacity` writable bytes that remain allocated
    /// at a stable address for the full arena scope.
    pub(crate) unsafe fn place(
        base: *mut core::mem::MaybeUninit<u8>,
        capacity: usize,
    ) -> Result<(NonNull<Self>, usize)> {
        let base = base.cast::<u8>();
        let (state_offset, cursor) = state_layout(base, capacity, 1)?;
        Self::place_at(base, capacity, state_offset, cursor)
    }

    /// # Safety
    ///
    /// `base` must point to `capacity` writable bytes that remain allocated
    /// at a stable address for the full arena scope.
    pub(crate) unsafe fn place_persistent(
        base: *mut core::mem::MaybeUninit<u8>,
        capacity: usize,
    ) -> Result<(NonNull<Self>, usize)> {
        let base = base.cast::<u8>();
        let header_len = size_of::<PersistentArenaHeader>();
        let (state_offset, cursor) = state_layout(base, capacity, header_len)?;
        let header = PersistentArenaHeader {
            magic: PERSISTENT_ARENA_MAGIC,
            abi_major: crate::ABI_VERSION.major,
            abi_minor: crate::ABI_VERSION.minor,
            capacity: capacity as u64,
            validity: PERSISTENT_ARENA_VALID,
        };
        // SAFETY: the prefix is reserved by `state_layout`; unaligned access
        // is used because byte backings only promise alignment one.
        unsafe { base.cast::<PersistentArenaHeader>().write_unaligned(header) };
        Self::place_at(base, capacity, state_offset, cursor)
    }

    fn place_at(
        base: *mut u8,
        capacity: usize,
        state_offset: usize,
        cursor: usize,
    ) -> Result<(NonNull<Self>, usize)> {
        let state_ptr = unsafe { base.add(state_offset).cast::<Self>() };
        // SAFETY: `state_layout` established alignment and that the complete
        // object fits inside the exclusively borrowed backing prefix.
        unsafe {
            state_ptr.write(Self {
                inner: UnsafeCell::new(ArenaInner {
                    base,
                    capacity,
                    cursor,
                    free_head: 0,
                    next_id: 1,
                }),
            });
        }
        // SAFETY: state_ptr is non-null and points at the initialized object.
        Ok((unsafe { NonNull::new_unchecked(state_ptr) }, cursor))
    }

    /// # Safety
    ///
    /// `base` must point to the exact stable backing previously initialized
    /// by `place_persistent`, with no other arena attached at the same time.
    pub(crate) unsafe fn attach_persistent(
        base: *mut core::mem::MaybeUninit<u8>,
        capacity: usize,
    ) -> Result<(NonNull<Self>, usize)> {
        let base = base.cast::<u8>();
        if capacity < size_of::<PersistentArenaHeader>() {
            return Err(Error::InitializationError);
        }
        // SAFETY: the caller guarantees persistent state was initialized in
        // this backing before attach; unaligned access handles any byte base.
        let header = unsafe { base.cast::<PersistentArenaHeader>().read_unaligned() };
        if header.magic != PERSISTENT_ARENA_MAGIC
            || header.validity != PERSISTENT_ARENA_VALID
            || header.capacity != capacity as u64
        {
            return Err(Error::InitializationError);
        }
        if header.abi_major != crate::ABI_VERSION.major
            || header.abi_minor != crate::ABI_VERSION.minor
        {
            return Err(Error::InitializationError);
        }
        let (state_offset, cursor) =
            state_layout(base, capacity, size_of::<PersistentArenaHeader>())?;
        // SAFETY: persistent attach callers guarantee this exact region was
        // initialized by `place_persistent` and has not moved or been aliased.
        let state_ptr = unsafe { base.add(state_offset).cast::<Self>() };
        // SAFETY: `state_ptr` is aligned and points inside the backing; the
        // attach precondition guarantees that a complete ArenaState is live.
        let state = unsafe { &*state_ptr };
        // SAFETY: the checked persistent header identifies an initialized
        // ArenaState, whose inner fields are verified before reuse.
        // SAFETY: the attach precondition guarantees no other Arena is active
        // for this backing, so the allocator state can be mutably rebound to
        // the provenance of the current StableBacking borrow.
        let inner = unsafe { &mut *state.inner.get() };
        if inner.base != base
            || inner.capacity != capacity
            || inner.cursor < cursor
            || inner.cursor > capacity
            || inner.cursor as u64 > MAX_ARENA_BYTES
        {
            return Err(Error::InitializationError);
        }
        // Persistent backing may be borrowed again through a fresh mutable
        // reference. Keep the stored raw pointer tied to that current borrow.
        inner.base = base;
        validate_free_list(inner, cursor)?;
        // SAFETY: the address is non-null and was computed from a nonempty
        // mutable slice.
        Ok((unsafe { NonNull::new_unchecked(state_ptr) }, cursor))
    }

    fn allocate(
        &self,
        bytes: usize,
        alignment: usize,
        capacity: u32,
        initialized: u32,
        owned: bool,
    ) -> Result<(u32, u32)> {
        if alignment == 0 || !alignment.is_power_of_two() {
            return Err(Error::AlignmentError);
        }
        if initialized > capacity {
            return Err(Error::InitializationError);
        }
        let inner = unsafe { &mut *self.inner.get() };
        let id = if owned {
            let id = inner.next_id;
            if id == 0 || id == u32::MAX {
                return Err(Error::AllocationIdExhausted);
            }
            id
        } else {
            0
        };

        let reserve_bytes = bytes.max(1);
        let mut previous = 0_u32;
        let mut current = inner.free_head;
        while current != 0 {
            let node = unsafe { read_free_node(inner.base, current) };
            let Some((data_offset, prefix, required)) =
                allocation_layout(inner.base, current as usize, reserve_bytes, alignment)?
            else {
                previous = current;
                current = node.next;
                continue;
            };
            if required > node.len as usize {
                previous = current;
                current = node.next;
                continue;
            }

            let remainder = node.len as usize - required;
            let (block_len, replacement_link) = if remainder >= FREE_NODE_BYTES {
                let remainder_start = current as usize + required;
                let remainder_start_u32 =
                    u32::try_from(remainder_start).map_err(|_| Error::OffsetOverflow)?;
                unsafe {
                    write_free_node(inner.base, remainder_start_u32, remainder as u32, node.next);
                }
                (required, remainder_start_u32)
            } else {
                (node.len as usize, node.next)
            };
            if previous == 0 {
                inner.free_head = replacement_link;
            } else {
                unsafe { write_free_next(inner.base, previous, replacement_link) };
            }
            write_allocation_header(
                inner.base,
                data_offset,
                AllocationHeader {
                    block_len: u32::try_from(block_len).map_err(|_| Error::OffsetOverflow)?,
                    prefix: u32::try_from(prefix).map_err(|_| Error::OffsetOverflow)?,
                    capacity,
                    initialized,
                },
            );
            if owned {
                inner.next_id += 1;
            }
            return Ok((data_offset, id));
        }

        let start = inner.cursor;
        let (data_offset, prefix, required) =
            allocation_layout(inner.base, start, reserve_bytes, alignment)?
                .ok_or(Error::AllocationExhausted)?;
        let end = start.checked_add(required).ok_or(Error::OffsetOverflow)?;
        if end > inner.capacity || end as u64 > MAX_ARENA_BYTES {
            return Err(Error::AllocationExhausted);
        }
        inner.cursor = end;
        write_allocation_header(
            inner.base,
            data_offset,
            AllocationHeader {
                block_len: u32::try_from(required).map_err(|_| Error::OffsetOverflow)?,
                prefix: u32::try_from(prefix).map_err(|_| Error::OffsetOverflow)?,
                capacity,
                initialized,
            },
        );
        if owned {
            inner.next_id += 1;
        }
        Ok((data_offset, id))
    }

    fn try_resize(
        &self,
        offset: u32,
        bytes: usize,
        capacity: u32,
        alignment: usize,
    ) -> Result<bool> {
        let inner = unsafe { &mut *self.inner.get() };
        let mut header = read_allocation_header(inner.base, offset)?;
        if alignment == 0 || !alignment.is_power_of_two() {
            return Err(Error::AlignmentError);
        }
        let address = (inner.base as usize)
            .checked_add(offset as usize)
            .ok_or(Error::OffsetOverflow)?;
        if address % alignment != 0 {
            return Err(Error::AlignmentError);
        }
        if capacity < header.initialized {
            return Err(Error::InitializationError);
        }
        let header_size = size_of::<AllocationHeader>();
        let header_offset = offset as usize - header_size;
        let block_start = header_offset
            .checked_sub(header.prefix as usize)
            .ok_or(Error::InvalidOffset)?;
        let required = aligned_block_len(header.prefix as usize, header_size, bytes.max(1))?;
        let old_len = header.block_len as usize;

        if required <= old_len {
            let released = old_len - required;
            if released >= FREE_NODE_BYTES {
                let tail_start = block_start + required;
                header.block_len = u32::try_from(required).map_err(|_| Error::OffsetOverflow)?;
                write_allocation_header(inner.base, offset, header);
                insert_free(inner, tail_start, released)?;
            }
            header.capacity = capacity;
            write_allocation_header(inner.base, offset, header);
            return Ok(true);
        }

        let block_end = block_start
            .checked_add(old_len)
            .ok_or(Error::OffsetOverflow)?;
        let extension = required - old_len;
        if block_end == inner.cursor {
            let end = inner
                .cursor
                .checked_add(extension)
                .ok_or(Error::OffsetOverflow)?;
            if end > inner.capacity || end as u64 > MAX_ARENA_BYTES {
                return Ok(false);
            }
            inner.cursor = end;
            header.block_len = u32::try_from(required).map_err(|_| Error::OffsetOverflow)?;
            header.capacity = capacity;
            write_allocation_header(inner.base, offset, header);
            return Ok(true);
        }

        if let Some((previous, node_offset, node)) = find_free_node(inner, block_end as u32) {
            let node_len = node.len as usize;
            if node_len >= extension {
                let remainder = node_len - extension;
                if remainder >= FREE_NODE_BYTES {
                    let new_start = node_offset as usize + extension;
                    let new_start = u32::try_from(new_start).map_err(|_| Error::OffsetOverflow)?;
                    unsafe {
                        write_free_node(inner.base, new_start, remainder as u32, node.next);
                    }
                    replace_free_link(inner, previous, new_start);
                    header.block_len =
                        u32::try_from(required).map_err(|_| Error::OffsetOverflow)?;
                } else {
                    replace_free_link(inner, previous, node.next);
                    header.block_len = header
                        .block_len
                        .checked_add(node.len)
                        .ok_or(Error::OffsetOverflow)?;
                }
                header.capacity = capacity;
                write_allocation_header(inner.base, offset, header);
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn release(&self, offset: u32, _id: u32) {
        let inner = unsafe { &mut *self.inner.get() };
        let Ok(header) = read_allocation_header(inner.base, offset) else {
            return;
        };
        let header_size = size_of::<AllocationHeader>();
        let Some(header_offset) = (offset as usize).checked_sub(header_size) else {
            return;
        };
        let Some(start) = header_offset.checked_sub(header.prefix as usize) else {
            return;
        };
        let len = header.block_len as usize;
        if len < FREE_NODE_BYTES
            || start
                .checked_add(len)
                .map_or(true, |end| end > inner.cursor)
        {
            return;
        }
        let _ = insert_free(inner, start, len);
    }

    fn is_free(&self, start: usize, end: usize) -> bool {
        let inner = unsafe { &*self.inner.get() };
        let mut current = inner.free_head;
        while current != 0 {
            let node = unsafe { read_free_node(inner.base, current) };
            let free_start = current as usize;
            let free_end = free_start + node.len as usize;
            if start < free_end && free_start < end {
                return true;
            }
            current = node.next;
        }
        false
    }

    pub(crate) fn used_bytes(&self) -> usize {
        unsafe { (*self.inner.get()).cursor }
    }

    pub(crate) fn remaining_bytes(&self) -> usize {
        let inner = unsafe { &*self.inner.get() };
        let mut total = inner.capacity.saturating_sub(inner.cursor);
        let mut current = inner.free_head;
        while current != 0 {
            let node = unsafe { read_free_node(inner.base, current) };
            total = total.saturating_add(node.len as usize);
            current = node.next;
        }
        total
    }

    #[cfg(test)]
    pub(crate) fn debug_validate_allocator(&self, live_ranges: &[(usize, usize)]) -> Result<()> {
        // SAFETY: the test calls this only between arena operations, with no
        // concurrent access to the single-owner allocator state.
        let inner = unsafe { &*self.inner.get() };
        let (_, minimum_cursor) = state_layout(inner.base, inner.capacity, 1)?;
        if inner.cursor < minimum_cursor || inner.cursor > inner.capacity {
            return Err(Error::InitializationError);
        }
        validate_free_list(inner, minimum_cursor)?;

        for &(start, end) in live_ranges {
            if start < minimum_cursor || start >= end || end > inner.cursor {
                return Err(Error::InitializationError);
            }
        }

        let mut current = inner.free_head;
        while current != 0 {
            let node = unsafe { read_free_node(inner.base, current) };
            let free_start = current as usize;
            let free_end = free_start
                .checked_add(node.len as usize)
                .ok_or(Error::InitializationError)?;
            if live_ranges
                .iter()
                .any(|&(live_start, live_end)| live_start < free_end && free_start < live_end)
            {
                return Err(Error::InitializationError);
            }
            current = node.next;
        }
        Ok(())
    }
}

/// A value that may safely live in and move between compact arena slots.
///
/// # Safety
///
/// Implementors must be valid at their ordinary Rust alignment in arena
/// storage, and moving a value with `ptr::read`/`ptr::write` to another slot
/// must preserve its invariants. In particular, the value must not depend on
/// its own address, require pinning, or contain a native reference whose
/// lifetime is not represented by its Rust type. Its destructor must be safe
/// to run while the containing arena backing is alive. Users may implement
/// this trait only when all of these conditions hold.
pub unsafe trait CompactValue {}

macro_rules! compact_values {
    ($($ty:ty),* $(,)?) => { $(unsafe impl CompactValue for $ty {})* };
}

compact_values!(
    (),
    bool,
    char,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    f32,
    f64
);

unsafe impl<T: CompactValue, const N: usize> CompactValue for [T; N] {}
// SAFETY: MaybeUninit can hold any bit pattern and moving it preserves every
// initialized T value; its own destructor intentionally does not drop T.
unsafe impl<T: CompactValue> CompactValue for MaybeUninit<T> {}
unsafe impl<T: CompactValue> CompactValue for Option<T> {}
unsafe impl<T: CompactValue, E: CompactValue> CompactValue for core::result::Result<T, E> {}
unsafe impl<A: CompactValue, B: CompactValue> CompactValue for (A, B) {}
unsafe impl<A: CompactValue, B: CompactValue, C: CompactValue> CompactValue for (A, B, C) {}
unsafe impl<A: CompactValue, B: CompactValue, C: CompactValue, D: CompactValue> CompactValue
    for (A, B, C, D)
{
}

/// A unique owner of one typed allocation inside an arena.
///
/// The owner is deliberately non-`Copy`. Its drop glue destroys every live
/// value in the initialized prefix and returns the allocation to the arena's
/// reusable free-range list.
#[must_use = "dropping this owner releases its arena allocation"]
pub struct ArenaAllocation<'arena, T: CompactValue> {
    state: NonNull<ArenaState>,
    offset: u32,
    id: u32,
    marker: PhantomData<fn(&'arena mut ()) -> &'arena mut ()>,
    not_send_sync: PhantomData<*mut T>,
}

impl<'arena, T: CompactValue> ArenaAllocation<'arena, T> {
    pub(crate) fn new(state: NonNull<ArenaState>, offset: u32, id: u32) -> Self {
        Self {
            state,
            offset,
            id,
            marker: PhantomData,
            not_send_sync: PhantomData,
        }
    }

    /// Return the number of initialized elements.
    pub fn len(&self) -> usize {
        self.header().initialized as usize
    }

    /// Return the number of element slots in this allocation.
    pub fn capacity(&self) -> usize {
        self.header().capacity as usize
    }

    /// Return whether the initialized prefix is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return the allocator-issued identity used by owner-checked handles.
    #[doc(hidden)]
    pub fn allocation_id(&self) -> u32 {
        self.id
    }

    /// Borrow the initialized prefix.
    #[doc(hidden)]
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: the allocator header tracks an initialized prefix, and the
        // unique owner is immutably borrowed for the returned view.
        unsafe { slice::from_raw_parts(self.data_ptr(), self.len()) }
    }

    /// Mutably borrow the initialized prefix.
    #[doc(hidden)]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        let len = self.len();
        // SAFETY: the allocation has one non-cloneable owner and is exclusively
        // borrowed for the returned view.
        unsafe { slice::from_raw_parts_mut(self.data_ptr(), len) }
    }

    /// Borrow an initialized element by index.
    #[doc(hidden)]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.as_slice().get(index)
    }

    /// Mutably borrow an initialized element by index.
    #[doc(hidden)]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.as_mut_slice().get_mut(index)
    }

    /// Initialize the next slot, advancing the initialized prefix on success.
    #[doc(hidden)]
    pub fn push(&mut self, value: T) -> Result<()> {
        let header = self.header();
        if header.initialized >= header.capacity {
            return Err(Error::OutOfBounds);
        }
        let index = header.initialized as usize;
        // SAFETY: index is an uninitialized slot within the owned allocation.
        unsafe { self.data_ptr().add(index).write(value) };
        self.set_initialized(index as u32 + 1);
        Ok(())
    }

    /// Pop and return the last initialized element.
    #[doc(hidden)]
    pub fn pop(&mut self) -> Option<T> {
        let len = self.len();
        if len == 0 {
            return None;
        }
        self.set_initialized((len - 1) as u32);
        // SAFETY: the former final slot was initialized and is no longer part
        // of the owner's drop prefix.
        Some(unsafe { self.data_ptr().add(len - 1).read() })
    }

    /// Drop initialized elements until `new_len` remains.
    #[doc(hidden)]
    pub fn truncate(&mut self, new_len: usize) {
        let mut guard = TruncateGuard {
            allocation: self,
            new_len,
            armed: true,
        };
        while self.len() > new_len {
            let index = self.len() - 1;
            self.set_initialized(index as u32);
            // SAFETY: the length was reduced first, so unwinding from T::drop
            // cannot cause this element to be dropped twice.
            unsafe { self.data_ptr().add(index).drop_in_place() };
        }
        guard.armed = false;
    }

    /// Append a copyable initialized slice to the current prefix.
    #[doc(hidden)]
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
        if values.is_empty() {
            return Ok(());
        }
        // SAFETY: the destination is the uninitialized suffix, values is a
        // valid source slice, and Copy values have no move/drop obligations.
        unsafe {
            self.data_ptr()
                .add(start)
                .copy_from_nonoverlapping(values.as_ptr(), values.len());
        }
        self.set_initialized(end as u32);
        Ok(())
    }

    /// Move the initialized prefix into an empty allocation of the same type.
    #[doc(hidden)]
    pub fn move_into(&mut self, destination: &mut Self) -> Result<()> {
        if self.state != destination.state || !destination.is_empty() {
            return Err(Error::InitializationError);
        }
        let len = self.len();
        if len > destination.capacity() {
            return Err(Error::OutOfBounds);
        }
        if len == 0 {
            return Ok(());
        }
        // All validation is complete. Raw reads and writes cannot fail, so
        // ownership transfer finishes without an intermediate error path.
        self.set_initialized(0);
        let source = self.data_ptr();
        let destination_ptr = destination.data_ptr();
        for index in 0..len {
            // SAFETY: each source slot is read exactly once; destination slots
            // are uninitialized and the two allocator-issued blocks differ.
            unsafe {
                destination_ptr.add(index).write(source.add(index).read());
            }
        }
        destination.set_initialized(len as u32);
        Ok(())
    }

    /// Move an initialized prefix from inline MaybeUninit slots into this
    /// allocation.
    ///
    /// # Safety
    ///
    /// `source` must point to `len` initialized, properly aligned `T` values.
    /// The source slots become uninitialized and must not be dropped or read
    /// as `T` after this call.
    #[doc(hidden)]
    pub unsafe fn move_from_uninit_slice(
        &mut self,
        source: *mut core::mem::MaybeUninit<T>,
        len: usize,
    ) -> Result<()> {
        let start = self.len();
        let end = start.checked_add(len).ok_or(Error::OffsetOverflow)?;
        if end > self.capacity() {
            return Err(Error::OutOfBounds);
        }
        if len == 0 {
            return Ok(());
        }
        let destination = self.data_ptr();
        for index in 0..len {
            // SAFETY: the caller promises each source slot is initialized;
            // destination capacity and source alignment were validated.
            let value = unsafe { source.add(index).cast::<T>().read() };
            unsafe { destination.add(start + index).write(value) };
        }
        self.set_initialized(end as u32);
        Ok(())
    }

    pub(crate) fn belongs_to(&self, state: NonNull<ArenaState>) -> bool {
        self.state == state
    }

    pub(crate) const fn raw_offset(&self) -> u32 {
        self.offset
    }

    #[cfg(test)]
    pub(crate) fn debug_block_range(&self) -> (usize, usize) {
        let header = self.header();
        let header_offset = self.offset as usize - size_of::<AllocationHeader>();
        let start = header_offset - header.prefix as usize;
        (start, start + header.block_len as usize)
    }

    fn header(&self) -> AllocationHeader {
        let inner = unsafe { &*self.state.as_ref().inner.get() };
        read_allocation_header(inner.base, self.offset)
            .expect("live ArenaAllocation always has an allocation header")
    }

    fn set_initialized(&mut self, initialized: u32) {
        let inner = unsafe { &*self.state.as_ref().inner.get() };
        let mut header = read_allocation_header(inner.base, self.offset)
            .expect("live ArenaAllocation always has an allocation header");
        debug_assert!(initialized <= header.capacity);
        header.initialized = initialized;
        write_allocation_header(inner.base, self.offset, header);
    }

    fn data_ptr(&self) -> *mut T {
        let inner = unsafe { &*self.state.as_ref().inner.get() };
        // SAFETY: the token is produced by the allocator and retains the same
        // arena state for its entire branded lifetime.
        unsafe { inner.base.add(self.offset as usize).cast::<T>() }
    }
}

struct TruncateGuard<'arena, T: CompactValue> {
    allocation: *mut ArenaAllocation<'arena, T>,
    new_len: usize,
    armed: bool,
}

struct AllocationReleaseGuard {
    state: NonNull<ArenaState>,
    offset: u32,
    id: u32,
}

impl Drop for AllocationReleaseGuard {
    fn drop(&mut self) {
        // SAFETY: the owner token is dropped before its branded arena state;
        // this guard is created from that token's exact allocator-issued ID.
        unsafe { self.state.as_ref() }.release(self.offset, self.id);
    }
}

impl<T: CompactValue> Drop for TruncateGuard<'_, T> {
    fn drop(&mut self) {
        if self.armed {
            // SAFETY: this guard is created from the exclusive `&mut self` in
            // `truncate`; it runs only while that call is unwinding and
            // resumes dropping the still-initialized prefix.
            unsafe { (*self.allocation).truncate(self.new_len) };
        }
    }
}

impl ArenaAllocation<'_, u8> {
    pub(crate) fn uninit_capacity_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        // SAFETY: this owner describes a unique byte allocation and the full
        // capacity is valid writable storage, whether or not it is initialized.
        unsafe {
            slice::from_raw_parts_mut(self.data_ptr().cast::<MaybeUninit<u8>>(), self.capacity())
        }
    }
}

impl<T: CompactValue> Drop for ArenaAllocation<'_, T> {
    fn drop(&mut self) {
        let _release = AllocationReleaseGuard {
            state: self.state,
            offset: self.offset,
            id: self.id,
        };
        self.truncate(0);
    }
}

// SAFETY: moving the owner token transfers unique allocation ownership; its
// destructor continues to drop its initialized values and release the block.
unsafe impl<T: CompactValue> CompactValue for ArenaAllocation<'_, T> {}

pub(crate) fn allocate(
    state: NonNull<ArenaState>,
    bytes: usize,
    alignment: usize,
    capacity: u32,
    initialized: u32,
    owned: bool,
) -> Result<(u32, u32)> {
    // SAFETY: the state pointer was initialized in the backing and is alive
    // for the branded arena scope.
    unsafe { state.as_ref() }.allocate(bytes, alignment, capacity, initialized, owned)
}

pub(crate) fn try_resize<T: CompactValue>(
    state: NonNull<ArenaState>,
    allocation: &mut ArenaAllocation<'_, T>,
    capacity: usize,
) -> Result<bool> {
    if !allocation.belongs_to(state) {
        return Err(Error::ForeignArena);
    }
    let capacity = u32::try_from(capacity).map_err(|_| Error::OffsetOverflow)?;
    if capacity < allocation.header().initialized {
        return Err(Error::InitializationError);
    }
    let bytes = size_of::<T>()
        .checked_mul(capacity as usize)
        .ok_or(Error::OffsetOverflow)?;
    // SAFETY: the allocation token is live and uniquely borrowed; the state
    // belongs to the Arena supplied by the caller.
    unsafe { state.as_ref() }.try_resize(allocation.offset, bytes, capacity, align_of::<T>())
}

pub(crate) fn is_free(state: NonNull<ArenaState>, start: usize, end: usize) -> bool {
    // SAFETY: state remains initialized for the Arena borrow.
    unsafe { state.as_ref() }.is_free(start, end)
}

fn allocation_layout(
    base: *mut u8,
    start: usize,
    bytes: usize,
    alignment: usize,
) -> Result<Option<(u32, usize, usize)>> {
    let requested_alignment = alignment.max(BLOCK_ALIGNMENT);
    let minimum = start
        .checked_add(size_of::<AllocationHeader>())
        .ok_or(Error::OffsetOverflow)?;
    let address = (base as usize)
        .checked_add(minimum)
        .ok_or(Error::OffsetOverflow)?;
    let aligned = checked_align_up(address, requested_alignment)?;
    let data_offset = aligned
        .checked_sub(base as usize)
        .ok_or(Error::OffsetOverflow)?;
    if data_offset == 0 {
        return Err(Error::InvalidOffset);
    }
    let header_offset = data_offset
        .checked_sub(size_of::<AllocationHeader>())
        .ok_or(Error::OffsetOverflow)?;
    let prefix = header_offset
        .checked_sub(start)
        .ok_or(Error::OffsetOverflow)?;
    let used = prefix
        .checked_add(size_of::<AllocationHeader>())
        .and_then(|value| value.checked_add(bytes))
        .ok_or(Error::OffsetOverflow)?;
    let required = checked_align_up(used, BLOCK_ALIGNMENT)?;
    Ok(Some((
        u32::try_from(data_offset).map_err(|_| Error::OffsetOverflow)?,
        prefix,
        required,
    )))
}

fn aligned_block_len(prefix: usize, header_size: usize, bytes: usize) -> Result<usize> {
    let total = prefix
        .checked_add(header_size)
        .and_then(|value| value.checked_add(bytes))
        .ok_or(Error::OffsetOverflow)?;
    checked_align_up(total, BLOCK_ALIGNMENT)
}

fn state_layout(base: *mut u8, capacity: usize, reserved_prefix: usize) -> Result<(usize, usize)> {
    if capacity < crate::MIN_ARENA_BYTES {
        return Err(Error::InvalidCapacity);
    }
    if capacity as u64 > MAX_ARENA_BYTES {
        return Err(Error::BackingTooLarge);
    }
    let base_address = base as usize;
    let after_prefix = base_address
        .checked_add(reserved_prefix)
        .ok_or(Error::OffsetOverflow)?;
    let state_address = checked_align_up(after_prefix, align_of::<ArenaState>())?;
    let state_offset = state_address
        .checked_sub(base_address)
        .ok_or(Error::OffsetOverflow)?;
    let state_end = state_offset
        .checked_add(size_of::<ArenaState>())
        .ok_or(Error::OffsetOverflow)?;
    // Backings promise byte alignment only, so align the absolute address
    // instead of assuming the base pointer itself meets BLOCK_ALIGNMENT.
    let cursor_address = base_address
        .checked_add(state_end)
        .ok_or(Error::OffsetOverflow)?;
    let cursor = checked_align_up(cursor_address, BLOCK_ALIGNMENT)?
        .checked_sub(base_address)
        .ok_or(Error::OffsetOverflow)?;
    if cursor >= capacity || cursor as u64 > MAX_ARENA_BYTES {
        return Err(Error::InvalidCapacity);
    }
    Ok((state_offset, cursor))
}

fn validate_free_list(inner: &ArenaInner, first_free_offset: usize) -> Result<()> {
    let mut current = inner.free_head;
    let mut previous_end = 0_usize;
    let mut visited = 0_usize;
    while current != 0 {
        visited = visited.checked_add(1).ok_or(Error::InitializationError)?;
        let start = current as usize;
        let node_end = start
            .checked_add(size_of::<FreeNode>())
            .ok_or(Error::InitializationError)?;
        if start < first_free_offset
            || (inner.base as usize + start) % align_of::<FreeNode>() != 0
            || node_end > inner.cursor
            || visited > inner.capacity / size_of::<FreeNode>()
        {
            return Err(Error::InitializationError);
        }
        // SAFETY: the link is in bounds and aligned for FreeNode, and every
        // free-list node is initialized before it is linked into the list.
        let node = unsafe { read_free_node(inner.base, current) };
        let range_end = start
            .checked_add(node.len as usize)
            .ok_or(Error::InitializationError)?;
        if node.len < FREE_NODE_BYTES as u32
            || range_end > inner.cursor
            || (previous_end != 0 && start <= previous_end)
            || (node.next != 0 && node.next as usize <= start)
        {
            return Err(Error::InitializationError);
        }
        previous_end = range_end;
        current = node.next;
    }
    Ok(())
}

fn read_allocation_header(base: *mut u8, offset: u32) -> Result<AllocationHeader> {
    let header_offset = (offset as usize)
        .checked_sub(size_of::<AllocationHeader>())
        .ok_or(Error::InvalidOffset)?;
    // SAFETY: only allocator-issued tokens/offsets reach this private helper;
    // allocation headers are initialized before those tokens are published.
    Ok(unsafe { base.add(header_offset).cast::<AllocationHeader>().read() })
}

fn write_allocation_header(base: *mut u8, offset: u32, header: AllocationHeader) {
    let header_offset = offset as usize - size_of::<AllocationHeader>();
    // SAFETY: the allocator keeps this metadata header reserved immediately
    // before the payload and does not expose its bytes to collection callers.
    unsafe {
        base.add(header_offset)
            .cast::<AllocationHeader>()
            .write(header)
    };
}

unsafe fn read_free_node(base: *mut u8, offset: u32) -> FreeNode {
    // SAFETY: callers traverse only nodes linked from the allocator free list.
    unsafe { base.add(offset as usize).cast::<FreeNode>().read() }
}

unsafe fn write_free_node(base: *mut u8, offset: u32, len: u32, next: u32) {
    // SAFETY: callers reserve at least FREE_NODE_BYTES for each free node.
    unsafe {
        base.add(offset as usize)
            .cast::<FreeNode>()
            .write(FreeNode { len, next })
    };
}

unsafe fn write_free_next(base: *mut u8, offset: u32, next: u32) {
    // SAFETY: offset points to a live free-list node.
    unsafe { ptr::addr_of_mut!((*base.add(offset as usize).cast::<FreeNode>()).next).write(next) };
}

fn find_free_node(inner: &ArenaInner, target: u32) -> Option<(u32, u32, FreeNode)> {
    let mut previous = 0_u32;
    let mut current = inner.free_head;
    while current != 0 {
        let node = unsafe { read_free_node(inner.base, current) };
        if current == target {
            return Some((previous, current, node));
        }
        if current > target {
            return None;
        }
        previous = current;
        current = node.next;
    }
    None
}

fn replace_free_link(inner: &mut ArenaInner, previous: u32, replacement: u32) {
    if previous == 0 {
        inner.free_head = replacement;
    } else {
        unsafe { write_free_next(inner.base, previous, replacement) };
    }
}

fn insert_free(inner: &mut ArenaInner, mut start: usize, mut len: usize) -> Result<()> {
    let mut previous = 0_u32;
    let mut current = inner.free_head;
    while current != 0 && (current as usize) < start {
        previous = current;
        current = unsafe { read_free_node(inner.base, current) }.next;
    }

    if previous != 0 {
        let previous_node = unsafe { read_free_node(inner.base, previous) };
        let previous_end = previous as usize + previous_node.len as usize;
        if previous_end > start {
            return Err(Error::InitializationError);
        }
        if previous_end == start {
            start = previous as usize;
            len = len
                .checked_add(previous_node.len as usize)
                .ok_or(Error::OffsetOverflow)?;
            let start_u32 = u32::try_from(start).map_err(|_| Error::OffsetOverflow)?;
            unsafe { write_free_node(inner.base, start_u32, len as u32, previous_node.next) };
            // The previous node is replaced at the same address; its
            // successor remains the next unmerged range.
            current = previous_node.next;
        } else {
            let start_u32 = u32::try_from(start).map_err(|_| Error::OffsetOverflow)?;
            unsafe { write_free_node(inner.base, start_u32, len as u32, current) };
            unsafe { write_free_next(inner.base, previous, start_u32) };
            previous = start_u32;
        }
    } else {
        let start_u32 = u32::try_from(start).map_err(|_| Error::OffsetOverflow)?;
        unsafe { write_free_node(inner.base, start_u32, len as u32, current) };
        inner.free_head = start_u32;
        previous = start_u32;
    }

    if current != 0 {
        let current_node = unsafe { read_free_node(inner.base, current) };
        let end = previous as usize + unsafe { read_free_node(inner.base, previous) }.len as usize;
        if end > current as usize {
            return Err(Error::InitializationError);
        }
        if end == current as usize {
            let previous_node = unsafe { read_free_node(inner.base, previous) };
            let merged = previous_node
                .len
                .checked_add(current_node.len)
                .ok_or(Error::OffsetOverflow)?;
            unsafe { write_free_node(inner.base, previous, merged, current_node.next) };
        }
    }

    // Contract the bump tail and repeatedly absorb a preceding free range.
    loop {
        let mut before_tail = 0_u32;
        let mut node_offset = inner.free_head;
        let mut tail = 0_u32;
        while node_offset != 0 {
            let node = unsafe { read_free_node(inner.base, node_offset) };
            if node_offset as usize + node.len as usize == inner.cursor {
                tail = node_offset;
                break;
            }
            before_tail = node_offset;
            node_offset = node.next;
        }
        if tail == 0 {
            break;
        }
        let node = unsafe { read_free_node(inner.base, tail) };
        inner.cursor = tail as usize;
        replace_free_link(inner, before_tail, node.next);
    }
    Ok(())
}
