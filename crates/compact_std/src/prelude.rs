//! Common imports for compact arena applications.

pub use crate::{
    arena, compact, Arena, Box, CollectionError, CompactBitVec, CompactBytes, CompactEnum,
    CompactInterner, CompactOption, CompactSlab, CompactSmallVec, CompactStore,
    CompactStringWriter, CompactValue, CoreError, InternId, Result, RootHandle, SlabHandle,
    StdArena, StdBacking, StoreRoot, String, Vec, COMPACT_BYTES_INLINE_CAPACITY,
};
pub use compact_core::{ArenaAllocation, Offset32, OffsetSlice32};
