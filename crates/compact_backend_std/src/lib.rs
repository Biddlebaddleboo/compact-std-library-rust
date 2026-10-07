//! Standard-library backing for the backend-independent compact arena.
//!
//! `StdBacking` owns one fixed-size allocation. Use its [`with_arena`](StdBacking::with_arena)
//! method to work within a generative arena scope. [`StdArena`] offers the
//! single-call convenience form when the backing should be dropped immediately
//! after the callback.

mod memory;

pub use compact_core::{
    bits_required, checked_align_up, smallest_word, validate_bit_range, Arena, BitField,
    CompactAbiVersion, Error as CoreError, Offset32, OffsetSlice32, PackedWord,
    Result as CoreResult, StableBacking, StorageWord, ABI_V1, MAX_ARENA_BYTES, MIN_ARENA_BYTES,
    NULL_OFFSET, OFFSET_WIDTH_BYTES,
};
pub use memory::{StdArena, StdBackendError, StdBacking};
