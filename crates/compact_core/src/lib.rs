#![no_std]
#![forbid(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs)]
//! Backend-independent compact memory primitives.
//!
//! The core crate owns the V1 ABI and uses only `core`. Use [`with_arena`] to
//! create a scoped arena over a backend-provided stable memory region.

mod abi;
mod arena;
mod backing;
mod error;
mod layout;
mod native;
mod offset;
mod packed;

pub use abi::{
    CompactAbiVersion, ABI_V1, MAX_ARENA_BYTES, MIN_ARENA_BYTES, NULL_OFFSET, OFFSET_WIDTH_BYTES,
};
pub use arena::{with_arena, Arena};
pub use backing::StableBacking;
pub use error::{Error, Result};
pub use layout::{bits_required, checked_align_up, smallest_word, StorageWord};
pub use offset::{Offset32, OffsetSlice32};
pub use packed::{validate_bit_range, BitField, PackedWord};

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests;
