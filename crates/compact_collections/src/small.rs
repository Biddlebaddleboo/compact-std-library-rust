//! Inline-first compact vector for small, known-capacity collections.

use core::mem::MaybeUninit;
use core::slice;

use compact_core::Arena;

use crate::{CollectionError, CompactVec, Result};

enum SmallStorage<'arena, T: Copy, const N: usize> {
    Inline {
        len: u32,
        values: [MaybeUninit<T>; N],
    },
    Heap(CompactVec<'arena, T>),
}

/// A vector that stores up to `N` values inline before allocating arena bytes.
///
/// Inline values live in this wrapper. After the inline capacity is reached,
/// the values are copied into a compact arena vector. `T: Copy` keeps moves
/// and arena teardown free of native drop obligations.
pub struct CompactSmallVec<'arena, T: Copy, const N: usize> {
    storage: SmallStorage<'arena, T, N>,
}

impl<'arena, T: Copy, const N: usize> CompactSmallVec<'arena, T, N> {
    /// Construct an empty inline vector without allocating in `arena`.
    pub fn new_in(_arena: &Arena<'arena, '_>) -> Self {
        assert!(
            N <= u32::MAX as usize,
            "inline capacity exceeds compact length metadata"
        );
        Self {
            storage: SmallStorage::Inline {
                len: 0,
                values: [MaybeUninit::uninit(); N],
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
                // SAFETY: only push_in increments len, and it initializes the
                // corresponding MaybeUninit slot before publishing the length.
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
                // SAFETY: the private length tracks initialized elements and
                // the exclusive wrapper borrow prevents any other inline view.
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
                // SAFETY: index was checked against the initialized prefix.
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
                // SAFETY: index is initialized and the wrapper is exclusively
                // borrowed for the returned mutable reference.
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
                // T: Copy permits reading without changing drop obligations.
                Ok(Some(unsafe { values[*len as usize].assume_init_read() }))
            }
            SmallStorage::Heap(values) => values.pop_in(arena),
        }
    }

    /// Reduce the logical length without reclaiming arena storage.
    pub fn truncate(&mut self, len: usize) {
        match &mut self.storage {
            SmallStorage::Inline { len: current, .. } => {
                *current = (*current as usize).min(len) as u32;
            }
            SmallStorage::Heap(values) => values.truncate(len),
        }
    }

    /// Clear all values while retaining the current representation.
    pub fn clear(&mut self) {
        self.truncate(0);
    }

    fn promote(&mut self, required: usize, arena: &mut Arena<'arena, '_>) -> Result<()> {
        if matches!(self.storage, SmallStorage::Heap(_)) {
            if let SmallStorage::Heap(values) = &mut self.storage {
                return values.reserve_in(required.saturating_sub(values.len()), arena);
            }
        }
        let inline_len = self.len();
        let grown = N.saturating_mul(2).max(4);
        let capacity = required.max(grown);
        let mut replacement = CompactVec::with_capacity_in(capacity, arena)?;
        if let SmallStorage::Inline { values, .. } = &self.storage {
            for value in values.iter().take(inline_len) {
                // SAFETY: inline_len is advanced only after this slot is written.
                let value = unsafe { value.assume_init() };
                replacement.push_in(value, arena)?;
            }
        }
        self.storage = SmallStorage::Heap(replacement);
        Ok(())
    }
}
