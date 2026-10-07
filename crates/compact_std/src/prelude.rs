//! Common imports for compact arena applications.

pub use crate::{
    arena, compact, Arena, Box, CollectionError, CompactBitVec, CompactBytes, CompactEnum,
    CompactHashMap, CompactHashMapEntry, CompactHashMapIter, CompactHashMapIterMut, CompactHashSet,
    CompactInterner, CompactOption, CompactRing, CompactSlab, CompactSmallVec, CompactStore,
    CompactStringWriter, CompactValue, CompactVecDeque, CompactVecDequeIter, CoreError, HashMap,
    HashSet, InternId, Result, RootHandle, SlabHandle, StdArena, StdBacking, StoreRoot, String,
    Vec, COMPACT_BYTES_INLINE_CAPACITY,
};
pub use compact_core::{ArenaAllocation, Offset32, OffsetSlice32};
