//! A single-dependency facade for V2.1.0 compact arena programming.
//!
//! V2.1.0 is the supported contract. Familiar collection names are aliases for
//! arena-aware types. Their methods accept an explicit arena or can be
//! shortened locally with [`arena!`]. No global or thread-local arena context
//! is installed.

pub mod prelude;

pub use compact_backend_std::{
    CompactStore, RootHandle, StdArena, StdBackendError, StdBacking, StoreRoot,
};
pub use compact_collections::format_in;
pub use compact_collections::{
    CloneIn, CollectionError, CompactBitVec, CompactBox, CompactBytes, CompactComponent,
    CompactComponentKind, CompactComponents, CompactEnum, CompactHashMap, CompactHashMapEntry,
    CompactHashMapIter, CompactHashMapIterMut, CompactHashSet, CompactInterner, CompactOption,
    CompactOsStr, CompactOsString, CompactPath, CompactPathBuf, CompactPathDisplay, CompactRing,
    CompactSlab, CompactSmallVec, CompactString, CompactStringWriter, CompactVec, CompactVecDeque,
    CompactVecDequeIter, ExtendIn, FromIteratorIn, InternId, Result, SlabHandle, ToCompactStringIn,
    COMPACT_BYTES_INLINE_CAPACITY,
};
pub use compact_core::{
    bits_required, checked_align_up, read_bits, smallest_word, validate_bit_range, write_bits,
    Arena, ArenaAllocation, BitField, ByteRange32, CompactAbiVersion, CompactValue,
    Error as CoreError, Offset32, OffsetSlice32, PackedWord, StableBacking, StorageWord,
    ABI_VERSION, MAX_ARENA_BYTES, MIN_ARENA_BYTES, NULL_OFFSET, OFFSET_WIDTH_BYTES,
};
pub use compact_macros::{arena, compact};

/// Compact aliases mirroring common standard-library type names.
pub type Box<'arena, T> = CompactBox<'arena, T>;
/// Compact string alias.
pub type String<'arena> = CompactString<'arena>;
/// Compact vector alias.
pub type Vec<'arena, T> = CompactVec<'arena, T>;
/// Hash map alias with randomized hashing by default.
pub type HashMap<'arena, K, V, S = std::collections::hash_map::RandomState> =
    CompactHashMap<'arena, K, V, S>;
/// Hash set alias with randomized hashing by default.
pub type HashSet<'arena, T, S = std::collections::hash_map::RandomState> =
    CompactHashSet<'arena, T, S>;
/// Compact OS string alias.
pub type OsString<'arena> = CompactOsString<'arena>;
/// Compact path buffer alias.
pub type PathBuf<'arena> = CompactPathBuf<'arena>;

/// Implementation paths referenced by the proc-macro expansion.
#[doc(hidden)]
pub mod __private {
    /// Public container contracts used by generated code.
    pub use compact_collections as collections;
    /// Public core contracts used by generated code.
    pub use compact_core as core;
}
