//! Optional arena-local interning for immutable byte strings.

use core::marker::PhantomData;

use compact_core::{Arena, ByteRange32};

use crate::{CollectionError, CompactVec, Result};

#[derive(Clone, Copy)]
struct InternEntry<'arena> {
    bytes: ByteRange32<'arena>,
}

/// A compact interner-local identifier.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InternId<'arena> {
    owner_offset: u32,
    index: u32,
    marker: PhantomData<fn(&'arena mut ()) -> &'arena mut ()>,
}

impl InternId<'_> {
    /// Return the zero-based entry index for diagnostics.
    pub const fn index(self) -> usize {
        self.index as usize
    }
}

/// A simple immutable byte/string interner.
///
/// Entries use an eight-byte compact range descriptor and are searched
/// linearly. This avoids a hash-table allocation for tiny intern sets; callers
/// should use it when repeated payloads justify the table metadata.
pub struct CompactInterner<'arena> {
    entries: CompactVec<'arena, InternEntry<'arena>>,
    owner_offset: u32,
}

impl<'arena> CompactInterner<'arena> {
    /// Create a table and allocate a unique one-byte arena identity marker.
    pub fn new_in(arena: &mut Arena<'arena, '_>) -> Result<Self> {
        let owner = arena.alloc_value(0_u8)?;
        Ok(Self {
            entries: CompactVec::new_in(arena),
            owner_offset: owner.as_u32(),
        })
    }

    /// Return the number of canonical payloads.
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return whether the table is empty.
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Intern an immutable byte payload, returning its canonical compact ID.
    pub fn intern_bytes(
        &mut self,
        bytes: &[u8],
        arena: &mut Arena<'arena, '_>,
    ) -> Result<InternId<'arena>> {
        for (index, entry) in self.entries.as_slice(arena)?.iter().enumerate() {
            if arena.get_bytes(entry.bytes)? == bytes {
                return self.id(index);
            }
        }
        let canonical = arena.alloc_bytes(bytes)?;
        let index = self.entries.len();
        self.entries
            .push_in(InternEntry { bytes: canonical }, arena)?;
        self.id(index)
    }

    /// Intern a valid UTF-8 string.
    pub fn intern_str(
        &mut self,
        value: &str,
        arena: &mut Arena<'arena, '_>,
    ) -> Result<InternId<'arena>> {
        self.intern_bytes(value.as_bytes(), arena)
    }

    /// Borrow the canonical bytes named by `id`.
    pub fn resolve_bytes<'view>(
        &self,
        id: InternId<'arena>,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<&'view [u8]> {
        let entry = self.entry(id, arena)?;
        Ok(arena.get_bytes(entry.bytes)?)
    }

    /// Borrow a canonical UTF-8 string named by `id`.
    pub fn resolve_str<'view>(
        &self,
        id: InternId<'arena>,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<&'view str> {
        core::str::from_utf8(self.resolve_bytes(id, arena)?)
            .map_err(|_| CollectionError::InvalidUtf8)
    }

    fn entry<'view>(
        &self,
        id: InternId<'arena>,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<&'view InternEntry<'arena>> {
        if id.owner_offset != self.owner_offset {
            return Err(CollectionError::StaleHandle);
        }
        self.entries
            .get(id.index as usize, arena)?
            .ok_or(CollectionError::StaleHandle)
    }

    fn id(&self, index: usize) -> Result<InternId<'arena>> {
        Ok(InternId {
            owner_offset: self.owner_offset,
            index: u32::try_from(index).map_err(|_| CollectionError::CapacityOverflow)?,
            marker: PhantomData,
        })
    }
}
