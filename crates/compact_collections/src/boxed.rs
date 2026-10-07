//! Four-byte cage-owned values and nullable compact offsets.

use compact_backend_std::{CageAllocation, CompactRuntime};
use compact_core::{CompactValue, Offset32};
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::{Deref, DerefMut};

use crate::Result;

/// A compact value owner represented by one cage offset.
pub struct CompactBox<T: CompactValue> {
    allocation: CageAllocation<T>,
}

impl<T: CompactValue> CompactBox<T> {
    /// Allocate and store one compact value.
    pub fn new(value: T) -> Result<Self> {
        Ok(Self {
            allocation: CompactRuntime::alloc_owned_value(value)?,
        })
    }
    /// Borrow the stored value.
    pub fn get(&self) -> &T {
        &self.allocation.as_slice()[0]
    }
    /// Mutably borrow the stored value.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.allocation.as_mut_slice()[0]
    }
}

impl<T: CompactValue> Deref for CompactBox<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.get()
    }
}
impl<T: CompactValue> DerefMut for CompactBox<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.get_mut()
    }
}
impl<T: CompactValue> AsRef<T> for CompactBox<T> {
    fn as_ref(&self) -> &T {
        self
    }
}
impl<T: CompactValue> AsMut<T> for CompactBox<T> {
    fn as_mut(&mut self) -> &mut T {
        self
    }
}
impl<T: CompactValue + fmt::Debug> fmt::Debug for CompactBox<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
impl<T: CompactValue + PartialEq> PartialEq for CompactBox<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl<T: CompactValue + Eq> Eq for CompactBox<T> {}
impl<T: CompactValue + PartialOrd> PartialOrd for CompactBox<T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        (**self).partial_cmp(&**other)
    }
}
impl<T: CompactValue + Ord> Ord for CompactBox<T> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        (**self).cmp(&**other)
    }
}
impl<T: CompactValue + Hash> Hash for CompactBox<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}
// SAFETY: moving transfers the sole cage owner, which drops T exactly once.
unsafe impl<T: CompactValue> CompactValue for CompactBox<T> {}

/// A four-byte nullable, non-owning cage offset.
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct CompactOption<T> {
    offset: Offset32<T>,
}

impl<T> CompactOption<T> {
    /// Construct the empty compact option.
    pub const fn none() -> Self {
        Self {
            offset: Offset32::null(),
        }
    }
    /// Construct an option from a live compact offset.
    pub const fn some(offset: Offset32<T>) -> Self {
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
    /// Return the stored offset, null when empty.
    pub const fn offset(self) -> Offset32<T> {
        self.offset
    }
}
// SAFETY: the descriptor contains one offset and no ownership or native pointer.
unsafe impl<T> CompactValue for CompactOption<T> {}
