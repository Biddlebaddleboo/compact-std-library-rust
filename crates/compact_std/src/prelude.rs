//! Common imports for compact arena applications.

pub use crate::{
    arena, compact, format_in, freeze_in, Arena, Box, CloneIn, CollectionError, CompactBitVec,
    CompactBytes, CompactComponent, CompactComponentKind, CompactComponents, CompactEnum,
    CompactFreeze, CompactHashMap, CompactHashMapEntry, CompactHashMapIter, CompactHashMapIterMut,
    CompactHashSet, CompactInterner, CompactOption, CompactOsStr, CompactOsString, CompactPath,
    CompactPathBuf, CompactPathDisplay, CompactRing, CompactSlab, CompactSmallVec, CompactStore,
    CompactString, CompactStringWriter, CompactValue, CompactVec, CompactVecDeque,
    CompactVecDequeIter, CoreError, ExtendIn, FfiByteBuffer, FreezeIn, FromIteratorIn, FrozenArena,
    FrozenBuilder, FrozenBytes, FrozenError, FrozenMap, FrozenOsString, FrozenPathBuf,
    FrozenResult, FrozenRoot, FrozenSet, FrozenString, FrozenValue, FrozenVec, FrozenVecDeque,
    HashMap, HashSet, InternId, OsString, PathBuf, Result, RootHandle, SlabHandle, StdArena,
    StdBacking, StoreRoot, String, ToCompactStringIn, Vec, VecDeque, COMPACT_BYTES_INLINE_CAPACITY,
};

#[cfg(feature = "serde")]
pub use crate::{CompactDeserialize, CompactDeserializeSeed};
pub use compact_core::{ArenaAllocation, Offset32, OffsetSlice32};
