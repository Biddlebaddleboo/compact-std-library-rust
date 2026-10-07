//! JSON helpers that construct compact owners directly in the process cage.

use crate::{CompactDeserialize, CompactDeserializeSeed};
use serde::de::DeserializeSeed;

/// Deserialize JSON bytes into a compact value.
pub fn from_slice<'de, T: CompactDeserialize<'de>>(
    input: &'de [u8],
) -> Result<T, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let value = CompactDeserializeSeed::<T>::new().deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

/// Deserialize a JSON string into a compact value.
pub fn from_str<'de, T: CompactDeserialize<'de>>(input: &'de str) -> Result<T, serde_json::Error> {
    from_slice(input.as_bytes())
}
