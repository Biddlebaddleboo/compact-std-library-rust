//! Scoped, stable-address bump arena.

use core::marker::PhantomData;
use core::mem::{align_of, size_of, MaybeUninit};

use crate::native;
use crate::{Error, Offset32, OffsetSlice32, Result, StableBacking, MAX_ARENA_BYTES};

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

    let mut arena = Arena {
        region,
        cursor: 1,
        brand: PhantomData,
        not_send_sync: PhantomData,
    };
    Ok(action(&mut arena))
}

/// A scoped monotonic arena over one stable backing region.
///
/// Construct arenas with [`with_arena`]. The two lifetimes represent the
/// generative reference brand and the backing borrow; callers normally infer
/// them and do not name them directly.
///
/// V1 arenas are single-owner and neither `Send` nor `Sync`; packed word
/// mutation is non-atomic.
pub struct Arena<'arena, 'memory> {
    region: &'memory mut [MaybeUninit<u8>],
    cursor: usize,
    brand: PhantomData<fn(&'arena mut ()) -> &'arena mut ()>,
    // V1 arenas are single-owner and intentionally do not imply thread-safe
    // access, even when the underlying byte allocation itself is Send/Sync.
    not_send_sync: PhantomData<*mut ()>,
}

impl<'arena, 'memory> Arena<'arena, 'memory> {
    /// Return the backing capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.region.len()
    }

    /// Return the used prefix, including the reserved null byte and alignment
    /// padding.
    pub fn used_bytes(&self) -> usize {
        self.cursor
    }

    /// Return the number of bytes not yet consumed by allocations.
    pub fn remaining_bytes(&self) -> usize {
        self.capacity() - self.cursor
    }

    /// Reserve uninitialized storage for one `Copy` value.
    ///
    /// The returned type is `MaybeUninit<T>` so it cannot be resolved as an
    /// initialized `T` until [`write_uninit`](Self::write_uninit) succeeds.
    /// V1 accepts only `Copy` values, so discarding the arena never skips a
    /// destructor.
    pub fn alloc_uninit<T: Copy>(&mut self) -> Result<Offset32<'arena, MaybeUninit<T>>> {
        let raw = self.allocate(size_of::<T>(), align_of::<T>())?;
        Ok(Offset32::new(raw))
    }

    /// Store an initialized `Copy` value in the arena.
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
        let byte_ptr = self.region.as_mut_ptr().cast::<u8>();
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
            let byte_ptr = self.region.as_mut_ptr().cast::<u8>();
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
            let byte_ptr = self.region.as_mut_ptr().cast::<u8>();
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
        let byte_ptr = self.region.as_mut_ptr().cast::<u8>();
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
        let byte_ptr = self.region.as_mut_ptr().cast::<u8>();
        // SAFETY: checked_range established bounds, alignment, and allocation
        // size. The offset/length originate from one arena allocation and
        // `&mut self` gives exclusive access for the returned borrow.
        Ok(unsafe { native::slice_mut(byte_ptr.add(start).cast::<T>(), len) })
    }

    fn allocate(&mut self, byte_len: usize, alignment: usize) -> Result<u32> {
        if alignment == 0 || !alignment.is_power_of_two() {
            return Err(Error::AlignmentError);
        }
        let base = self.region.as_mut_ptr() as usize;
        let cursor_address = base.checked_add(self.cursor).ok_or(Error::OffsetOverflow)?;
        let aligned_address = crate::checked_align_up(cursor_address, alignment)?;
        let start = aligned_address
            .checked_sub(base)
            .ok_or(Error::OffsetOverflow)?;
        let raw = u32::try_from(start).map_err(|_| Error::OffsetOverflow)?;
        if raw == crate::NULL_OFFSET {
            return Err(Error::InvalidOffset);
        }
        let reserved = byte_len.max(1);
        let end = start.checked_add(reserved).ok_or(Error::OffsetOverflow)?;
        if end > self.region.len() {
            return Err(Error::AllocationExhausted);
        }
        if end as u64 > MAX_ARENA_BYTES {
            return Err(Error::AllocationExhausted);
        }
        self.cursor = end;
        Ok(raw)
    }

    fn checked_range<T>(&self, raw: u32, byte_len: usize) -> Result<usize> {
        if raw == crate::NULL_OFFSET {
            return Err(Error::InvalidOffset);
        }
        let start = raw as usize;
        let end = start
            .checked_add(byte_len.max(1))
            .ok_or(Error::OffsetOverflow)?;
        if end > self.cursor || end > self.region.len() {
            return Err(Error::OutOfBounds);
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

fn checked_slice_bytes<T>(len: usize) -> Result<usize> {
    let bytes = size_of::<T>()
        .checked_mul(len)
        .ok_or(Error::OffsetOverflow)?;
    if bytes > isize::MAX as usize {
        return Err(Error::OffsetOverflow);
    }
    Ok(bytes)
}
