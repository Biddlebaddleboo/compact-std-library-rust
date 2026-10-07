//! Common imports for compact arena applications.

pub use crate::{
    arena, compact, Arena, Box, CollectionError, CompactBitVec, CompactEnum, CompactInterner,
    CompactOption, CompactSlab, CompactSmallVec, CompactValue, CoreError, InternId, Result,
    SlabHandle, StdArena, StdBacking, String, Vec,
};
pub use compact_core::{ArenaAllocation, Offset32, OffsetSlice32};
