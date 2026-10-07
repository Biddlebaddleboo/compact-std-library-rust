//! Direct cage-backed Serde seeds and visitors.

use compact_backend_std::CompactValue;
use compact_collections::{
    CompactBytes, CompactHashMap, CompactHashSet, CompactOsString, CompactPathBuf, CompactString,
    CompactVec, CompactVecDeque,
};
use core::{fmt, hash::Hash, marker::PhantomData};
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

/// Construct a value directly from a Serde deserializer.
pub trait CompactDeserialize<'de>: Sized {
    /// Deserialize this value, allocating compact fields in the process cage.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>;
}

/// A seed for nested compact deserialization.
pub struct CompactDeserializeSeed<T>(PhantomData<fn() -> T>);
impl<T> CompactDeserializeSeed<T> {
    /// Create a seed for `T`.
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}
impl<T> Default for CompactDeserializeSeed<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<'de, T: CompactDeserialize<'de>> DeserializeSeed<'de> for CompactDeserializeSeed<T> {
    type Value = T;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<T, D::Error> {
        T::deserialize(deserializer)
    }
}

macro_rules! scalar_impls {
    ($($ty:ty),* $(,)?) => {$ (
        impl<'de> CompactDeserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                <$ty as serde::Deserialize<'de>>::deserialize(deserializer)
            }
        }
    )*};
}
scalar_impls!(
    (),
    bool,
    char,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    f32,
    f64,
    std::string::String
);

struct StringVisitor;
impl<'de> Visitor<'de> for StringVisitor {
    type Value = CompactString;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a UTF-8 string")
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        CompactString::from_str(value).map_err(E::custom)
    }
    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
        self.visit_str(value)
    }
    fn visit_string<E: de::Error>(self, value: std::string::String) -> Result<Self::Value, E> {
        self.visit_str(&value)
    }
}
impl<'de> CompactDeserialize<'de> for CompactString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(StringVisitor)
    }
}

struct BytesVisitor;
impl<'de> Visitor<'de> for BytesVisitor {
    type Value = CompactBytes;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a byte string or byte sequence")
    }
    fn visit_bytes<E: de::Error>(self, value: &[u8]) -> Result<Self::Value, E> {
        CompactBytes::from_slice(value).map_err(E::custom)
    }
    fn visit_borrowed_bytes<E: de::Error>(self, value: &'de [u8]) -> Result<Self::Value, E> {
        self.visit_bytes(value)
    }
    fn visit_byte_buf<E: de::Error>(self, value: std::vec::Vec<u8>) -> Result<Self::Value, E> {
        self.visit_bytes(&value)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let lower_bound = seq.size_hint().unwrap_or(0);
        let mut bytes = CompactBytes::new();
        let mut source_error = None;
        bytes
            .try_extend_fallible(lower_bound, || seq.next_element::<u8>(), &mut source_error)
            .map_err(de::Error::custom)?;
        if let Some(error) = source_error {
            return Err(error);
        }
        Ok(bytes)
    }
}
impl<'de> CompactDeserialize<'de> for CompactBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_bytes(BytesVisitor)
    }
}

macro_rules! compact_string_like {
    ($ty:ty, $visitor:ident, $make:expr, $expecting:literal) => {
        struct $visitor;
        impl<'de> Visitor<'de> for $visitor {
            type Value = $ty;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str($expecting)
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                ($make)(value).map_err(E::custom)
            }
            fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
                self.visit_str(value)
            }
            fn visit_string<E: de::Error>(
                self,
                value: std::string::String,
            ) -> Result<Self::Value, E> {
                self.visit_str(&value)
            }
        }
        impl<'de> CompactDeserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                deserializer.deserialize_str($visitor)
            }
        }
    };
}
compact_string_like!(
    CompactOsString,
    OsStringVisitor,
    |value: &str| CompactOsString::from(value),
    "an operating-system string"
);
compact_string_like!(
    CompactPathBuf,
    PathBufVisitor,
    |value: &str| CompactPathBuf::from(value),
    "a path string"
);

struct OptionVisitor<T>(PhantomData<fn() -> T>);
impl<'de, T: CompactDeserialize<'de>> Visitor<'de> for OptionVisitor<T> {
    type Value = Option<T>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an optional value")
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        T::deserialize(deserializer).map(Some)
    }
}
impl<'de, T: CompactDeserialize<'de>> CompactDeserialize<'de> for Option<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_option(OptionVisitor(PhantomData))
    }
}

struct VecVisitor<T>(PhantomData<fn() -> T>);
impl<'de, T: CompactValue + CompactDeserialize<'de>> Visitor<'de> for VecVisitor<T> {
    type Value = CompactVec<T>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a sequence")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let lower_bound = seq.size_hint().unwrap_or(0);
        let mut values = CompactVec::new();
        let mut source_error = None;
        values
            .try_extend_fallible(
                lower_bound,
                || seq.next_element_seed(CompactDeserializeSeed::<T>::new()),
                &mut source_error,
            )
            .map_err(de::Error::custom)?;
        if let Some(error) = source_error {
            return Err(error);
        }
        Ok(values)
    }
}
impl<'de, T: CompactValue + CompactDeserialize<'de>> CompactDeserialize<'de> for CompactVec<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(VecVisitor(PhantomData))
    }
}

struct DequeVisitor<T>(PhantomData<fn() -> T>);
impl<'de, T: CompactValue + CompactDeserialize<'de>> Visitor<'de> for DequeVisitor<T> {
    type Value = CompactVecDeque<T>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a sequence")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut values = CompactVecDeque::with_capacity(seq.size_hint().unwrap_or(0))
            .map_err(de::Error::custom)?;
        while let Some(value) = seq.next_element_seed(CompactDeserializeSeed::<T>::new())? {
            values.push_back(value).map_err(de::Error::custom)?;
        }
        Ok(values)
    }
}
impl<'de, T: CompactValue + CompactDeserialize<'de>> CompactDeserialize<'de>
    for CompactVecDeque<T>
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(DequeVisitor(PhantomData))
    }
}

struct MapVisitor<K, V>(PhantomData<fn() -> (K, V)>);
impl<'de, K, V> Visitor<'de> for MapVisitor<K, V>
where
    K: CompactValue + Hash + Eq + CompactDeserialize<'de>,
    V: CompactValue + CompactDeserialize<'de>,
{
    type Value = CompactHashMap<K, V>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a map")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut values = CompactHashMap::with_capacity(map.size_hint().unwrap_or(0))
            .map_err(de::Error::custom)?;
        while let Some(key) = map.next_key_seed(CompactDeserializeSeed::<K>::new())? {
            let value = map.next_value_seed(CompactDeserializeSeed::<V>::new())?;
            values.insert(key, value).map_err(de::Error::custom)?;
        }
        Ok(values)
    }
}
impl<'de, K, V> CompactDeserialize<'de> for CompactHashMap<K, V>
where
    K: CompactValue + Hash + Eq + CompactDeserialize<'de>,
    V: CompactValue + CompactDeserialize<'de>,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(MapVisitor(PhantomData))
    }
}

struct SetVisitor<T>(PhantomData<fn() -> T>);
impl<'de, T> Visitor<'de> for SetVisitor<T>
where
    T: CompactValue + Hash + Eq + CompactDeserialize<'de>,
{
    type Value = CompactHashSet<T>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a sequence")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut values = CompactHashSet::with_capacity(seq.size_hint().unwrap_or(0))
            .map_err(de::Error::custom)?;
        while let Some(value) = seq.next_element_seed(CompactDeserializeSeed::<T>::new())? {
            values.insert(value).map_err(de::Error::custom)?;
        }
        Ok(values)
    }
}
impl<'de, T> CompactDeserialize<'de> for CompactHashSet<T>
where
    T: CompactValue + Hash + Eq + CompactDeserialize<'de>,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(SetVisitor(PhantomData))
    }
}
