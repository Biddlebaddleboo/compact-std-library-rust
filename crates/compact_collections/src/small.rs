//! Inline-first compact vector for small, known-capacity collections.

use core::mem::MaybeUninit;
use core::ptr;
use core::slice;

use compact_core::{Arena, CompactValue};

use crate::{CollectionError, CompactVec, Result};

enum SmallStorage<'arena, T: CompactValue, const N: usize> {
    Inline {
        len: u32,
        values: [MaybeUninit<T>; N],
    },
    Heap(CompactVec<'arena, T>),
}

/// A vector that stores up to `N` values inline before allocating arena bytes.
///
/// Promotion moves values into the owned arena allocation; dropping the value
/// runs destructors for either the inline prefix or heap-backed vector.
pub struct CompactSmallVec<'arena, T: CompactValue, const N: usize> {
    storage: SmallStorage<'arena, T, N>,
}

impl<'arena, T: CompactValue, const N: usize> CompactSmallVec<'arena, T, N> {
    /// Construct an empty inline vector without allocating in `arena`.
    pub fn new_in(_arena: &Arena<'arena, '_>) -> Self {
        assert!(
            N <= u32::MAX as usize,
            "inline capacity exceeds compact length metadata"
        );
        Self {
            storage: SmallStorage::Inline {
                len: 0,
                values: [const { MaybeUninit::uninit() }; N],
            },
        }
    }

    /// Return the number of initialized values.
    pub fn len(&self) -> usize {
        match &self.storage {
            SmallStorage::Inline { len, .. } => *len as usize,
            SmallStorage::Heap(values) => values.len(),
        }
    }

    /// Return the current capacity.
    pub fn capacity(&self) -> usize {
        match &self.storage {
            SmallStorage::Inline { .. } => N,
            SmallStorage::Heap(values) => values.capacity(),
        }
    }

    /// Return whether this vector still uses its inline representation.
    pub fn is_inline(&self) -> bool {
        matches!(self.storage, SmallStorage::Inline { .. })
    }

    /// Return whether the vector contains no values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Ensure the vector can hold `additional` more values.
    pub fn reserve_in(&mut self, additional: usize, arena: &mut Arena<'arena, '_>) -> Result<()> {
        let required = self
            .len()
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        if let SmallStorage::Heap(values) = &mut self.storage {
            return values.reserve_in(additional, arena);
        }
        if required > N {
            self.promote(required, arena)?;
        }
        Ok(())
    }

    /// Append a value, promoting to arena storage when all inline slots are used.
    pub fn push_in(&mut self, value: T, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if let SmallStorage::Inline { len, values } = &mut self.storage {
            if (*len as usize) < N {
                values[*len as usize].write(value);
                *len += 1;
                return Ok(());
            }
        }
        self.promote(self.len().saturating_add(1), arena)?;
        let SmallStorage::Heap(values) = &mut self.storage else {
            unreachable!("promote always switches to the heap representation")
        };
        values.push_in(value, arena)
    }

    /// Borrow all initialized values as a contiguous native slice.
    pub fn as_slice<'view>(&'view self, arena: &'view Arena<'arena, '_>) -> Result<&'view [T]> {
        match &self.storage {
            SmallStorage::Inline { len, values } => {
                if *len == 0 {
                    return Ok(&[]);
                }
                // SAFETY: len is advanced only after writing each value, and
                // the wrapper borrow keeps the inline slots alive.
                Ok(unsafe { slice::from_raw_parts(values.as_ptr().cast::<T>(), *len as usize) })
            }
            SmallStorage::Heap(values) => values.as_slice(arena),
        }
    }

    /// Mutably borrow all initialized values.
    pub fn as_mut_slice<'view>(
        &'view mut self,
        arena: &'view mut Arena<'arena, '_>,
    ) -> Result<&'view mut [T]> {
        match &mut self.storage {
            SmallStorage::Inline { len, values } => {
                if *len == 0 {
                    return Ok(&mut []);
                }
                // SAFETY: len tracks the initialized prefix and exclusive
                // access to self prevents other inline views.
                Ok(unsafe {
                    slice::from_raw_parts_mut(values.as_mut_ptr().cast::<T>(), *len as usize)
                })
            }
            SmallStorage::Heap(values) => values.as_mut_slice(arena),
        }
    }

    /// Borrow an element by index.
    pub fn get<'view>(
        &'view self,
        index: usize,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<Option<&'view T>> {
        if index >= self.len() {
            return Ok(None);
        }
        match &self.storage {
            SmallStorage::Inline { values, .. } => {
                // SAFETY: index is below the initialized inline prefix.
                Ok(Some(unsafe { values[index].assume_init_ref() }))
            }
            SmallStorage::Heap(values) => values.get(index, arena),
        }
    }

    /// Mutably borrow an element by index.
    pub fn get_mut<'view>(
        &'view mut self,
        index: usize,
        arena: &'view mut Arena<'arena, '_>,
    ) -> Result<Option<&'view mut T>> {
        if index >= self.len() {
            return Ok(None);
        }
        match &mut self.storage {
            SmallStorage::Inline { values, .. } => {
                // SAFETY: index is initialized and self is exclusively borrowed.
                Ok(Some(unsafe { values[index].assume_init_mut() }))
            }
            SmallStorage::Heap(values) => values.get_mut(index, arena),
        }
    }

    /// Remove and return the last element.
    pub fn pop_in(&mut self, arena: &Arena<'arena, '_>) -> Result<Option<T>> {
        match &mut self.storage {
            SmallStorage::Inline { len, values } => {
                if *len == 0 {
                    return Ok(None);
                }
                *len -= 1;
                // SAFETY: the previous length proved this slot initialized;
                // lowering len transfers its unique ownership to the result.
                Ok(Some(unsafe { values[*len as usize].assume_init_read() }))
            }
            SmallStorage::Heap(values) => values.pop_in(arena),
        }
    }

    /// Drop initialized values after `len`.
    pub fn truncate(&mut self, len: usize) {
        match &mut self.storage {
            SmallStorage::Inline {
                len: current,
                values,
            } => {
                while (*current as usize) > len {
                    *current -= 1;
                    // SAFETY: the length is reduced before the destructor is
                    // called, so unwinding cannot drop this element twice.
                    unsafe { values[*current as usize].assume_init_drop() };
                }
            }
            SmallStorage::Heap(values) => values.truncate(len),
        }
    }

    /// Drop all values while retaining the current representation.
    pub fn clear(&mut self) {
        self.truncate(0);
    }

    fn promote(&mut self, required: usize, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if let SmallStorage::Heap(values) = &mut self.storage {
            return values.reserve_in(required.saturating_sub(values.len()), arena);
        }

        let inline_len = self.len();
        let grown = N.saturating_mul(2).max(4);
        let capacity = required.max(grown);
        let mut replacement = CompactVec::with_capacity_in(capacity, arena)?;
        if let SmallStorage::Inline { len, values } = &mut self.storage {
            if let Some(allocation) = replacement.allocation_mut() {
                // SAFETY: the inline length is the initialized prefix. The
                // destination was allocated with capacity >= inline_len.
                unsafe {
                    allocation.move_from_uninit_slice(values.as_mut_ptr(), inline_len)?;
                }
            }
            *len = 0;
        }
        self.storage = SmallStorage::Heap(replacement);
        Ok(())
    }
}

impl<T: CompactValue, const N: usize> Drop for CompactSmallVec<'_, T, N> {
    fn drop(&mut self) {
        if let SmallStorage::Inline { len, values } = &mut self.storage {
            while *len != 0 {
                *len -= 1;
                // SAFETY: the stored prefix is initialized and len is reduced
                // before each destructor invocation.
                unsafe { ptr::drop_in_place(values[*len as usize].as_mut_ptr()) };
            }
        }
    }
}

// SAFETY: all owned values move with the wrapper, and the explicit Drop impl
// destroys inline values while the heap variant owns a CompactVec token.
unsafe impl<T: CompactValue, const N: usize> CompactValue for CompactSmallVec<'_, T, N> {}
