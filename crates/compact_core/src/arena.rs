//! Scoped, stable-address arena with reusable owned allocations.

use core::marker::PhantomData;
use core::mem::{align_of, size_of, MaybeUninit};
use core::ptr::NonNull;

use crate::allocation::{self, ArenaState};
use crate::native;
use crate::{
    ArenaAllocation, CompactValue, Error, Offset32, OffsetSlice32, Result, StableBacking,
    MAX_ARENA_BYTES,
};

/// Run `action` with a fresh arena over `backing`.
///
/// The arena and its offsets are scoped to the callback. This generative scope
/// prevents safe code from using an offset with another arena, while ensuring
/// native borrows cannot outlive the backing. Arena bytes are not cleared when
/// the scope ends.
pub fn with_arena<'memory, B, R, F>(backing: &'memory mut B, action: F) -> Result<R>
where
    B: StableBacking + ?Sized,
    F: for<'arena> FnOnce(&mut Arena<'arena, 'memory>) -> R,
{
    let region = backing.bytes_mut();
    if region.len() < crate::MIN_ARENA_BYTES {
        return Err(Error::InvalidCapacity);
    }
    if region.len() as u64 > MAX_ARENA_BYTES {
        return Err(Error::BackingTooLarge);
    }

    let region_pointer = NonNull::new(region.as_mut_ptr()).expect("validated backing is nonempty");
    let capacity = region.len();
    // SAFETY: StableBacking guarantees that this writable slice remains at a
    // fixed address for the callback's backing borrow.
    let (state, _) = unsafe { ArenaState::place(region_pointer.as_ptr(), capacity) }?;
    let mut arena = Arena {
        region: region_pointer,
        capacity,
        state,
        drop_state: true,
        backing: PhantomData,
        brand: PhantomData,
        not_send_sync: PhantomData,
    };
    Ok(action(&mut arena))
}

/// Initialize an arena in `backing` and preserve its allocator state after
/// `action` returns so a trusted owner can attach again later.
///
/// The callback's higher-ranked arena lifetime prevents branded handles and
/// references from escaping. Persistent values must use non-owning compact
/// descriptors whose destructor behavior is handled by their owner.
pub fn with_arena_persistent<'memory, B, R, F>(backing: &'memory mut B, action: F) -> Result<R>
where
    B: StableBacking + ?Sized,
    F: for<'arena> FnOnce(&mut Arena<'arena, 'memory>) -> R,
{
    let region = backing.bytes_mut();
    if region.len() < crate::MIN_ARENA_BYTES {
        return Err(Error::InvalidCapacity);
    }
    if region.len() as u64 > MAX_ARENA_BYTES {
        return Err(Error::BackingTooLarge);
    }

    let region_pointer = NonNull::new(region.as_mut_ptr()).expect("validated backing is nonempty");
    let capacity = region.len();
    // SAFETY: StableBacking guarantees that this writable slice remains at a
    // fixed address for the callback's backing borrow.
    let (state, _) = unsafe { ArenaState::place_persistent(region_pointer.as_ptr(), capacity) }?;
    let mut arena = Arena {
        region: region_pointer,
        capacity,
        state,
        drop_state: false,
        backing: PhantomData,
        brand: PhantomData,
        not_send_sync: PhantomData,
    };
    Ok(action(&mut arena))
}

/// Attach to allocator state previously initialized by
/// [`with_arena_persistent`].
///
/// # Safety
///
/// `backing` must be the exact stable allocation previously passed to
/// `with_arena_persistent`. Its allocation must not have moved or been
/// overwritten, and no other arena may currently be attached to it. Arena
/// accesses may update its allocator state between attachments. The header
/// and allocator links are validated before access, but this function cannot
/// prove that an arbitrary initialized byte region originated from this
/// allocator. Invalid or ABI-incompatible state returns
/// [`Error::InitializationError`].
pub unsafe fn with_arena_attached<'memory, B, R, F>(backing: &'memory mut B, action: F) -> Result<R>
where
    B: StableBacking + ?Sized,
    F: for<'arena> FnOnce(&mut Arena<'arena, 'memory>) -> R,
{
    let region = backing.bytes_mut();
    if region.len() < crate::MIN_ARENA_BYTES {
        return Err(Error::InvalidCapacity);
    }
    if region.len() as u64 > MAX_ARENA_BYTES {
        return Err(Error::BackingTooLarge);
    }

    // SAFETY: the caller guarantees the bytes contain an initialized state
    // from this exact stable backing; attach checks its header and links.
    let region_pointer = NonNull::new(region.as_mut_ptr()).expect("validated backing is nonempty");
    let capacity = region.len();
    // SAFETY: inherited from this function's contract: the caller provides
    // the original initialized backing and no concurrent attachment.
    let (state, _) = unsafe { ArenaState::attach_persistent(region_pointer.as_ptr(), capacity) }?;
    let mut arena = Arena {
        region: region_pointer,
        capacity,
        state,
        drop_state: false,
        backing: PhantomData,
        brand: PhantomData,
        not_send_sync: PhantomData,
    };
    Ok(action(&mut arena))
}

/// A scoped arena over one stable backing region.
///
/// Construct arenas with [`with_arena`]. The two lifetimes represent the
/// generative reference brand and the backing borrow; callers normally infer
/// them and do not name them directly.
///
/// Mutable arenas are single-owner and neither `Send` nor `Sync`; packed word
/// mutation is non-atomic.
pub struct Arena<'arena, 'memory> {
    // Keep the backing borrow in `backing`, but do not retain a wide mutable
    // reference that aliases allocator state stored inside this region.
    region: NonNull<MaybeUninit<u8>>,
    capacity: usize,
    state: NonNull<ArenaState>,
    drop_state: bool,
    backing: PhantomData<&'memory mut [MaybeUninit<u8>]>,
    brand: PhantomData<fn(&'arena mut ()) -> &'arena mut ()>,
    // Arenas are single-owner and intentionally do not imply thread-safe
    // access, even when the underlying byte allocation itself is Send/Sync.
    not_send_sync: PhantomData<*mut ()>,
}

impl Drop for Arena<'_, '_> {
    fn drop(&mut self) {
        if self.drop_state {
            // SAFETY: `with_arena`'s generative callback prevents allocations
            // from escaping; all owners are dropped before this state ends.
            unsafe { core::ptr::drop_in_place(self.state.as_ptr()) };
        }
    }
}

impl<'arena, 'memory> Arena<'arena, 'memory> {
    /// Return the backing capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Return the allocator high-water prefix, including state and alignment
    /// metadata. Releasing a tail allocation can reduce this value.
    pub fn used_bytes(&self) -> usize {
        // SAFETY: the state object remains in the backing for the arena scope.
        unsafe { self.state.as_ref() }.used_bytes()
    }

    /// Return total tail space plus reusable free ranges.
    pub fn remaining_bytes(&self) -> usize {
        // SAFETY: the state object remains in the backing for the arena scope.
        unsafe { self.state.as_ref() }.remaining_bytes()
    }

    #[cfg(test)]
    pub(crate) fn debug_validate_allocator(&self, live_ranges: &[(usize, usize)]) -> Result<()> {
        // SAFETY: the arena scope retains its initialized allocator state.
        unsafe { self.state.as_ref() }.debug_validate_allocator(live_ranges)
    }

    /// Run a nested arena in a parent-owned allocation and release all of its
    /// storage when the callback returns.
    ///
    /// Scratch capacity includes the nested arena's allocator metadata. Values
    /// with destructors are supported through ordinary arena owners, which are
    /// dropped before the scratch allocation is released. Nested scratch calls
    /// are supported. The callback's higher-ranked lifetimes prevent scratch
    /// references and owner handles from escaping.
    pub fn scratch<R, F>(&mut self, capacity: usize, action: F) -> Result<R>
    where
        F: for<'scratch, 'scratch_memory> FnOnce(&mut Arena<'scratch, 'scratch_memory>) -> R,
    {
        if capacity < crate::MIN_ARENA_BYTES {
            return Err(Error::InvalidCapacity);
        }
        if capacity as u64 > MAX_ARENA_BYTES {
            return Err(Error::BackingTooLarge);
        }

        let mut allocation = self.alloc_owned_slice::<u8>(capacity)?;
        let result = {
            let mut backing = ScratchBacking {
                bytes: allocation.uninit_capacity_mut(),
            };
            with_arena(&mut backing, action)
        };
        drop(allocation);
        result
    }

    /// Allocate uninitialized storage owned by one move-only compact value.
    ///
    /// The returned owner tracks its initialized prefix, runs element
    /// destructors when dropped, and returns its block to the arena free list.
    #[doc(hidden)]
    pub fn alloc_owned_slice<T: CompactValue>(
        &mut self,
        capacity: usize,
    ) -> Result<ArenaAllocation<'arena, T>> {
        let compact_capacity = u32::try_from(capacity).map_err(|_| Error::OffsetOverflow)?;
        let bytes = checked_slice_bytes::<T>(capacity)?;
        let (offset, id) = allocation::allocate(
            self.state,
            bytes,
            align_of::<T>(),
            compact_capacity,
            0,
            true,
        )?;
        Ok(ArenaAllocation::new(self.state, offset, id))
    }

    /// Allocate and initialize one arena-owned value.
    #[doc(hidden)]
    pub fn alloc_owned_value<T: CompactValue>(
        &mut self,
        value: T,
    ) -> Result<ArenaAllocation<'arena, T>> {
        let mut allocation = self.alloc_owned_slice::<T>(1)?;
        allocation.push(value)?;
        Ok(allocation)
    }

    /// Check that an owning token was created by this arena.
    #[doc(hidden)]
    pub fn validate_owned<T: CompactValue>(
        &self,
        allocation: &ArenaAllocation<'arena, T>,
    ) -> Result<()> {
        if !allocation.belongs_to(self.state) {
            return Err(Error::ForeignArena);
        }
        let byte_len = checked_slice_bytes::<T>(allocation.capacity())?;
        self.checked_range::<T>(allocation.raw_offset(), byte_len)?;
        Ok(())
    }

    /// Grow or shrink an owning allocation in place when adjacent storage
    /// permits it. Returns `false` without changing the token when relocation
    /// is required.
    #[doc(hidden)]
    pub fn try_resize_owned<T: CompactValue>(
        &mut self,
        allocation: &mut ArenaAllocation<'arena, T>,
        capacity: usize,
    ) -> Result<bool> {
        self.validate_owned(allocation)?;
        allocation::try_resize(self.state, allocation, capacity)
    }

    /// Inspect the used arena prefix without claiming that any byte is
    /// initialized. Alignment padding, allocator metadata, and uninitialized
    /// allocations remain represented as [`MaybeUninit<u8>`](MaybeUninit).
    ///
    /// # Safety
    ///
    /// While the returned slice is live, no operation may allocate, resize,
    /// or release an arena allocation. These operations can update allocator
    /// metadata inside the returned prefix.
    pub unsafe fn used_uninit_bytes(&self) -> &[MaybeUninit<u8>] {
        // SAFETY: the prefix is within `region`; MaybeUninit permits every
        // initialization state. The caller promises not to mutate the prefix
        // through allocator operations while this shared view is live.
        unsafe { core::slice::from_raw_parts(self.region.as_ptr(), self.used_bytes()) }
    }

    /// Allocate and initialize an exact byte range from `bytes`.
    ///
    /// Empty input returns the canonical null range and consumes no storage.
    pub fn alloc_bytes(&mut self, bytes: &[u8]) -> Result<crate::ByteRange32<'arena>> {
        let len = u32::try_from(bytes.len()).map_err(|_| Error::OffsetOverflow)?;
        if bytes.is_empty() {
            return Ok(crate::ByteRange32::empty());
        }
        let raw = self.allocate(bytes.len(), 1)?;
        let destination = self.region.as_ptr().cast::<u8>();
        // SAFETY: allocate reserved this exact range, and the immutable source
        // cannot overlap the arena's exclusive backing borrow.
        unsafe {
            destination
                .add(raw as usize)
                .copy_from_nonoverlapping(bytes.as_ptr(), bytes.len());
        }
        Ok(crate::ByteRange32::new(raw, len))
    }

    /// Allocate an initialized zero-filled byte range.
    ///
    /// Zero is a valid `u8` value, so the returned range is safe to read and
    /// mutate through [`get_bytes`](Self::get_bytes) and
    /// [`get_bytes_mut`](Self::get_bytes_mut).
    pub fn alloc_zeroed_bytes(&mut self, len: usize) -> Result<crate::ByteRange32<'arena>> {
        let compact_len = u32::try_from(len).map_err(|_| Error::OffsetOverflow)?;
        if len == 0 {
            return Ok(crate::ByteRange32::empty());
        }
        let raw = self.allocate(len, 1)?;
        let destination = self.region.as_ptr().cast::<u8>();
        // SAFETY: allocate reserved `len` bytes and u8 accepts the all-zero
        // bit pattern. The exclusive arena borrow guarantees unique access.
        unsafe {
            destination.add(raw as usize).write_bytes(0, len);
        }
        Ok(crate::ByteRange32::new(raw, compact_len))
    }

    /// Borrow the exact initialized bytes in a byte allocation.
    pub fn get_bytes<'view>(&'view self, range: crate::ByteRange32<'arena>) -> Result<&'view [u8]> {
        if range.is_empty() {
            return Ok(&[]);
        }
        let start = self.checked_range::<u8>(range.offset, range.len())?;
        let byte_ptr = self.region.as_ptr().cast::<u8>();
        // SAFETY: the branded descriptor came from an initialized byte
        // allocation (or an unsafe constructor whose caller guarantees it),
        // and checked_range validates its full extent.
        Ok(unsafe { native::slice(byte_ptr.add(start), range.len()) })
    }

    /// Mutably borrow the exact initialized bytes in a byte allocation.
    pub fn get_bytes_mut<'view>(
        &'view mut self,
        range: crate::ByteRange32<'arena>,
    ) -> Result<&'view mut [u8]> {
        if range.is_empty() {
            return Ok(&mut []);
        }
        let start = self.checked_range::<u8>(range.offset, range.len())?;
        let byte_ptr = self.region.as_ptr().cast::<u8>();
        // SAFETY: the descriptor validates the complete initialized byte
        // range and the exclusive arena borrow excludes all other references.
        Ok(unsafe { native::slice_mut(byte_ptr.add(start), range.len()) })
    }

    /// Mutably inspect an exact initialized allocation as raw bytes.
    ///
    /// # Safety
    ///
    /// `offset` and `byte_len` must describe one exact allocation whose every
    /// byte is initialized, with no padding bytes that may be uninitialized.
    /// The caller must ensure every byte pattern written through the returned
    /// slice preserves the validity of the underlying value, and must ensure
    /// no other reference to that value is live while the slice exists.
    pub unsafe fn get_bytes_mut_unchecked<'view, T>(
        &'view mut self,
        offset: Offset32<'arena, T>,
        byte_len: usize,
    ) -> Result<&'view mut [u8]> {
        let start = self.checked_range::<T>(offset.raw, byte_len)?;
        let byte_ptr = self.region.as_ptr().cast::<u8>();
        // SAFETY: the caller guarantees initialized bytes and preserves type
        // validity; checked_range proves arena extent/alignment and &mut self
        // guarantees exclusive access.
        Ok(unsafe { native::slice_mut(byte_ptr.add(start), byte_len) })
    }

    /// Copy initialized bytes between two ranges in this arena.
    pub fn copy_bytes(
        &mut self,
        source: crate::ByteRange32<'arena>,
        destination: crate::ByteRange32<'arena>,
        len: usize,
    ) -> Result<()> {
        if len > source.len() || len > destination.len() {
            return Err(Error::OutOfBounds);
        }
        if len == 0 {
            return Ok(());
        }
        let source_start = self.checked_range::<u8>(source.offset, source.len())?;
        let destination_start = self.checked_range::<u8>(destination.offset, destination.len())?;
        let byte_ptr = self.region.as_ptr().cast::<u8>();
        // SAFETY: both ranges are checked initialized byte allocations. `copy`
        // permits overlap, and `&mut self` guarantees exclusive arena access.
        unsafe {
            core::ptr::copy(
                byte_ptr.add(source_start),
                byte_ptr.add(destination_start),
                len,
            );
        }
        Ok(())
    }

    /// Read a sequence whose initialized prefix is tracked by a higher-level
    /// container invariant.
    ///
    /// # Safety
    ///
    /// The first `initialized_len` elements of `allocation` must have been
    /// initialized as valid `T` values and must remain initialized for this
    /// borrow. No other reference may mutate them during the returned borrow.
    pub unsafe fn get_slice_assume_init<'view, T: Copy + 'view>(
        &'view self,
        allocation: OffsetSlice32<'arena, MaybeUninit<T>>,
        initialized_len: usize,
    ) -> Result<&'view [T]> {
        if initialized_len > allocation.len() {
            return Err(Error::OutOfBounds);
        }
        let byte_len = checked_slice_bytes::<T>(initialized_len)?;
        let start = self.checked_range::<MaybeUninit<T>>(allocation.offset.raw, byte_len)?;
        let byte_ptr = self.region.as_ptr().cast::<u8>();
        // SAFETY: the caller proves initialization; this method proves extent
        // and alignment and bounds the reference to `self`.
        Ok(unsafe { native::slice(byte_ptr.add(start).cast::<T>(), initialized_len) })
    }

    /// Mutably borrow a sequence whose initialized prefix is tracked by a
    /// higher-level container invariant.
    ///
    /// # Safety
    ///
    /// The first `initialized_len` elements must be valid initialized `T`
    /// values. The caller must ensure no other references or handles expose
    /// those elements while the returned exclusive slice is alive.
    pub unsafe fn get_slice_mut_assume_init<'view, T: Copy + 'view>(
        &'view mut self,
        allocation: OffsetSlice32<'arena, MaybeUninit<T>>,
        initialized_len: usize,
    ) -> Result<&'view mut [T]> {
        if initialized_len > allocation.len() {
            return Err(Error::OutOfBounds);
        }
        let byte_len = checked_slice_bytes::<T>(initialized_len)?;
        let start = self.checked_range::<MaybeUninit<T>>(allocation.offset.raw, byte_len)?;
        let byte_ptr = self.region.as_ptr().cast::<u8>();
        // SAFETY: the caller proves initialization and unique access; this
        // method proves extent/alignment and ties the result to `&mut self`.
        Ok(unsafe { native::slice_mut(byte_ptr.add(start).cast::<T>(), initialized_len) })
    }

    /// Copy an initialized prefix into a second reserved slice without
    /// creating overlapping native borrows of the arena.
    ///
    /// # Safety
    ///
    /// The first `initialized_len` source elements must be valid initialized
    /// `T` values. The destination must not be observed as initialized until
    /// after this call succeeds.
    pub unsafe fn copy_slice_assume_init<T: Copy>(
        &mut self,
        source: OffsetSlice32<'arena, MaybeUninit<T>>,
        initialized_len: usize,
        destination: OffsetSlice32<'arena, MaybeUninit<T>>,
    ) -> Result<()> {
        if initialized_len > source.len() || initialized_len > destination.len() {
            return Err(Error::OutOfBounds);
        }
        if initialized_len == 0 {
            return Ok(());
        }
        let byte_len = checked_slice_bytes::<T>(initialized_len)?;
        let source_start = self.checked_range::<MaybeUninit<T>>(source.offset.raw, byte_len)?;
        let destination_len = checked_slice_bytes::<T>(destination.len())?;
        let destination_start =
            self.checked_range::<MaybeUninit<T>>(destination.offset.raw, destination_len)?;
        if initialized_len != 0 {
            let byte_ptr = self.region.as_ptr().cast::<u8>();
            // SAFETY: both ranges are checked and the source initialization is
            // guaranteed by the caller. `copy` supports overlap; Copy values
            // require no ownership transfer or destructor bookkeeping.
            unsafe {
                core::ptr::copy(
                    byte_ptr.add(source_start).cast::<T>(),
                    byte_ptr
                        .add(destination_start)
                        .cast::<MaybeUninit<T>>()
                        .cast::<T>(),
                    initialized_len,
                );
            }
        }
        Ok(())
    }

    /// Initialize one element of a reserved uninitialized slice.
    pub fn write_uninit_at<T: Copy>(
        &mut self,
        allocation: OffsetSlice32<'arena, MaybeUninit<T>>,
        index: usize,
        value: T,
    ) -> Result<()> {
        if index >= allocation.len() {
            return Err(Error::OutOfBounds);
        }
        let byte_len = checked_slice_bytes::<T>(allocation.len())?;
        let start = self.checked_range::<MaybeUninit<T>>(allocation.offset.raw, byte_len)?;
        let byte_ptr = self.region.as_ptr().cast::<u8>();
        // SAFETY: the checked allocation includes `index`; MaybeUninit<T> has
        // the same size/alignment as T and writing it initializes that slot.
        unsafe {
            byte_ptr
                .add(start)
                .cast::<MaybeUninit<T>>()
                .add(index)
                .write(MaybeUninit::new(value));
        }
        Ok(())
    }

    /// Initialize a prefix of a reserved uninitialized slice by copying values.
    pub fn write_uninit_prefix<T: Copy>(
        &mut self,
        allocation: OffsetSlice32<'arena, MaybeUninit<T>>,
        values: &[T],
    ) -> Result<OffsetSlice32<'arena, T>> {
        if values.len() > allocation.len() {
            return Err(Error::OutOfBounds);
        }
        let byte_len = checked_slice_bytes::<T>(allocation.len())?;
        let start = self.checked_range::<MaybeUninit<T>>(allocation.offset.raw, byte_len)?;
        if !values.is_empty() {
            let byte_ptr = self.region.as_ptr().cast::<u8>();
            // SAFETY: the destination has room for the source prefix, the
            // source is initialized, and arena storage is exclusively borrowed.
            unsafe {
                byte_ptr
                    .add(start)
                    .cast::<MaybeUninit<T>>()
                    .cast::<T>()
                    .copy_from_nonoverlapping(values.as_ptr(), values.len());
            }
        }
        Ok(OffsetSlice32::new(
            Offset32::new(allocation.offset.raw),
            u32::try_from(values.len()).map_err(|_| Error::OffsetOverflow)?,
        ))
    }

    /// Reserve uninitialized storage for one `Copy` value accessed through a
    /// non-owning offset.
    ///
    /// The returned type is `MaybeUninit<T>` so it cannot be resolved as an
    /// initialized `T` until [`write_uninit`](Self::write_uninit) succeeds.
    /// Use [`alloc_owned_slice`](Self::alloc_owned_slice) for values with
    /// destructor obligations.
    pub fn alloc_uninit<T: Copy>(&mut self) -> Result<Offset32<'arena, MaybeUninit<T>>> {
        let raw = self.allocate(size_of::<T>(), align_of::<T>())?;
        Ok(Offset32::new(raw))
    }

    /// Store an initialized `Copy` value and return a non-owning offset.
    ///
    /// This API does not create a destructor owner. Values with destructor
    /// obligations must use [`alloc_owned_value`](Self::alloc_owned_value).
    pub fn alloc_value<T: Copy>(&mut self, value: T) -> Result<Offset32<'arena, T>> {
        let slot = self.alloc_uninit::<T>()?;
        self.write_uninit(slot, value)
    }

    /// Initialize a slot returned by [`alloc_uninit`](Self::alloc_uninit).
    pub fn write_uninit<T: Copy>(
        &mut self,
        offset: Offset32<'arena, MaybeUninit<T>>,
        value: T,
    ) -> Result<Offset32<'arena, T>> {
        let start = self.checked_range::<MaybeUninit<T>>(offset.raw, size_of::<T>())?;
        let byte_ptr = self.region.as_ptr().cast::<u8>();
        // SAFETY: checked_range proved the complete reserved range is inside
        // the backing and correctly aligned. `&mut self` gives exclusive
        // access, `T: Copy` has no destructor, and the backing remains stable
        // for the arena borrow. Writing the value initializes this allocation.
        unsafe {
            byte_ptr.add(start).cast::<T>().write(value);
        }
        Ok(Offset32::new(offset.raw))
    }

    /// Allocate and initialize a contiguous slice, returning its element count
    /// together with the four-byte first-element offset.
    pub fn alloc_slice<T: Copy>(&mut self, values: &[T]) -> Result<OffsetSlice32<'arena, T>> {
        let len = u32::try_from(values.len()).map_err(|_| Error::OffsetOverflow)?;
        let byte_len = checked_slice_bytes::<T>(values.len())?;
        let raw = self.allocate(byte_len, align_of::<T>())?;
        if !values.is_empty() {
            let byte_ptr = self.region.as_ptr().cast::<u8>();
            // SAFETY: allocate reserved byte_len bytes at a T-aligned address;
            // the source is a valid initialized slice, the destination is
            // exclusively borrowed, and Copy values need no drop tracking.
            unsafe {
                byte_ptr
                    .add(raw as usize)
                    .cast::<T>()
                    .copy_from_nonoverlapping(values.as_ptr(), values.len());
            }
        }
        Ok(OffsetSlice32::new(Offset32::new(raw), len))
    }

    /// Reserve uninitialized storage for a contiguous slice of `len` values.
    pub fn alloc_uninit_slice<T: Copy>(
        &mut self,
        len: usize,
    ) -> Result<OffsetSlice32<'arena, MaybeUninit<T>>> {
        let compact_len = u32::try_from(len).map_err(|_| Error::OffsetOverflow)?;
        let byte_len = checked_slice_bytes::<T>(len)?;
        let raw = self.allocate(byte_len, align_of::<T>())?;
        Ok(OffsetSlice32::new(Offset32::new(raw), compact_len))
    }

    /// Initialize every element of a slice returned by
    /// [`alloc_uninit_slice`](Self::alloc_uninit_slice).
    pub fn write_uninit_slice<T: Copy>(
        &mut self,
        offset: OffsetSlice32<'arena, MaybeUninit<T>>,
        values: &[T],
    ) -> Result<OffsetSlice32<'arena, T>> {
        if values.len() != offset.len() {
            return Err(Error::InitializationError);
        }
        let byte_len = checked_slice_bytes::<T>(values.len())?;
        let start = self.checked_range::<MaybeUninit<T>>(offset.offset.raw, byte_len)?;
        if !values.is_empty() {
            let byte_ptr = self.region.as_ptr().cast::<u8>();
            // SAFETY: checked_range proved the destination covers the complete
            // slice and is aligned. `&mut self` guarantees exclusivity; the
            // source has `values.len()` initialized Copy elements; and the
            // backing cannot move while this arena borrow is active.
            unsafe {
                byte_ptr
                    .add(start)
                    .cast::<T>()
                    .copy_from_nonoverlapping(values.as_ptr(), values.len());
            }
        }
        Ok(OffsetSlice32::new(
            Offset32::new(offset.offset.raw),
            offset.len,
        ))
    }

    /// Resolve an initialized value as an immutable native borrow.
    pub fn get<'view, T: Copy + 'view>(
        &'view self,
        offset: Offset32<'arena, T>,
    ) -> Result<&'view T> {
        let start = self.checked_range::<T>(offset.raw, size_of::<T>())?;
        let byte_ptr = self.region.as_ptr().cast::<u8>();
        // SAFETY: checked_range established bounds and alignment. The offset's
        // invariant ties it to this arena and to an initialized T allocation;
        // the returned reference is bounded by the shared borrow of `self`.
        Ok(unsafe { native::reference(byte_ptr.add(start).cast::<T>()) })
    }

    /// Resolve an initialized value as an exclusive native borrow.
    pub fn get_mut<'view, T: Copy + 'view>(
        &'view mut self,
        offset: Offset32<'arena, T>,
    ) -> Result<&'view mut T> {
        let start = self.checked_range::<T>(offset.raw, size_of::<T>())?;
        let byte_ptr = self.region.as_ptr().cast::<u8>();
        // SAFETY: checked_range established bounds and alignment. The offset
        // identifies one allocation from this arena; `&mut self` excludes all
        // other arena borrows, and the reference cannot outlive that borrow.
        Ok(unsafe { native::reference_mut(byte_ptr.add(start).cast::<T>()) })
    }

    /// Resolve a contiguous initialized allocation as a zero-copy slice.
    pub fn get_slice<'view, T: Copy + 'view>(
        &'view self,
        allocation: OffsetSlice32<'arena, T>,
    ) -> Result<&'view [T]> {
        let len = allocation.len();
        let byte_len = checked_slice_bytes::<T>(len)?;
        let start = self.checked_range::<T>(allocation.offset.raw, byte_len)?;
        let byte_ptr = self.region.as_ptr().cast::<u8>();
        // SAFETY: checked_range established the complete allocation is in
        // bounds and T-aligned. The generative offset brand and private length
        // tie the initialized elements to this arena; the borrow is limited to
        // `self` and the backing remains stable.
        Ok(unsafe { native::slice(byte_ptr.add(start).cast::<T>(), len) })
    }

    /// Resolve a contiguous initialized allocation as an exclusive zero-copy
    /// slice.
    pub fn get_slice_mut<'view, T: Copy + 'view>(
        &'view mut self,
        allocation: OffsetSlice32<'arena, T>,
    ) -> Result<&'view mut [T]> {
        let len = allocation.len();
        let byte_len = checked_slice_bytes::<T>(len)?;
        let start = self.checked_range::<T>(allocation.offset.raw, byte_len)?;
        let byte_ptr = self.region.as_ptr().cast::<u8>();
        // SAFETY: checked_range established bounds, alignment, and allocation
        // size. The offset/length originate from one arena allocation and
        // `&mut self` gives exclusive access for the returned borrow.
        Ok(unsafe { native::slice_mut(byte_ptr.add(start).cast::<T>(), len) })
    }

    fn allocate(&mut self, byte_len: usize, alignment: usize) -> Result<u32> {
        let capacity = u32::try_from(byte_len).map_err(|_| Error::OffsetOverflow)?;
        allocation::allocate(self.state, byte_len, alignment, capacity, capacity, false)
            .map(|(offset, _)| offset)
    }

    fn checked_range<T>(&self, raw: u32, byte_len: usize) -> Result<usize> {
        if raw == crate::NULL_OFFSET {
            return Err(Error::InvalidOffset);
        }
        let start = raw as usize;
        let end = start
            .checked_add(byte_len.max(1))
            .ok_or(Error::OffsetOverflow)?;
        if end > self.used_bytes() || end > self.capacity {
            return Err(Error::OutOfBounds);
        }
        if allocation::is_free(self.state, start, end) {
            return Err(Error::InvalidOffset);
        }
        let address = (self.region.as_ptr() as usize)
            .checked_add(start)
            .ok_or(Error::OffsetOverflow)?;
        if address % align_of::<T>() != 0 {
            return Err(Error::AlignmentError);
        }
        Ok(start)
    }
}

struct ScratchBacking<'memory> {
    bytes: &'memory mut [MaybeUninit<u8>],
}

// SAFETY: the byte slice is borrowed from a unique parent-owned allocation;
// its address stays fixed while the nested arena holds the mutable borrow.
unsafe impl StableBacking for ScratchBacking<'_> {
    fn bytes_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        self.bytes
    }
}

fn checked_slice_bytes<T>(len: usize) -> Result<usize> {
    let bytes = size_of::<T>()
        .checked_mul(len)
        .ok_or(Error::OffsetOverflow)?;
    if bytes > isize::MAX as usize {
        return Err(Error::OffsetOverflow);
    }
    Ok(bytes)
}
