//! Error types shared by compact containers.

use core::fmt;

/// Errors returned by compact collection operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollectionError {
    /// A checked arena operation failed.
    Core(compact_core::Error),
    /// A requested length or capacity cannot fit compact metadata.
    CapacityOverflow,
    /// A slab handle is vacant, stale, or belongs to another slab.
    StaleHandle,
    /// A byte sequence is not valid UTF-8.
    InvalidUtf8,
    /// A compact enum discriminant does not name a declared variant.
    InvalidCompactValue,
}

/// Result type used by compact collections.
pub type Result<T> = core::result::Result<T, CollectionError>;

impl From<compact_core::Error> for CollectionError {
    fn from(error: compact_core::Error) -> Self {
        Self::Core(error)
    }
}

impl fmt::Display for CollectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => error.fmt(formatter),
            Self::CapacityOverflow => formatter.write_str("compact collection capacity overflowed"),
            Self::StaleHandle => formatter.write_str("compact slab handle is stale or vacant"),
            Self::InvalidUtf8 => formatter.write_str("compact bytes are not valid UTF-8"),
            Self::InvalidCompactValue => {
                formatter.write_str("compact value has an invalid discriminant")
            }
        }
    }
}

impl core::error::Error for CollectionError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Core(error) => Some(error),
            _ => None,
        }
    }
}
