//! TOML helpers that construct compact owners directly in the process cage.

use crate::{CompactDeserialize, CompactDeserializeSeed};
use serde::de::DeserializeSeed;

/// Deserialize a TOML document into a compact value.
pub fn from_str<'de, T: CompactDeserialize<'de>>(input: &'de str) -> Result<T, toml::de::Error> {
    let deserializer = toml::de::Deserializer::new(input);
    CompactDeserializeSeed::<T>::new().deserialize(deserializer)
}
