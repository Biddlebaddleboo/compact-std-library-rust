//! Direct arena-aware Serde seeds and built-in compact visitors.

use core::marker::PhantomData;
use core::{fmt, hash::Hash};
use std::collections::hash_map::RandomState;

use compact_collections::{
    CompactBytes, CompactHashMap, CompactHashSet, CompactOsString, CompactPathBuf, CompactString,
    CompactVec, CompactVecDeque,
};
use compact_core::{Arena, CompactValue};
use serde::de::{self, DeserializeSeed, Deserializer, Error as _, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;

/// Construct a compact value directly from a Serde deserializer.
pub trait CompactDeserialize<'de, 'arena>: Sized {
    /// Deserialize this value while storing owned data in `arena`.
    fn deserialize_in<'memory, D>(
        deserializer: D,
        arena: &mut Arena<'arena, 'memory>,
    ) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>;
}

/// A Serde seed carrying the arena used by nested compact visitors.
pub struct CompactDeserializeSeed<'seed, 'arena, 'memory, T> {
    arena: &'seed mut Arena<'arena, 'memory>,
    marker: PhantomData<fn() -> T>,
}

impl<'seed, 'arena, 'memory, T> CompactDeserializeSeed<'seed, 'arena, 'memory, T> {
    /// Create a seed that stores all owned values in `arena`.
    pub fn new(arena: &'seed mut Arena<'arena, 'memory>) -> Self {
        Self {
            arena,
            marker: PhantomData,
        }
    }
}

impl<'de, 'seed, 'arena, 'memory, T> DeserializeSeed<'de>
    for CompactDeserializeSeed<'seed, 'arena, 'memory, T>
where
    T: CompactDeserialize<'de, 'arena>,
{
    type Value = T;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::deserialize_in(deserializer, self.arena)
    }
}

macro_rules! scalar_impls {
    ($($ty:ty),* $(,)?) => {
        $(
            impl<'de, 'arena> CompactDeserialize<'de, 'arena> for $ty {
                fn deserialize_in<'memory, D>(
                    deserializer: D,
                    _arena: &mut Arena<'arena, 'memory>,
                ) -> Result<Self, D::Error>
                where
                    D: Deserializer<'de>,
                {
                    <$ty as Deserialize<'de>>::deserialize(deserializer)
                }
            }
        )*
    };
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
    f64
);

struct OptionVisitor<'seed, 'arena, 'memory, T> {
    arena: &'seed mut Arena<'arena, 'memory>,
    marker: PhantomData<fn() -> T>,
}

impl<'de, 'seed, 'arena, 'memory, T> Visitor<'de> for OptionVisitor<'seed, 'arena, 'memory, T>
where
    T: CompactDeserialize<'de, 'arena>,
{
    type Value = Option<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an optional compact value")
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(None)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(None)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::deserialize_in(deserializer, self.arena).map(Some)
    }
}

impl<'de, 'arena, T> CompactDeserialize<'de, 'arena> for Option<T>
where
    T: CompactDeserialize<'de, 'arena>,
{
    fn deserialize_in<'memory, D>(
        deserializer: D,
        arena: &mut Arena<'arena, 'memory>,
    ) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_option(OptionVisitor {
            arena,
            marker: PhantomData,
        })
    }
}

struct StringVisitor<'seed, 'arena, 'memory> {
    arena: &'seed mut Arena<'arena, 'memory>,
}

impl<'de, 'seed, 'arena, 'memory> Visitor<'de> for StringVisitor<'seed, 'arena, 'memory> {
    type Value = CompactString<'arena>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a UTF-8 string for compact storage")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        CompactString::from_str_in(value, self.arena).map_err(E::custom)
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(value)
    }
}

impl<'de, 'arena> CompactDeserialize<'de, 'arena> for CompactString<'arena> {
    fn deserialize_in<'memory, D>(
        deserializer: D,
        arena: &mut Arena<'arena, 'memory>,
    ) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(StringVisitor { arena })
    }
}

struct OsStringVisitor<'seed, 'arena, 'memory> {
    arena: &'seed mut Arena<'arena, 'memory>,
}

impl<'de, 'seed, 'arena, 'memory> Visitor<'de> for OsStringVisitor<'seed, 'arena, 'memory> {
    type Value = CompactOsString<'arena>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a UTF-8 string for a compact operating-system string")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        CompactOsString::from(value, self.arena).map_err(E::custom)
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(value)
    }
}

impl<'de, 'arena> CompactDeserialize<'de, 'arena> for CompactOsString<'arena> {
    fn deserialize_in<'memory, D>(
        deserializer: D,
        arena: &mut Arena<'arena, 'memory>,
    ) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(OsStringVisitor { arena })
    }
}

struct PathBufVisitor<'seed, 'arena, 'memory> {
    arena: &'seed mut Arena<'arena, 'memory>,
}

impl<'de, 'seed, 'arena, 'memory> Visitor<'de> for PathBufVisitor<'seed, 'arena, 'memory> {
    type Value = CompactPathBuf<'arena>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a UTF-8 string for a compact path")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        CompactPathBuf::from(value, self.arena).map_err(E::custom)
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(value)
    }
}

impl<'de, 'arena> CompactDeserialize<'de, 'arena> for CompactPathBuf<'arena> {
    fn deserialize_in<'memory, D>(
        deserializer: D,
        arena: &mut Arena<'arena, 'memory>,
    ) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(PathBufVisitor { arena })
    }
}

struct BytesVisitor<'seed, 'arena, 'memory> {
    arena: &'seed mut Arena<'arena, 'memory>,
}

impl<'de, 'seed, 'arena, 'memory> Visitor<'de> for BytesVisitor<'seed, 'arena, 'memory> {
    type Value = CompactBytes<'arena>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a byte string or sequence of bytes")
    }

    fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        CompactBytes::from_slice_in(value, self.arena).map_err(E::custom)
    }

    fn visit_borrowed_bytes<E>(self, value: &'de [u8]) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_bytes(value)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut bytes = CompactBytes::new();
        while let Some(byte) = sequence.next_element::<u8>()? {
            bytes
                .push_in(byte, &mut *self.arena)
                .map_err(A::Error::custom)?;
        }
        Ok(bytes)
    }
}

impl<'de, 'arena> CompactDeserialize<'de, 'arena> for CompactBytes<'arena> {
    fn deserialize_in<'memory, D>(
        deserializer: D,
        arena: &mut Arena<'arena, 'memory>,
    ) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_bytes(BytesVisitor { arena })
    }
}

struct VecVisitor<'seed, 'arena, 'memory, T> {
    arena: &'seed mut Arena<'arena, 'memory>,
    marker: PhantomData<fn() -> T>,
}

impl<'de, 'seed, 'arena, 'memory, T> Visitor<'de> for VecVisitor<'seed, 'arena, 'memory, T>
where
    T: CompactValue + CompactDeserialize<'de, 'arena>,
{
    type Value = CompactVec<'arena, T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a sequence of compact values")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = CompactVec::new_in(self.arena);
        while let Some(value) =
            sequence.next_element_seed(CompactDeserializeSeed::<T>::new(&mut *self.arena))?
        {
            values
                .push_in(value, &mut *self.arena)
                .map_err(A::Error::custom)?;
        }
        Ok(values)
    }
}

impl<'de, 'arena, T> CompactDeserialize<'de, 'arena> for CompactVec<'arena, T>
where
    T: CompactValue + CompactDeserialize<'de, 'arena>,
{
    fn deserialize_in<'memory, D>(
        deserializer: D,
        arena: &mut Arena<'arena, 'memory>,
    ) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(VecVisitor {
            arena,
            marker: PhantomData,
        })
    }
}

struct VecDequeVisitor<'seed, 'arena, 'memory, T> {
    arena: &'seed mut Arena<'arena, 'memory>,
    marker: PhantomData<fn() -> T>,
}

impl<'de, 'seed, 'arena, 'memory, T> Visitor<'de> for VecDequeVisitor<'seed, 'arena, 'memory, T>
where
    T: CompactValue + CompactDeserialize<'de, 'arena>,
{
    type Value = CompactVecDeque<'arena, T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a sequence of compact deque values")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = CompactVecDeque::new_in(self.arena);
        while let Some(value) =
            sequence.next_element_seed(CompactDeserializeSeed::<T>::new(&mut *self.arena))?
        {
            values
                .push_back_in(value, &mut *self.arena)
                .map_err(A::Error::custom)?;
        }
        Ok(values)
    }
}

impl<'de, 'arena, T> CompactDeserialize<'de, 'arena> for CompactVecDeque<'arena, T>
where
    T: CompactValue + CompactDeserialize<'de, 'arena>,
{
    fn deserialize_in<'memory, D>(
        deserializer: D,
        arena: &mut Arena<'arena, 'memory>,
    ) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(VecDequeVisitor {
            arena,
            marker: PhantomData,
        })
    }
}

struct HashSetVisitor<'seed, 'arena, 'memory, T> {
    arena: &'seed mut Arena<'arena, 'memory>,
    marker: PhantomData<fn() -> T>,
}

impl<'de, 'seed, 'arena, 'memory, T> Visitor<'de> for HashSetVisitor<'seed, 'arena, 'memory, T>
where
    T: CompactValue + CompactDeserialize<'de, 'arena> + Eq + Hash,
{
    type Value = CompactHashSet<'arena, T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a sequence of compact hash-set values")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = CompactHashSet::new();
        while let Some(value) =
            sequence.next_element_seed(CompactDeserializeSeed::<T>::new(&mut *self.arena))?
        {
            values
                .insert(value, &mut *self.arena)
                .map_err(A::Error::custom)?;
        }
        Ok(values)
    }
}

impl<'de, 'arena, T> CompactDeserialize<'de, 'arena> for CompactHashSet<'arena, T, RandomState>
where
    T: CompactValue + CompactDeserialize<'de, 'arena> + Eq + Hash,
{
    fn deserialize_in<'memory, D>(
        deserializer: D,
        arena: &mut Arena<'arena, 'memory>,
    ) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(HashSetVisitor {
            arena,
            marker: PhantomData,
        })
    }
}

struct HashMapVisitor<'seed, 'arena, 'memory, K, V> {
    arena: &'seed mut Arena<'arena, 'memory>,
    marker: PhantomData<fn() -> (K, V)>,
}

impl<'de, 'seed, 'arena, 'memory, K, V> Visitor<'de>
    for HashMapVisitor<'seed, 'arena, 'memory, K, V>
where
    K: CompactValue + CompactDeserialize<'de, 'arena> + Eq + Hash,
    V: CompactValue + CompactDeserialize<'de, 'arena>,
{
    type Value = CompactHashMap<'arena, K, V>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a map of compact keys and values")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = CompactHashMap::new();
        while let Some(key) =
            map.next_key_seed(CompactDeserializeSeed::<K>::new(&mut *self.arena))?
        {
            let value = map.next_value_seed(CompactDeserializeSeed::<V>::new(&mut *self.arena))?;
            values
                .insert(key, value, &mut *self.arena)
                .map_err(A::Error::custom)?;
        }
        Ok(values)
    }
}

impl<'de, 'arena, K, V> CompactDeserialize<'de, 'arena>
    for CompactHashMap<'arena, K, V, RandomState>
where
    K: CompactValue + CompactDeserialize<'de, 'arena> + Eq + Hash,
    V: CompactValue + CompactDeserialize<'de, 'arena>,
{
    fn deserialize_in<'memory, D>(
        deserializer: D,
        arena: &mut Arena<'arena, 'memory>,
    ) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(HashMapVisitor {
            arena,
            marker: PhantomData,
        })
    }
}

macro_rules! tuple_impl {
    ($visitor:ident, $length:literal; $($ty:ident : $value:ident : $index:literal),+ $(,)?) => {
        struct $visitor<'seed, 'arena, 'memory, $($ty),+> {
            arena: &'seed mut Arena<'arena, 'memory>,
            marker: PhantomData<fn() -> ($($ty,)+)>,
        }

        impl<'de, 'seed, 'arena, 'memory, $($ty),+> Visitor<'de>
            for $visitor<'seed, 'arena, 'memory, $($ty),+>
        where
            $($ty: CompactDeserialize<'de, 'arena>,)+
        {
            type Value = ($($ty,)+);

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "a tuple with {} compact elements", $length)
            }

            fn visit_seq<__Access>(self, mut sequence: __Access) -> Result<Self::Value, __Access::Error>
            where
                __Access: SeqAccess<'de>,
            {
                $(
                    let $value = sequence
                        .next_element_seed(CompactDeserializeSeed::<$ty>::new(&mut *self.arena))?
                        .ok_or_else(|| __Access::Error::invalid_length($index, &self))?;
                )+
                Ok(($($value,)+))
            }
        }

        impl<'de, 'arena, $($ty),+> CompactDeserialize<'de, 'arena> for ($($ty,)+)
        where
            $($ty: CompactDeserialize<'de, 'arena>,)+
        {
            fn deserialize_in<'memory, __Deserializer>(
                deserializer: __Deserializer,
                arena: &mut Arena<'arena, 'memory>,
            ) -> Result<Self, __Deserializer::Error>
            where
                __Deserializer: Deserializer<'de>,
            {
                deserializer.deserialize_tuple($length, $visitor {
                    arena,
                    marker: PhantomData,
                })
            }
        }
    };
}

tuple_impl!(Tuple1Visitor, 1; A: first: 0);
tuple_impl!(Tuple2Visitor, 2; A: first: 0, B: second: 1);
tuple_impl!(Tuple3Visitor, 3; A: first: 0, B: second: 1, C: third: 2);
tuple_impl!(Tuple4Visitor, 4; A: first: 0, B: second: 1, C: third: 2, D: fourth: 3);
