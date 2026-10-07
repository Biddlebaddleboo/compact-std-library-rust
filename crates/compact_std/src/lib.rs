//! Single-dependency facade for the V2.4 process-wide compact cage.

pub mod ffi;
pub mod prelude;

pub use compact_backend_std::{CageAllocation, CageConfig, CompactRuntime, ScratchRegion};
pub use compact_collections::{
    compact_format, CollectionError, CompactBitVec, CompactBox, CompactBuildHasher, CompactBytes,
    CompactComponent, CompactComponentKind, CompactComponents, CompactEnum, CompactHashMap,
    CompactHashMapIter, CompactHashMapIterMut, CompactHashSet, CompactInterner, CompactOption,
    CompactOsStr, CompactOsString, CompactPath, CompactPathBuf, CompactPathDisplay, CompactRing,
    CompactSlab, CompactSmallVec, CompactString, CompactStringWriter, CompactVec, CompactVecDeque,
    CompactVecDequeIter, InternId, Result, SlabHandle, TryClone, TryExtend, TryFromIterator,
    TryToCompactString, COMPACT_BYTES_INLINE_CAPACITY,
};
pub use compact_core::{
    bits_required, checked_align_up, read_bits, smallest_word, validate_bit_range, write_bits,
    BitField, ByteRange32, CompactAbiVersion, CompactValue, Error as CoreError, Offset32,
    OffsetSlice32, PackedWord, StorageWord, ABI_VERSION, MAX_CAGE_BYTES, MIN_CAGE_BYTES,
    NULL_OFFSET, OFFSET_WIDTH_BYTES,
};
pub use compact_frozen::{
    FrozenBuilder, FrozenBytes, FrozenError, FrozenGraph, FrozenMap, FrozenOsString, FrozenPathBuf,
    FrozenResult, FrozenSet, FrozenString, FrozenValue, FrozenVec,
};
pub use compact_macros::{compact, CompactDeserialize, FrozenValue};
#[cfg(feature = "json")]
pub use compact_serde::json;
#[cfg(feature = "toml")]
pub use compact_serde::toml;
#[cfg(feature = "serde")]
pub use compact_serde::{CompactDeserialize, CompactDeserializeSeed};
pub use ffi::{compact_std_ffi_bytes_free, FfiByteBuffer};

/// Compact owner matching the familiar `Box` name.
pub type Box<T> = CompactBox<T>;
/// Compact string type.
pub type String = CompactString;
/// Compact vector type.
pub type Vec<T> = CompactVec<T>;
/// Compact double-ended queue type.
pub type VecDeque<T> = CompactVecDeque<T>;
/// Randomized compact map type.
pub type HashMap<K, V, S = CompactBuildHasher> = CompactHashMap<K, V, S>;
/// Randomized compact set type.
pub type HashSet<T, S = CompactBuildHasher> = CompactHashSet<T, S>;
/// Compact operating-system string type.
pub type OsString = CompactOsString;
/// Compact path buffer type.
pub type PathBuf = CompactPathBuf;

/// Implementation paths referenced by proc-macro expansions.
#[doc(hidden)]
pub mod __private {
    pub use compact_collections as collections;
    pub use compact_core as core;
    pub use compact_frozen as frozen;
    #[cfg(feature = "serde")]
    pub use compact_serde as serde;
}
