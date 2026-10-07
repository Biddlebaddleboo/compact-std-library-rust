#![no_std]
#![forbid(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs)]
//! Backend-independent compact memory primitives.
//!
//! V2.1.0 is the supported contract. The core crate uses only `core` and
//! provides scoped arenas, four-byte offsets, owned arena allocations, and
//! packed-field primitives. Use [`with_arena`] to create a scoped arena over a
//! backend-provided stable memory region.

mod abi;
mod allocation;
mod arena;
mod backing;
mod bytes;
mod error;
mod layout;
mod native;
mod offset;
mod packed;

pub use abi::{
    CompactAbiVersion, ABI_VERSION, MAX_ARENA_BYTES, MIN_ARENA_BYTES, NULL_OFFSET,
    OFFSET_WIDTH_BYTES,
};
pub use allocation::{ArenaAllocation, CompactValue};
pub use arena::{with_arena, with_arena_attached, with_arena_persistent, Arena};
pub use backing::StableBacking;
pub use bytes::ByteRange32;
pub use error::{Error, Result};
pub use layout::{bits_required, checked_align_up, smallest_word, StorageWord};
pub use offset::{Offset32, OffsetSlice32};
pub use packed::{read_bits, validate_bit_range, write_bits, BitField, PackedWord};

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests;
