//! Cage-backed byte and string interning.

use compact_core::CompactValue;
use core::marker::PhantomData;

use crate::{CollectionError, CompactBytes, CompactVec, Result};

/// Stable index of an interned byte string.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct InternId {
    index: u32,
    marker: PhantomData<fn() -> ()>,
}
impl InternId {
    /// Return the stable insertion index.
    pub const fn index(self) -> usize {
        self.index as usize
    }
}
unsafe impl CompactValue for InternId {}

/// Append-only string interner whose ID table is a compact vector.
pub struct CompactInterner {
    strings: CompactVec<CompactBytes>,
}
impl CompactInterner {
    /// Construct an empty interner.
    pub fn new() -> Self {
        Self {
            strings: CompactVec::new(),
        }
    }
    /// Return the number of interned byte strings.
    pub fn len(&self) -> usize {
        self.strings.len()
    }
    /// Return whether no strings have been interned.
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
    /// Intern bytes and return a stable ID.
    pub fn intern_bytes(&mut self, bytes: &[u8]) -> Result<InternId> {
        if let Some(index) = self
            .strings
            .iter()
            .position(|value| value.as_slice() == bytes)
        {
            return Ok(InternId {
                index: index as u32,
                marker: PhantomData,
            });
        }
        let index = u32::try_from(self.len()).map_err(|_| CollectionError::CapacityOverflow)?;
        self.strings.push(CompactBytes::from_slice(bytes)?)?;
        Ok(InternId {
            index,
            marker: PhantomData,
        })
    }
    /// Intern UTF-8 text and return a stable ID.
    pub fn intern_str(&mut self, value: &str) -> Result<InternId> {
        self.intern_bytes(value.as_bytes())
    }
    /// Resolve an interned byte string.
    pub fn resolve_bytes(&self, id: InternId) -> Option<&[u8]> {
        self.strings
            .get(id.index as usize)
            .map(CompactBytes::as_slice)
    }
    /// Resolve an interned UTF-8 string.
    pub fn resolve_str(&self, id: InternId) -> Result<Option<&str>> {
        self.resolve_bytes(id)
            .map(|bytes| core::str::from_utf8(bytes).map_err(|_| CollectionError::InvalidUtf8))
            .transpose()
    }
}
impl Default for CompactInterner {
    fn default() -> Self {
        Self::new()
    }
}
// SAFETY: all strings are stored in cage-owned CompactBytes values.
unsafe impl CompactValue for CompactInterner {}
