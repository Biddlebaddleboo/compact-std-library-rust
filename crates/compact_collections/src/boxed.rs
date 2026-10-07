//! Compact single-value and nullable-reference wrappers.

use compact_core::{Arena, Offset32};

use crate::Result;

/// An arena-owned value represented by one four-byte offset.
///
/// Dropping this wrapper does not reclaim bytes from a monotonic arena.
pub struct CompactBox<'arena, T: Copy> {
    offset: Offset32<'arena, T>,
}

impl<'arena, T: Copy> CompactBox<'arena, T> {
    /// Allocate and store one value in `arena`.
    pub fn new_in(value: T, arena: &mut Arena<'arena, '_>) -> Result<Self> {
        Ok(Self {
            offset: arena.alloc_value(value)?,
        })
    }

    /// Borrow the stored value.
    pub fn get<'view>(&self, arena: &'view Arena<'arena, '_>) -> Result<&'view T> {
        Ok(arena.get(self.offset)?)
    }

    /// Mutably borrow the stored value.
    pub fn get_mut<'view>(&self, arena: &'view mut Arena<'arena, '_>) -> Result<&'view mut T> {
        Ok(arena.get_mut(self.offset)?)
    }

    /// Return the compact offset without exposing a native arena pointer.
    pub const fn offset(&self) -> Offset32<'arena, T> {
        self.offset
    }
}

/// An explicit four-byte nullable compact offset.
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
