//! TOML helpers backed by direct compact `DeserializeSeed` visitors.

use compact_core::Arena;
use serde::de::DeserializeSeed;

use crate::{CompactDeserialize, CompactDeserializeSeed};

/// Deserialize a TOML document directly into a compact value.
pub fn from_str_in<'de, 'arena, 'memory, T>(
    input: &'de str,
    arena: &mut Arena<'arena, 'memory>,
) -> Result<T, toml::de::Error>
where
    T: CompactDeserialize<'de, 'arena>,
{
    let deserializer = toml::de::Deserializer::new(input);
    CompactDeserializeSeed::<T>::new(arena).deserialize(deserializer)
}
