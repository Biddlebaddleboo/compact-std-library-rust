//! Common imports for compact arena applications.

pub use crate::{
    arena, compact, format_in, Arena, Box, CloneIn, CollectionError, CompactBitVec, CompactBytes,
    CompactComponent, CompactComponentKind, CompactComponents, CompactEnum, CompactHashMap,
    CompactHashMapEntry, CompactHashMapIter, CompactHashMapIterMut, CompactHashSet,
    CompactInterner, CompactOption, CompactOsStr, CompactOsString, CompactPath, CompactPathBuf,
    CompactPathDisplay, CompactRing, CompactSlab, CompactSmallVec, CompactStore,
    CompactStringWriter, CompactValue, CompactVecDeque, CompactVecDequeIter, CoreError, ExtendIn,
    FromIteratorIn, HashMap, HashSet, InternId, OsString, PathBuf, Result, RootHandle, SlabHandle,
    StdArena, StdBacking, StoreRoot, String, ToCompactStringIn, Vec, COMPACT_BYTES_INLINE_CAPACITY,
};
pub use compact_core::{ArenaAllocation, Offset32, OffsetSlice32};
