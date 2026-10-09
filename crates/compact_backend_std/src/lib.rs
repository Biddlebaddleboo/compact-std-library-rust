//! Standard-library process-wide compact cage runtime.

#![forbid(unsafe_op_in_unsafe_fn)]

#[cfg(not(target_pointer_width = "64"))]
compile_error!("compact_backend_std requires a 64-bit process ABI");

#[cfg(test)]
mod allocator_model;
mod cage;
mod scratch;

pub use cage::{AllocatorStats, CageAllocation, CageConfig, CompactRuntime};
pub use compact_core::{
    bits_required, checked_align_up, smallest_word, validate_bit_range, BitField,
    CompactAbiVersion, CompactValue, Error as CoreError, Offset32, OffsetSlice32, PackedWord,
    Result as CoreResult, StorageWord, ABI_VERSION, MAX_CAGE_BYTES, MIN_CAGE_BYTES, NULL_OFFSET,
    OFFSET_WIDTH_BYTES,
};
pub use scratch::ScratchRegion;
