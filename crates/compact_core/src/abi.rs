//! Compact ABI V1 representation rules.
//!
//! V1 uses byte offsets, LSB-first bit numbering within packed numeric words,
//! and each target's native byte order for the in-memory bytes of those words.
//! The four-byte offset representation and its byte unit are the stable V1
//! source/ABI rules. Packed values are runtime memory, not a persistent or
//! cross-process file format; their byte order is not fixed across targets.
//! Arena alignment is established from the actual base address plus a checked
//! byte offset. No `repr(packed)` layout is required.

/// The compact ABI version understood by this crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CompactAbiVersion {
    /// Major representation version.
    pub major: u16,
    /// Minor compatible extension version.
    pub minor: u16,
}

/// Version marker for compact ABI V1.
pub const ABI_V1: CompactAbiVersion = CompactAbiVersion { major: 1, minor: 0 };

/// Maximum logical span of one compact arena: 2^32 bytes.
pub const MAX_ARENA_BYTES: u64 = 1_u64 << 32;

/// Smallest usable arena capacity because byte offset zero is reserved.
pub const MIN_ARENA_BYTES: usize = 2;

/// Width in bytes of an [`Offset32`](crate::Offset32) payload.
pub const OFFSET_WIDTH_BYTES: usize = 4;

/// Reserved raw offset representing no allocation.
///
/// Offset zero is reserved, so the first arena allocation starts at a positive
/// byte offset. An arena may still have a logical capacity of exactly 2^32
/// bytes; its last addressable byte then has offset `u32::MAX`.
pub const NULL_OFFSET: u32 = 0;
