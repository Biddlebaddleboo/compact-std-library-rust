//! Allocation-free core errors.

use core::fmt;

/// Errors returned by compact memory operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// A backing region is larger than the V1 4 GiB address domain.
    BackingTooLarge,
    /// The backing region has no usable bytes.
    InvalidCapacity,
    /// The requested allocation does not fit in the remaining arena.
    AllocationExhausted,
    /// An address or size calculation overflowed.
    OffsetOverflow,
    /// The offset is the null sentinel or otherwise cannot name an allocation.
    InvalidOffset,
    /// The requested bytes are not within the arena's allocated prefix.
    OutOfBounds,
    /// An alignment is zero, not a power of two, or is not met by the address.
    AlignmentError,
    /// A bit range is empty or extends beyond its word.
    InvalidBitRange,
    /// A value has set bits outside the selected packed field.
    ValueDoesNotFit,
    /// An initialization operation does not match its allocated region.
    InitializationError,
    /// An allocation owner was used with an arena other than its creator.
    ForeignArena,
    /// The arena-local allocation identity counter is exhausted.
    AllocationIdExhausted,
}

/// Core result type.
pub type Result<T> = core::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::BackingTooLarge => "backing region exceeds the V1 arena limit",
            Self::InvalidCapacity => "backing region cannot hold arena allocator state",
            Self::AllocationExhausted => "compact arena is exhausted",
            Self::OffsetOverflow => "compact arena offset arithmetic overflowed",
            Self::InvalidOffset => "invalid or null compact offset",
            Self::OutOfBounds => "compact memory range is out of bounds",
            Self::AlignmentError => "compact memory alignment requirement was not met",
            Self::InvalidBitRange => "packed bit range is invalid",
            Self::ValueDoesNotFit => "value does not fit in the packed field",
            Self::InitializationError => "initialization does not match the allocated region",
            Self::ForeignArena => "allocation belongs to a different compact arena",
            Self::AllocationIdExhausted => "compact arena allocation identity space is exhausted",
        };
        formatter.write_str(message)
    }
}

impl core::error::Error for Error {}
