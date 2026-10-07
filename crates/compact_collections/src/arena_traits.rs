//! Explicit arena-aware counterparts to allocation-requiring std traits.

use compact_core::{Arena, CompactValue};
use core::fmt;

use crate::{
    CompactBox, CompactBytes, CompactHashMap, CompactHashSet, CompactOsString, CompactPathBuf,
    CompactRing, CompactSmallVec, CompactString, CompactVec, CompactVecDeque, Result,
};

/// Construct an arena-owned collection from an iterator.
pub trait FromIteratorIn<'arena, Item>: Sized {
    /// Collect values into `arena`, reporting allocation failure explicitly.
    fn from_iter_in<I>(iter: I, arena: &mut Arena<'arena, '_>) -> Result<Self>
    where
        I: IntoIterator<Item = Item>;
}

/// Extend an arena-owned collection using an explicit allocation context.
pub trait ExtendIn<'arena, Item> {
    /// Append values, reporting allocation failure explicitly.
    fn extend_in<I>(&mut self, iter: I, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        I: IntoIterator<Item = Item>;
}

/// Clone an arena owner into `arena` without hidden allocation context.
pub trait CloneIn<'arena> {
    /// The owner type produced in the destination arena.
    type Cloned;

    /// Clone into `arena`, reporting allocation failure explicitly.
    fn clone_in(&self, arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned>;
}

impl<'arena, T: CompactValue + Copy> CloneIn<'arena> for T {
    type Cloned = T;

    fn clone_in(&self, _arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned> {
        Ok(*self)
    }
}

/// Convert a displayable value to an arena-backed UTF-8 string.
pub trait ToCompactStringIn<'arena> {
    /// Format into a compact string, reporting arena allocation failure.
    fn to_compact_string_in(&self, arena: &mut Arena<'arena, '_>) -> Result<CompactString<'arena>>;
}

impl<'arena, T: CompactValue> FromIteratorIn<'arena, T> for CompactVec<'arena, T> {
    fn from_iter_in<I>(iter: I, arena: &mut Arena<'arena, '_>) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
    {
        let iter = iter.into_iter();
        let mut values = CompactVec::with_capacity_in(iter.size_hint().0, arena)?;
        values.extend_in(iter, arena)?;
        Ok(values)
    }
}

impl<'arena, T: CompactValue> ExtendIn<'arena, T> for CompactVec<'arena, T> {
    fn extend_in<I>(&mut self, iter: I, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        I: IntoIterator<Item = T>,
    {
        for value in iter {
            self.push_in(value, arena)?;
        }
        Ok(())
    }
}

impl<'arena, T> CloneIn<'arena> for CompactVec<'arena, T>
where
    T: CompactValue + CloneIn<'arena>,
    T::Cloned: CompactValue,
{
    type Cloned = CompactVec<'arena, T::Cloned>;

    fn clone_in(&self, arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned> {
        let mut values = CompactVec::with_capacity_in(self.len(), arena)?;
        for value in self.as_ref() {
            values.push_in(value.clone_in(arena)?, arena)?;
        }
        Ok(values)
    }
}

impl<'arena> FromIteratorIn<'arena, u8> for CompactBytes<'arena> {
    fn from_iter_in<I>(iter: I, arena: &mut Arena<'arena, '_>) -> Result<Self>
    where
        I: IntoIterator<Item = u8>,
    {
        let iter = iter.into_iter();
        let mut bytes = CompactBytes::with_capacity_in(iter.size_hint().0, arena)?;
        bytes.extend_in(iter, arena)?;
        Ok(bytes)
    }
}

impl<'arena> ExtendIn<'arena, u8> for CompactBytes<'arena> {
    fn extend_in<I>(&mut self, iter: I, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        I: IntoIterator<Item = u8>,
    {
        for byte in iter {
            self.push_in(byte, arena)?;
        }
        Ok(())
    }
}

impl<'arena> CloneIn<'arena> for CompactBytes<'arena> {
    type Cloned = CompactBytes<'arena>;

    fn clone_in(&self, arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned> {
        CompactBytes::from_slice_in(self.as_slice(), arena)
    }
}

impl<'arena> FromIteratorIn<'arena, char> for CompactString<'arena> {
    fn from_iter_in<I>(iter: I, arena: &mut Arena<'arena, '_>) -> Result<Self>
    where
        I: IntoIterator<Item = char>,
    {
        let mut text = CompactString::empty();
        text.extend_in(iter, arena)?;
        Ok(text)
    }
}

impl<'arena> ExtendIn<'arena, char> for CompactString<'arena> {
    fn extend_in<I>(&mut self, iter: I, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        I: IntoIterator<Item = char>,
    {
        for value in iter {
            self.push_char_in(value, arena)?;
        }
        Ok(())
    }
}

impl<'arena> CloneIn<'arena> for CompactString<'arena> {
    type Cloned = CompactString<'arena>;

    fn clone_in(&self, arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned> {
        CompactString::from_str_in(self, arena)
    }
}

impl<'arena, T: CompactValue> FromIteratorIn<'arena, T> for CompactVecDeque<'arena, T> {
    fn from_iter_in<I>(iter: I, arena: &mut Arena<'arena, '_>) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
    {
        let iter = iter.into_iter();
        let mut values = CompactVecDeque::with_capacity_in(iter.size_hint().0, arena)?;
        values.extend_in(iter, arena)?;
        Ok(values)
    }
}

impl<'arena, T: CompactValue> ExtendIn<'arena, T> for CompactVecDeque<'arena, T> {
    fn extend_in<I>(&mut self, iter: I, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        I: IntoIterator<Item = T>,
    {
        for value in iter {
            self.push_back_in(value, arena)?;
        }
        Ok(())
    }
}

impl<'arena, T> CloneIn<'arena> for CompactVecDeque<'arena, T>
where
    T: CompactValue + CloneIn<'arena>,
    T::Cloned: CompactValue,
{
    type Cloned = CompactVecDeque<'arena, T::Cloned>;

    fn clone_in(&self, arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned> {
        let mut values = CompactVecDeque::with_capacity_in(self.len(), arena)?;
        for value in self.iter(arena)? {
            values.push_back_in(value.clone_in(arena)?, arena)?;
        }
        Ok(values)
    }
}

impl<'arena, T: CompactValue, const N: usize> FromIteratorIn<'arena, T>
    for CompactSmallVec<'arena, T, N>
{
    fn from_iter_in<I>(iter: I, arena: &mut Arena<'arena, '_>) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
    {
        let mut values = CompactSmallVec::new_in(arena);
        for value in iter {
            values.push_in(value, arena)?;
        }
        Ok(values)
    }
}

impl<'arena, T: CompactValue, const N: usize> ExtendIn<'arena, T>
    for CompactSmallVec<'arena, T, N>
{
    fn extend_in<I>(&mut self, iter: I, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        I: IntoIterator<Item = T>,
    {
        for value in iter {
            self.push_in(value, arena)?;
        }
        Ok(())
    }
}

impl<'arena, T, const N: usize> CloneIn<'arena> for CompactSmallVec<'arena, T, N>
where
    T: CompactValue + CloneIn<'arena>,
    T::Cloned: CompactValue,
{
    type Cloned = CompactSmallVec<'arena, T::Cloned, N>;

    fn clone_in(&self, arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned> {
        let mut values = CompactSmallVec::new_in(arena);
        for value in self.as_slice(arena)? {
            values.push_in(value.clone_in(arena)?, arena)?;
        }
        Ok(values)
    }
}

impl<'arena, K, V, S> FromIteratorIn<'arena, (K, V)> for CompactHashMap<'arena, K, V, S>
where
    K: CompactValue + Eq + core::hash::Hash,
    V: CompactValue,
    S: std::hash::BuildHasher + Default,
{
    fn from_iter_in<I>(iter: I, arena: &mut Arena<'arena, '_>) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
    {
        let iter = iter.into_iter();
        let mut map = CompactHashMap::with_hasher(S::default());
        map.reserve(iter.size_hint().0, arena)?;
        map.extend_in(iter, arena)?;
        Ok(map)
    }
}

impl<'arena, K, V, S> ExtendIn<'arena, (K, V)> for CompactHashMap<'arena, K, V, S>
where
    K: CompactValue + Eq + core::hash::Hash,
    V: CompactValue,
    S: std::hash::BuildHasher,
{
    fn extend_in<I>(&mut self, iter: I, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        I: IntoIterator<Item = (K, V)>,
    {
        for (key, value) in iter {
            self.insert(key, value, arena)?;
        }
        Ok(())
    }
}

impl<'arena, K, V, S> CloneIn<'arena> for CompactHashMap<'arena, K, V, S>
where
    K: CompactValue + CloneIn<'arena> + Eq + core::hash::Hash,
    K::Cloned: CompactValue + Eq + core::hash::Hash,
    V: CompactValue + CloneIn<'arena>,
    V::Cloned: CompactValue,
    S: std::hash::BuildHasher + Clone,
{
    type Cloned = CompactHashMap<'arena, K::Cloned, V::Cloned, S>;

    fn clone_in(&self, arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned> {
        let mut map = CompactHashMap::with_hasher(self.hasher().clone());
        for (key, value) in self.iter(arena)? {
            map.insert(key.clone_in(arena)?, value.clone_in(arena)?, arena)?;
        }
        Ok(map)
    }
}

impl<'arena, T, S> FromIteratorIn<'arena, T> for CompactHashSet<'arena, T, S>
where
    T: CompactValue + Eq + core::hash::Hash,
    S: std::hash::BuildHasher + Default,
{
    fn from_iter_in<I>(iter: I, arena: &mut Arena<'arena, '_>) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
    {
        let iter = iter.into_iter();
        let mut set = CompactHashSet::with_hasher(S::default());
        set.reserve(iter.size_hint().0, arena)?;
        set.extend_in(iter, arena)?;
        Ok(set)
    }
}

impl<'arena, T, S> ExtendIn<'arena, T> for CompactHashSet<'arena, T, S>
where
    T: CompactValue + Eq + core::hash::Hash,
    S: std::hash::BuildHasher,
{
    fn extend_in<I>(&mut self, iter: I, arena: &mut Arena<'arena, '_>) -> Result<()>
    where
        I: IntoIterator<Item = T>,
    {
        for value in iter {
            self.insert(value, arena)?;
        }
        Ok(())
    }
}

impl<'arena, T, S> CloneIn<'arena> for CompactHashSet<'arena, T, S>
where
    T: CompactValue + CloneIn<'arena> + Eq + core::hash::Hash,
    T::Cloned: CompactValue + Eq + core::hash::Hash,
    S: std::hash::BuildHasher + Clone,
{
    type Cloned = CompactHashSet<'arena, T::Cloned, S>;

    fn clone_in(&self, arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned> {
        let mut set = CompactHashSet::with_hasher(self.hasher().clone());
        for value in self.iter(arena)? {
            set.insert(value.clone_in(arena)?, arena)?;
        }
        Ok(set)
    }
}

impl<'arena, T> CloneIn<'arena> for CompactBox<'arena, T>
where
    T: CompactValue + CloneIn<'arena>,
    T::Cloned: CompactValue,
{
    type Cloned = CompactBox<'arena, T::Cloned>;

    fn clone_in(&self, arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned> {
        CompactBox::new_in((**self).clone_in(arena)?, arena)
    }
}

impl<'arena, T> CloneIn<'arena> for CompactRing<'arena, T>
where
    T: CompactValue + CloneIn<'arena>,
    T::Cloned: CompactValue,
{
    type Cloned = CompactRing<'arena, T::Cloned>;

    fn clone_in(&self, arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned> {
        let mut ring = CompactRing::with_capacity(self.capacity(), arena)?;
        for value in self.iter(arena)? {
            ring.push_back(value.clone_in(arena)?, arena)?;
        }
        Ok(ring)
    }
}

impl<'arena> CloneIn<'arena> for CompactOsString<'arena> {
    type Cloned = CompactOsString<'arena>;

    fn clone_in(&self, arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned> {
        CompactOsString::from_os_string(self.to_os_string(), arena)
    }
}

impl<'arena> CloneIn<'arena> for CompactPathBuf<'arena> {
    type Cloned = CompactPathBuf<'arena>;

    fn clone_in(&self, arena: &mut Arena<'arena, '_>) -> Result<Self::Cloned> {
        CompactPathBuf::from_path(&self.to_path_buf(), arena)
    }
}

impl<'arena, T: fmt::Display + ?Sized> ToCompactStringIn<'arena> for T {
    fn to_compact_string_in(&self, arena: &mut Arena<'arena, '_>) -> Result<CompactString<'arena>> {
        let mut text = CompactString::empty();
        text.writer(arena).write_fmt_in(format_args!("{self}"))?;
        Ok(text)
    }
}
