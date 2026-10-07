//! Optional arena-local interning for immutable byte strings.

use core::marker::PhantomData;

use compact_core::{Arena, ArenaAllocation, CompactValue};

use crate::{CollectionError, CompactVec, Result};

struct InternEntry<'arena> {
    bytes: ArenaAllocation<'arena, u8>,
}

// SAFETY: moving the entry transfers its unique byte allocation owner.
unsafe impl CompactValue for InternEntry<'_> {}

/// A compact interner-local identifier.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InternId<'arena> {
    owner_id: u32,
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
/// Entries are searched linearly. Each canonical payload has one owned arena
/// allocation, and dropping the interner releases its table and payloads.
pub struct CompactInterner<'arena> {
    entries: CompactVec<'arena, InternEntry<'arena>>,
    identity: ArenaAllocation<'arena, u8>,
}

// SAFETY: moving an interner transfers its vector, identity, and canonical
// payload allocation owners without changing their address-independent data.
unsafe impl CompactValue for CompactInterner<'_> {}

impl<'arena> CompactInterner<'arena> {
    /// Create a table and allocate a unique arena-local identity.
    pub fn new_in(arena: &mut Arena<'arena, '_>) -> Result<Self> {
        let identity = arena.alloc_owned_slice::<u8>(0)?;
        Ok(Self {
            entries: CompactVec::new_in(arena),
            identity,
        })
    }

    /// Return the number of canonical payloads.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Intern an immutable byte payload, returning its canonical compact ID.
    pub fn intern_bytes(
        &mut self,
        bytes: &[u8],
        arena: &mut Arena<'arena, '_>,
    ) -> Result<InternId<'arena>> {
        for (index, entry) in self.entries.as_slice(arena)?.iter().enumerate() {
            if entry.bytes.as_slice() == bytes {
                return self.id(index);
            }
        }
        let mut canonical = arena.alloc_owned_slice::<u8>(bytes.len())?;
        canonical.extend_copy(bytes)?;
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
        &'view self,
        id: InternId<'arena>,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<&'view [u8]> {
        let entry = self.entry(id, arena)?;
        Ok(entry.bytes.as_slice())
    }

    /// Borrow a canonical UTF-8 string named by `id`.
    pub fn resolve_str<'view>(
        &'view self,
        id: InternId<'arena>,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<&'view str> {
        core::str::from_utf8(self.resolve_bytes(id, arena)?)
            .map_err(|_| CollectionError::InvalidUtf8)
    }

    fn entry<'view>(
        &'view self,
        id: InternId<'arena>,
        arena: &'view Arena<'arena, '_>,
    ) -> Result<&'view InternEntry<'arena>> {
        if id.owner_id != self.identity.allocation_id() {
            return Err(CollectionError::StaleHandle);
        }
        self.entries
            .get(id.index as usize, arena)?
            .ok_or(CollectionError::StaleHandle)
    }

    fn id(&self, index: usize) -> Result<InternId<'arena>> {
        Ok(InternId {
            owner_id: self.identity.allocation_id(),
            index: u32::try_from(index).map_err(|_| CollectionError::CapacityOverflow)?,
            marker: PhantomData,
        })
    }
}
