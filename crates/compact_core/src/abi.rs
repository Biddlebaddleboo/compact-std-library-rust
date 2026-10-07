//! V2.1.0 compact representation rules.
//!
//! Compact references use 32-bit byte offsets. Packed fields use LSB-first bit
//! numbering, and multi-byte packed words use the target's native byte order.
//! The four-byte offset representation and byte unit are stable V2.1.0
//! contract rules.
//!
//! Arena bytes are runtime memory, not a persistent or cross-process file
//! format. Arena alignment is established from the actual base address plus a
//! checked byte offset. No `repr(packed)` layout is required.

/// The compact ABI version understood by this crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CompactAbiVersion {
    /// Major representation version.
    pub major: u16,
    /// Minor representation version.
    pub minor: u16,
}

/// Current supported compact ABI version.
pub const ABI_VERSION: CompactAbiVersion = CompactAbiVersion { major: 2, minor: 1 };

/// Maximum logical span of one compact arena: 2^32 bytes.
pub const MAX_ARENA_BYTES: u64 = 1_u64 << 32;

/// Smallest backing that can hold allocator state and one byte allocation.
pub const MIN_ARENA_BYTES: usize = 60;

/// Width in bytes of an [`Offset32`](crate::Offset32) payload.
pub const OFFSET_WIDTH_BYTES: usize = 4;

/// Reserved raw offset representing no allocation.
///
/// Offset zero is reserved, so the first arena allocation starts at a positive
/// byte offset. An arena may still have a logical capacity of exactly 2^32
/// bytes; its last addressable byte then has offset `u32::MAX`.
pub const NULL_OFFSET: u32 = 0;
