//! Direct Serde support for process-cage compact values.

#![forbid(unsafe_code)]

mod deserialize;

pub use deserialize::{CompactDeserialize, CompactDeserializeSeed};

#[cfg(feature = "json")]
pub mod json;
#[cfg(feature = "toml")]
pub mod toml;

/// Paths used by generated compact deserialization implementations.
#[doc(hidden)]
pub mod __private {
    pub use serde;
}
