//! Errors returned by compact cage operations.

use core::fmt;

/// Errors returned by compact memory operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The requested process cage exceeds the 32-bit offset domain.
    CageTooLarge,
    /// The requested process cage is too small.
    InvalidCapacity,
    /// The process cage has not been initialized.
    RuntimeNotInitialized,
    /// The process cage was already initialized.
    RuntimeAlreadyInitialized,
    /// The system allocator could not reserve the process cage.
    AllocationFailed,
    /// The requested allocation does not fit in the process cage.
    AllocationExhausted,
    /// An address or size calculation overflowed.
    OffsetOverflow,
    /// The offset is null or otherwise cannot name an allocation.
    InvalidOffset,
    /// The requested range is outside a live allocation.
    OutOfBounds,
    /// An alignment is invalid or is not met by the address.
    AlignmentError,
    /// A bit range is empty or extends beyond its word.
    InvalidBitRange,
    /// A value has set bits outside the selected packed field.
    ValueDoesNotFit,
    /// The operation received inconsistent initialization metadata.
    InitializationError,
    /// The global allocator lock was poisoned.
    AllocatorPoisoned,
}

/// Core result type.
pub type Result<T> = core::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::CageTooLarge => "process cage exceeds the compact 32-bit offset limit",
            Self::InvalidCapacity => "process cage capacity is too small",
            Self::RuntimeNotInitialized => "compact runtime has not been initialized",
            Self::RuntimeAlreadyInitialized => "compact runtime has already been initialized",
            Self::AllocationFailed => "system allocator could not reserve the process cage",
            Self::AllocationExhausted => "compact process cage is exhausted",
            Self::OffsetOverflow => "compact cage offset arithmetic overflowed",
            Self::InvalidOffset => "invalid or null compact offset",
            Self::OutOfBounds => "compact memory range is out of bounds",
            Self::AlignmentError => "compact memory alignment requirement was not met",
            Self::InvalidBitRange => "packed bit range is invalid",
            Self::ValueDoesNotFit => "value does not fit in the packed field",
            Self::InitializationError => "initialization metadata is inconsistent",
            Self::AllocatorPoisoned => "compact cage allocator lock was poisoned",
        };
        formatter.write_str(message)
    }
}

impl core::error::Error for Error {}
