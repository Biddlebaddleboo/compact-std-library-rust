//! JSON helpers backed by direct compact `DeserializeSeed` visitors.

use compact_core::Arena;
use serde::de::DeserializeSeed;

use crate::{CompactDeserialize, CompactDeserializeSeed};

/// Deserialize JSON bytes directly into a compact value.
pub fn from_slice_in<'de, 'arena, 'memory, T>(
    input: &'de [u8],
    arena: &mut Arena<'arena, 'memory>,
) -> Result<T, serde_json::Error>
where
    T: CompactDeserialize<'de, 'arena>,
{
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let value = CompactDeserializeSeed::<T>::new(arena).deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

/// Deserialize a JSON string directly into a compact value.
pub fn from_str_in<'de, 'arena, 'memory, T>(
    input: &'de str,
    arena: &mut Arena<'arena, 'memory>,
) -> Result<T, serde_json::Error>
where
    T: CompactDeserialize<'de, 'arena>,
{
    from_slice_in(input.as_bytes(), arena)
}
