//! Compact single-value and nullable-reference wrappers.

use compact_core::{Arena, ArenaAllocation, CompactValue, Offset32};
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::{Deref, DerefMut};

use crate::Result;

/// An arena-owned value represented by a unique allocation token.
///
/// Dropping the box runs `T::drop` once and returns its bytes to the arena.
pub struct CompactBox<'arena, T: CompactValue> {
    allocation: ArenaAllocation<'arena, T>,
}

impl<'arena, T: CompactValue> CompactBox<'arena, T> {
    /// Allocate and store one value in `arena`.
    pub fn new_in(value: T, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        Ok(Self {
            allocation: arena.alloc_owned_value(value)?,
        })
    }

    /// Borrow the stored value.
    pub fn get<'view>(&'view self, arena: &'view Arena<'arena, '_>) -> Result<&'view T> {
        arena.validate_owned(&self.allocation)?;
        Ok(&self.allocation.as_slice()[0])
    }

    /// Mutably borrow the stored value.
    pub fn get_mut<'view>(
        &'view mut self,
        arena: &'view mut Arena<'arena, '_>,
    ) -> Result<&'view mut T> {
        arena.validate_owned(&self.allocation)?;
        Ok(&mut self.allocation.as_mut_slice()[0])
    }
}

impl<T: CompactValue> Deref for CompactBox<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.allocation
            .as_slice()
            .first()
            .expect("CompactBox always owns one initialized value")
    }
}

impl<T: CompactValue> DerefMut for CompactBox<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.allocation
            .as_mut_slice()
            .first_mut()
            .expect("CompactBox always owns one initialized value")
    }
}

impl<T: CompactValue> AsRef<T> for CompactBox<'_, T> {
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T: CompactValue> AsMut<T> for CompactBox<'_, T> {
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T: CompactValue + fmt::Debug> fmt::Debug for CompactBox<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
    }
}

impl<T: CompactValue + PartialEq> PartialEq for CompactBox<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: CompactValue + Eq> Eq for CompactBox<'_, T> {}

impl<T: CompactValue + PartialOrd> PartialOrd for CompactBox<'_, T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        (**self).partial_cmp(&**other)
    }
}

impl<T: CompactValue + Ord> Ord for CompactBox<'_, T> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        (**self).cmp(&**other)
    }
}

impl<T: CompactValue + Hash> Hash for CompactBox<'_, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

// SAFETY: the owner token is movable and its drop glue destroys T before
// releasing the allocation. T's own compact-move guarantees are explicit.
unsafe impl<T: CompactValue> CompactValue for CompactBox<'_, T> {}

/// An explicit four-byte nullable, non-owning compact offset.
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct CompactOption<'arena, T> {
    offset: Offset32<'arena, T>,
}

impl<'arena, T> CompactOption<'arena, T> {
    /// Construct the empty compact option.
    pub const fn none() -> Self {
        Self {
            offset: Offset32::null(),
        }
    }

    /// Construct an option from a live compact offset.
    pub const fn some(offset: Offset32<'arena, T>) -> Self {
        Self { offset }
    }

    /// Return whether a value is present.
    pub const fn is_some(self) -> bool {
        !self.offset.is_null()
    }

    /// Return whether no value is present.
    pub const fn is_none(self) -> bool {
        self.offset.is_null()
    }

    /// Return the underlying offset, which is null for `None`.
    pub const fn offset(self) -> Offset32<'arena, T> {
        self.offset
    }

    /// Resolve the optional value through its branded arena.
    pub fn get<'view>(self, arena: &'view Arena<'arena, '_>) -> Result<Option<&'view T>>
    where
        T: Copy,
    {
        if self.is_none() {
            Ok(None)
        } else {
            Ok(Some(arena.get(self.offset)?))
        }
    }

    /// Mutably resolve the optional value through its branded arena.
    pub fn get_mut<'view>(self, arena: &'view mut Arena<'arena, '_>) -> Result<Option<&'view mut T>>
    where
        T: Copy,
    {
        if self.is_none() {
            Ok(None)
        } else {
            Ok(Some(arena.get_mut(self.offset)?))
        }
    }
}

// SAFETY: CompactOption stores only a branded offset and owns no allocation.
unsafe impl<T> CompactValue for CompactOption<'_, T> {}
