//! Copy compact mutable values into an immutable frozen representation.

use compact_collections::{
    CompactBytes, CompactHashMap, CompactHashSet, CompactOsString, CompactPathBuf, CompactString,
    CompactVec, CompactVecDeque,
};
use compact_core::{Arena, CompactValue};
use core::hash::Hash;

use crate::storage::{
    FrozenArena, FrozenBuilder, FrozenBytes, FrozenError, FrozenMap, FrozenOsString, FrozenPathBuf,
    FrozenResult, FrozenRoot, FrozenSet, FrozenString, FrozenValue, FrozenVec, FrozenVecDeque,
};

/// Convert a value into a freeze-safe representation using `source` for
/// validating arena-owned input and `destination` for immutable output.
pub trait FreezeIn<'source> {
    /// The frozen representation stored in the destination arena.
    type Frozen: FrozenValue;

    /// Copy this value without changing or consuming its mutable source.
    fn freeze_in<'memory>(
        &self,
        source: &Arena<'source, 'memory>,
        destination: &mut FrozenBuilder,
    ) -> FrozenResult<Self::Frozen>;
}

macro_rules! scalar_freeze {
    ($($ty:ty),* $(,)?) => {
        $(
            impl<'source> FreezeIn<'source> for $ty {
                type Frozen = $ty;

                fn freeze_in<'memory>(
                    &self,
                    _source: &Arena<'source, 'memory>,
                    _destination: &mut FrozenBuilder,
                ) -> FrozenResult<Self::Frozen> {
                    Ok(*self)
                }
            }
        )*
    };
}

scalar_freeze!(
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

impl<'source, T> FreezeIn<'source> for Option<T>
where
    T: FreezeIn<'source>,
{
    type Frozen = Option<T::Frozen>;

    fn freeze_in<'memory>(
        &self,
        source: &Arena<'source, 'memory>,
        destination: &mut FrozenBuilder,
    ) -> FrozenResult<Self::Frozen> {
        self.as_ref()
            .map(|value| value.freeze_in(source, destination))
            .transpose()
    }
}

impl<'source, T, E> FreezeIn<'source> for core::result::Result<T, E>
where
    T: FreezeIn<'source>,
    E: FreezeIn<'source>,
{
    type Frozen = core::result::Result<T::Frozen, E::Frozen>;

    fn freeze_in<'memory>(
        &self,
        source: &Arena<'source, 'memory>,
        destination: &mut FrozenBuilder,
    ) -> FrozenResult<Self::Frozen> {
        match self {
            Ok(value) => Ok(Ok(value.freeze_in(source, destination)?)),
            Err(error) => Ok(Err(error.freeze_in(source, destination)?)),
        }
    }
}

impl<'source, T, const N: usize> FreezeIn<'source> for [T; N]
where
    T: FreezeIn<'source>,
{
    type Frozen = [T::Frozen; N];

    fn freeze_in<'memory>(
        &self,
        source: &Arena<'source, 'memory>,
        destination: &mut FrozenBuilder,
    ) -> FrozenResult<Self::Frozen> {
        let mut frozen = Vec::new();
        frozen
            .try_reserve_exact(N)
            .map_err(FrozenError::Allocation)?;
        for value in self {
            frozen.push(value.freeze_in(source, destination)?);
        }
        frozen.try_into().map_err(|_| FrozenError::OffsetOverflow)
    }
}

macro_rules! tuple_freeze {
    ($($ty:ident:$index:tt),+ $(,)?) => {
        impl<'source, $($ty),+> FreezeIn<'source> for ($($ty,)+)
        where
            $($ty: FreezeIn<'source>,)+
        {
            type Frozen = ($($ty::Frozen,)+);

            fn freeze_in<'memory>(
                &self,
                source: &Arena<'source, 'memory>,
                destination: &mut FrozenBuilder,
            ) -> FrozenResult<Self::Frozen> {
                Ok(($(
                    self.$index.freeze_in(source, destination)?,
                )+))
            }
        }
    };
}

tuple_freeze!(A:0);
tuple_freeze!(A:0, B:1);
tuple_freeze!(A:0, B:1, C:2);
tuple_freeze!(A:0, B:1, C:2, D:3);

impl<'source, T: CompactValue + FreezeIn<'source>> FreezeIn<'source> for CompactVec<'source, T> {
    type Frozen = FrozenVec<T::Frozen>;

    fn freeze_in<'memory>(
        &self,
        source: &Arena<'source, 'memory>,
        destination: &mut FrozenBuilder,
    ) -> FrozenResult<Self::Frozen> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(self.len())
            .map_err(FrozenError::Allocation)?;
        for value in self.iter(source)? {
            values.push(value.freeze_in(source, destination)?);
        }
        destination.store_slice(&values)
    }
}

impl<'source, T: CompactValue + FreezeIn<'source>> FreezeIn<'source>
    for CompactVecDeque<'source, T>
{
    type Frozen = FrozenVecDeque<T::Frozen>;

    fn freeze_in<'memory>(
        &self,
        source: &Arena<'source, 'memory>,
        destination: &mut FrozenBuilder,
    ) -> FrozenResult<Self::Frozen> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(self.len())
            .map_err(FrozenError::Allocation)?;
        for value in self.iter(source)? {
            values.push(value.freeze_in(source, destination)?);
        }
        Ok(FrozenVecDeque {
            values: destination.store_slice(&values)?,
        })
    }
}

impl<'source> FreezeIn<'source> for CompactString<'source> {
    type Frozen = FrozenString;

    fn freeze_in<'memory>(
        &self,
        source: &Arena<'source, 'memory>,
        destination: &mut FrozenBuilder,
    ) -> FrozenResult<Self::Frozen> {
        destination.store_str(self.as_str(source)?)
    }
}

impl<'source> FreezeIn<'source> for CompactBytes<'source> {
    type Frozen = FrozenBytes;

    fn freeze_in<'memory>(
        &self,
        _source: &Arena<'source, 'memory>,
        destination: &mut FrozenBuilder,
    ) -> FrozenResult<Self::Frozen> {
        destination.store_bytes(self.as_slice())
    }
}

impl<'source, K, V, S> FreezeIn<'source> for CompactHashMap<'source, K, V, S>
where
    K: CompactValue + FreezeIn<'source> + Eq + Hash,
    V: CompactValue + FreezeIn<'source>,
    S: std::hash::BuildHasher,
{
    type Frozen = FrozenMap<K::Frozen, V::Frozen>;

    fn freeze_in<'memory>(
        &self,
        source: &Arena<'source, 'memory>,
        destination: &mut FrozenBuilder,
    ) -> FrozenResult<Self::Frozen> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(self.len())
            .map_err(FrozenError::Allocation)?;
        for (key, value) in self.iter(source)? {
            entries.push((
                key.freeze_in(source, destination)?,
                value.freeze_in(source, destination)?,
            ));
        }
        Ok(FrozenMap {
            entries: destination.store_slice(&entries)?,
        })
    }
}

impl<'source, T, S> FreezeIn<'source> for CompactHashSet<'source, T, S>
where
    T: CompactValue + FreezeIn<'source> + Eq + Hash,
    S: std::hash::BuildHasher,
{
    type Frozen = FrozenSet<T::Frozen>;

    fn freeze_in<'memory>(
        &self,
        source: &Arena<'source, 'memory>,
        destination: &mut FrozenBuilder,
    ) -> FrozenResult<Self::Frozen> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(self.len())
            .map_err(FrozenError::Allocation)?;
        for value in self.iter(source)? {
            values.push(value.freeze_in(source, destination)?);
        }
        Ok(FrozenSet {
            values: destination.store_slice(&values)?,
        })
    }
}

impl<'source> FreezeIn<'source> for CompactOsString<'source> {
    type Frozen = FrozenOsString;

    fn freeze_in<'memory>(
        &self,
        _source: &Arena<'source, 'memory>,
        destination: &mut FrozenBuilder,
    ) -> FrozenResult<Self::Frozen> {
        freeze_os_str(self.as_os_str(), destination)
    }
}

impl<'source> FreezeIn<'source> for CompactPathBuf<'source> {
    type Frozen = FrozenPathBuf;

    fn freeze_in<'memory>(
        &self,
        _source: &Arena<'source, 'memory>,
        destination: &mut FrozenBuilder,
    ) -> FrozenResult<Self::Frozen> {
        Ok(FrozenPathBuf {
            inner: freeze_os_str(self.as_os_str(), destination)?,
        })
    }
}

fn freeze_os_str(
    value: compact_collections::CompactOsStr<'_>,
    destination: &mut FrozenBuilder,
) -> FrozenResult<FrozenOsString> {
    Ok(FrozenOsString {
        bytes: destination.store_bytes(value.as_encoded_bytes())?,
    })
}

/// Freeze one value graph into a new immutable arena and return its root.
///
/// The source arena and values are only borrowed. If copying fails, they remain
/// valid and the unfinished frozen builder is discarded.
pub fn freeze_in<'source, 'memory, T>(
    value: &T,
    source: &Arena<'source, 'memory>,
) -> FrozenResult<(FrozenArena, FrozenRoot<T::Frozen>)>
where
    T: FreezeIn<'source>,
{
    let mut builder = FrozenBuilder::new()?;
    let root = value.freeze_in(source, &mut builder)?;
    builder.finish_root(root)
}
