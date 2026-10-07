//! V2.4 compact cage representation rules.

/// The compact runtime ABI version understood by this crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CompactAbiVersion {
    /// Major representation version.
    pub major: u16,
    /// Minor representation version.
    pub minor: u16,
}

/// Current compact runtime ABI version.
pub const ABI_VERSION: CompactAbiVersion = CompactAbiVersion { major: 2, minor: 4 };

/// Maximum logical span of the process cage.
pub const MAX_CAGE_BYTES: u64 = 1_u64 << 32;

/// Smallest supported process cage.
pub const MIN_CAGE_BYTES: usize = 64;

/// Width in bytes of a compact offset payload.
pub const OFFSET_WIDTH_BYTES: usize = 4;

/// Reserved raw offset representing no allocation.
pub const NULL_OFFSET: u32 = 0;
