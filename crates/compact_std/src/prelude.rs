//! Common imports for process-cage compact applications.

pub use crate::{
    compact, compact_format, Box, CageConfig, CollectionError, CompactBitVec, CompactBox,
    CompactBytes, CompactEnum, CompactHashMap, CompactHashSet, CompactInterner, CompactOption,
    CompactOsString, CompactPathBuf, CompactRing, CompactRuntime, CompactSlab, CompactSmallVec,
    CompactString, CompactValue, CompactVec, CompactVecDeque, FrozenBuilder, FrozenGraph,
    FrozenValue, HashMap, HashSet, OsString, PathBuf, Result, ScratchRegion, SlabHandle, String,
    TryClone, TryExtend, TryFromIterator, TryToCompactString, Vec, VecDeque,
};
#[cfg(feature = "serde")]
pub use crate::{CompactDeserialize, CompactDeserializeSeed};
